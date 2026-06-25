//! Phase 5 / US3.c-1 — end-to-end pipeline test exercising the full
//! Phase 5 surface: enable plugin → index → search → get → prompts.
//!
//! Verifies that a plugin shipping both `skills/` and `commands/` with
//! substitution-bearing bodies surfaces correctly via the MCP read
//! surfaces (`get_skill`) and the user-facing prompts surface
//! (`prompts/list` + `prompts/get`), and that retrieval renders the
//! body through the now-operational 4-stage substitution pipeline
//! (built-ins → env passthrough → arguments → `$ARGUMENTS` fallback).
//!
//! Scope deliberately narrower than `tests/substitution_pipeline.rs`:
//! this file is the cross-surface integration proof — confirming that
//! the wiring between `lifecycle::enable`, `index::skills::find`,
//! `mcp::tools::get_skill::handle`, `PromptRegistry::descriptors` /
//! `lookup`, and `mcp::prompts::handle_get` agree on the same on-disk
//! fixture.
//!
//! Library-API tests using `StubEmbedder` + `StubReranker`; no ONNX
//! model load; the CLI binary is not invoked.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde_json::{Map, Value, json};
use tempfile::TempDir;
use tokio::sync::OnceCell;
use tome::config::Config;
use tome::embedding::Reranker;
use tome::embedding::registry::{ModelEntry, ModelKind, lookup};
use tome::embedding::stub::{StubEmbedder, StubReranker};
use tome::index::{self, OpenOptions};
use tome::mcp::prompts::{self, PromptRegistry};
use tome::mcp::state::McpState;
use tome::mcp::tools::{get_skill, search_skills};
use tome::plugin::PluginId;
use tome::plugin::identity::EntryKind;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, WorkspaceName};

use crate::common::{
    config_with_catalog, fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

// ---------------------------------------------------------------------------
// Fixture staging
//
// Mirrors `tests/substitution_pipeline.rs::stage_workspace` (which is in
// turn modelled on `tests/mcp_prompts.rs::stage_workspace_with`). We
// keep a file-local copy because:
//
// - `mcp_prompts.rs::stage_workspace_with` does NOT persist `config.toml`
//   to disk, so `mcp::tools::get_skill::handle` (which calls
//   `store::load(&paths.global_config_file)`) returns `unknown_catalog`.
// - `substitution_pipeline.rs::stage_workspace` does the right thing
//   but the helper is `fn` not `pub`, and it carries enough fixture
//   knobs (env guards, plugin/workspace data-dir guards) that a global
//   promotion would over-fit.
//
// Promotion to `tests/common/mod.rs` deferred — when a fifth consumer
// lands, fold the variants into a builder-style helper.
// ---------------------------------------------------------------------------

fn write_plugin(
    catalog_root: &Path,
    plugin_name: &str,
    skills: &[(&str, &str)],
    commands: &[(&str, &str)],
) {
    let plugin_dir = catalog_root.join(plugin_name);
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    std::fs::write(
        plugin_dir.join("tome-plugin.toml"),
        format!(
            "name = \"{}\"\nversion = \"1.0.0\"\n",
            plugin_dir.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
    fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        format!(r#"{{"name": "{plugin_name}", "version": "1.0.0"}}"#),
    )
    .unwrap();
    for (name, body) in skills {
        let dir = plugin_dir.join("skills").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
    }
    if !commands.is_empty() {
        let cmd_dir = plugin_dir.join("commands");
        fs::create_dir_all(&cmd_dir).unwrap();
        for (name, body) in commands {
            fs::write(cmd_dir.join(format!("{name}.md")), body).unwrap();
        }
    }
}

fn save_config(paths: &tome::paths::Paths, config: &Config) {
    if let Some(parent) = paths.global_config_file.parent() {
        fs::create_dir_all(parent).expect("create config parent");
    }
    let text = toml::to_string(config).expect("serialize config");
    tome::catalog::store::write_atomic(&paths.global_config_file, text.as_bytes())
        .expect("save config");
}

fn open_index(paths: &tome::paths::Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open index db")
}

fn seed_catalog_enrolment(paths: &tome::paths::Paths, catalog_root: &Path, catalog_name: &str) {
    let url = format!("file://{}", catalog_root.display());
    let conn = open_index(paths);
    tome::index::workspace_catalogs::insert(&conn, "global", catalog_name, &url, "main")
        .expect("seed workspace_catalogs");
    drop(conn);

    let cache_dir = paths.cache_dir_for(&url);
    if let Some(parent) = cache_dir.parent() {
        fs::create_dir_all(parent).expect("create catalogs parent");
    }
    if !cache_dir.exists() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(catalog_root, &cache_dir).expect("symlink catalog cache");
        #[cfg(not(unix))]
        {
            fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
                fs::create_dir_all(dst)?;
                for entry in fs::read_dir(src)? {
                    let entry = entry?;
                    let to = dst.join(entry.file_name());
                    if entry.file_type()?.is_dir() {
                        copy_dir(&entry.path(), &to)?;
                    } else {
                        fs::copy(entry.path(), &to)?;
                    }
                }
                Ok(())
            }
            copy_dir(catalog_root, &cache_dir).expect("copy catalog cache");
        }
    }
}

/// Stage a workspace + plugin + persist `config.toml` to disk so both
/// `lifecycle::enable` (in-memory config) AND `get_skill::handle`
/// (disk-loaded config) agree on the catalog enrolment.
fn stage_workspace(
    tmp: &TempDir,
    paths: &tome::paths::Paths,
    skills: &[(&str, &str)],
    commands: &[(&str, &str)],
) -> PathBuf {
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(paths);

    let catalog_root = tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);
    write_plugin(&catalog_root, "plug", skills, commands);

    save_config(paths, &config);

    let embedder = StubEmbedder::new();
    let scope = tome::workspace::Scope(WorkspaceName::global());
    let deps = LifecycleDeps {
        paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "acme/plug".parse().unwrap();
    // FF1: enrolment + cache symlink before enable — resolve_plugin_dir now
    // reads workspace_catalogs, not the in-memory Config.
    seed_catalog_enrolment(paths, &catalog_root, "acme");
    lifecycle::enable(&id, &deps).expect("enable plugin");
    catalog_root
}

// ---------------------------------------------------------------------------
// State builders
// ---------------------------------------------------------------------------

fn build_state(paths: &tome::paths::Paths, registry: PromptRegistry) -> Arc<McpState> {
    let embedder_entry = lookup("bge-small-en-v1.5").expect("registry has embedder");
    let reranker_entry = lookup("bge-reranker-base").expect("registry has reranker");
    let reranker: Arc<dyn Reranker> = Arc::new(StubReranker::new());

    Arc::new(McpState {
        embedder: Arc::new(StubEmbedder::new()),
        reranker: OnceCell::new_with(Some(reranker)),
        scope: ResolvedScope::global_fallback(),
        paths: paths.clone(),
        embedder_entry,
        embedder_seed: tome::index::MetaSeed {
            name: embedder_entry.name.into(),
            version: embedder_entry.version.into(),
        },
        reranker_entry,
        prompt_registry: Arc::new(std::sync::RwLock::new(Arc::new(registry))),
        host_harness: None,
        last_search_ranks: std::sync::Mutex::new(std::collections::HashMap::new()),
        flush_signal: std::sync::Arc::new(tokio::sync::Notify::new()),
        enqueued_since_flush: std::sync::atomic::AtomicUsize::new(0),
    })
}

fn build_registry(paths: &tome::paths::Paths) -> PromptRegistry {
    let conn = open_index(paths);
    PromptRegistry::build_for_workspace(&WorkspaceName::global(), paths, &conn, false)
        .expect("build prompt registry")
}

/// Stub `ModelEntry`s — required when invoking `search_skills::handle`
/// (the handler reads `state.embedder_entry.name/version` for drift
/// detection against the index `meta` row, which is seeded with
/// `stub_embedder_seed()`). Using `lookup("bge-small-en-v1.5")` instead
/// would mismatch the stub-seeded index and trip `embedder_drift`.
/// Mirrors `tests/mcp_search_skills_truncation.rs::{STUB_EMBEDDER_ENTRY,
/// STUB_RERANKER_ENTRY}` — file-local copies stay in sync with the stub
/// embedder/reranker's reported identity.
static STUB_EMBEDDER_ENTRY: ModelEntry = ModelEntry {
    name: "stub-embedder",
    version: "0",
    kind: ModelKind::Embedder,
    source_url: "stub://embedder",
    sha256: "0000000000000000000000000000000000000000000000000000000000000000",
    size_bytes: 0,
    licence: "MIT",
    embedding_dim: Some(384),
    files: &[],
    aux_urls: &[],
};

static STUB_RERANKER_ENTRY: ModelEntry = ModelEntry {
    name: "stub-reranker",
    version: "0",
    kind: ModelKind::Reranker,
    source_url: "stub://reranker",
    sha256: "0000000000000000000000000000000000000000000000000000000000000000",
    size_bytes: 0,
    licence: "MIT",
    embedding_dim: None,
    files: &[],
    aux_urls: &[],
};

/// State builder for tests that invoke `search_skills::handle`. Uses
/// the `STUB_EMBEDDER_ENTRY`/`STUB_RERANKER_ENTRY` so the search
/// pipeline's drift detection agrees with the index seeded by
/// `lifecycle::enable` (which records `stub_embedder_seed()` /
/// `stub_reranker_seed()` in `meta`).
fn build_state_with_stub_entries(
    paths: &tome::paths::Paths,
    registry: PromptRegistry,
) -> Arc<McpState> {
    let reranker: Arc<dyn Reranker> = Arc::new(StubReranker::new());
    Arc::new(McpState {
        embedder: Arc::new(StubEmbedder::new()),
        reranker: OnceCell::new_with(Some(reranker)),
        scope: ResolvedScope::global_fallback(),
        paths: paths.clone(),
        embedder_entry: &STUB_EMBEDDER_ENTRY,
        embedder_seed: tome::index::MetaSeed {
            name: STUB_EMBEDDER_ENTRY.name.into(),
            version: STUB_EMBEDDER_ENTRY.version.into(),
        },
        reranker_entry: &STUB_RERANKER_ENTRY,
        prompt_registry: Arc::new(std::sync::RwLock::new(Arc::new(registry))),
        host_harness: None,
        last_search_ranks: std::sync::Mutex::new(std::collections::HashMap::new()),
        flush_signal: std::sync::Arc::new(tokio::sync::Notify::new()),
        enqueued_since_flush: std::sync::atomic::AtomicUsize::new(0),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Full read-side pipeline: enable a plugin with a skill whose body
/// references `${TOME_SKILL_NAME}` (a Stage-1 built-in), then verify
/// that:
///
/// 1. The skill is reachable via `index::skills::find` (search-side
///    library entry, what `commands::query::pipeline` walks).
/// 2. The skill is reachable via `mcp::tools::get_skill::handle` (the
///    MCP read tool).
/// 3. The body returned by `get_skill::handle` has the Stage-1 built-in
///    substituted — i.e. the substitution pipeline runs end-to-end on
///    the read path even though `get_skill` never carries caller args
///    (Stages 3 + 4 are no-ops here per the contract).
#[test]
fn enable_search_get_skill_with_substitution() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());

    let skill_body = "---\nname: pipe-skill\ndescription: pipe.\n---\nname=${TOME_SKILL_NAME} cat=${TOME_CATALOG_NAME}\n";
    stage_workspace(&tmp, &paths, &[("pipe-skill", skill_body)], &[]);

    // (1) Search-side: confirm the skill is indexed and findable. The
    // production `commands::query::pipeline` runs an ANN query then a
    // rerank; for this integration check we go straight to the index
    // helper to keep the assertion focused on the catalog/plugin/name
    // round-trip rather than the score ranking (covered in `query.rs`).
    let conn = open_index(&paths);
    let row = tome::index::skills::find(
        &conn,
        "global",
        "acme",
        "plug",
        EntryKind::Skill,
        "pipe-skill",
    )
    .expect("find query")
    .expect("skill row present");
    assert!(row.enabled, "indexed skill must be enabled post-enable");
    assert_eq!(row.name, "pipe-skill");
    drop(conn);

    // (2) Read-side via the MCP tool. The handler builds a
    // `SubstitutionContext` with `args = None` (Stage 3 + 4 no-op) and
    // renders Stages 1 + 2 over the body.
    let state = build_state(&paths, PromptRegistry::default());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let output = rt
        .block_on(get_skill::handle(
            state,
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "pipe-skill".into(),
            },
        ))
        .expect("get_skill ok");

    // (3) Body rendered through the substitution pipeline.
    assert!(
        output.content.contains("name=pipe-skill"),
        "TOME_SKILL_NAME (Stage 1) substituted; got: {:?}",
        output.content,
    );
    assert!(
        output.content.contains("cat=acme"),
        "TOME_CATALOG_NAME (Stage 1) substituted; got: {:?}",
        output.content,
    );
    assert!(
        !output.content.contains("${TOME_SKILL_NAME}"),
        "no Stage-1 references must survive in the rendered body; got: {:?}",
        output.content,
    );
}

/// User-invocable command rendered through `prompts/get` with caller
/// arguments. Exercises Stage 3 (named-argument substitution) end-to-end
/// from caller `arguments` → `map_caller_arguments` →
/// `ArgumentValues::Object` → `substitution::render` Stage 3.
#[test]
fn enable_command_invocable_via_prompts_get() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());

    // Command body references the declared `name` via the `$name`
    // syntax. Per Claude Code's contract (FR-051 / substitution-engine
    // §Stage 3), named args are addressable by `$<name>`.
    let cmd_body =
        "---\nname: greet\ndescription: Greet someone.\narguments: [name]\n---\nHello, $name!\n";
    stage_workspace(&tmp, &paths, &[], &[("greet", cmd_body)]);

    let registry = build_registry(&paths);
    let state = build_state(&paths, registry);

    let mut args = Map::new();
    args.insert("name".into(), json!("Alice"));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let response = rt
        .block_on(prompts::handle_get(state, "plug__greet".into(), Some(args)))
        .expect("prompts/get ok");

    assert_eq!(response.messages.len(), 1);
    let text = match &response.messages[0].content {
        rmcp::model::PromptMessageContent::Text { text } => text.clone(),
        other => panic!("expected text content, got {other:?}"),
    };
    assert!(
        text.contains("Hello, Alice!"),
        "Stage 3 named-arg substitution: expected `Hello, Alice!`; got: {text:?}",
    );
    assert!(
        !text.contains("$name"),
        "no unresolved $name reference must survive; got: {text:?}",
    );
}

/// `prompts/list` surfaces user-invocable entries (commands by default;
/// skills only on opt-in) but hides non-invocable ones. The plugin in
/// this fixture ships BOTH a skill (default `user_invocable = false`)
/// and a command (default `user_invocable = true`); the registry must
/// expose only the command.
#[test]
fn prompts_list_shows_only_user_invocable_entries() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());

    let skill_body = "---\nname: silent-skill\ndescription: not in prompts.\n---\nbody\n";
    let cmd_body = "---\nname: visible\ndescription: in prompts.\n---\nbody\n";
    stage_workspace(
        &tmp,
        &paths,
        &[("silent-skill", skill_body)],
        &[("visible", cmd_body)],
    );

    let registry = build_registry(&paths);
    // Phase 9 / US3: drop the always-on reserved `add-tome-conversion-skill`
    // built-in so this asserts only the PLUGIN-derived prompt surface.
    let names: Vec<String> = registry
        .descriptors()
        .iter()
        .map(|d| d.name.clone())
        .filter(|n| n != "add-tome-conversion-skill")
        .collect();
    assert_eq!(
        names,
        vec!["plug__visible".to_string()],
        "prompts/list must surface only the user-invocable command; got: {names:?}",
    );

    // Confirm the skill IS reachable via the read-side (it's indexed
    // and enabled — just not user-invocable) so the prompts/list filter
    // doesn't leak into the get_skill surface.
    let conn = open_index(&paths);
    let row = tome::index::skills::find(
        &conn,
        "global",
        "acme",
        "plug",
        EntryKind::Skill,
        "silent-skill",
    )
    .expect("find query")
    .expect("indexed skill remains reachable via the read surface");
    assert!(row.enabled);
}

/// `prompts/get` with an unknown named-arg key must surface the
/// `prompt_argument_mismatch` MCP error envelope per
/// `contracts/exit-codes-p5.md` (exit code 26). This is the integration
/// proof that `map_caller_arguments` → `TomeError::PromptArgumentMismatch`
/// → `emit_tome_error_for_get` propagates the contract-pinned `code` slug.
#[test]
fn prompts_get_with_unknown_named_arg_surfaces_mismatch() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());

    let cmd_body = "---\nname: pick\ndescription: pick one.\narguments: [foo]\n---\nGot $foo\n";
    stage_workspace(&tmp, &paths, &[], &[("pick", cmd_body)]);

    let registry = build_registry(&paths);
    let state = build_state(&paths, registry);

    // Supply an unknown key `bar` (the entry declared only `foo`).
    let mut args = Map::new();
    args.insert("bar".into(), Value::from("oops"));

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let err = rt
        .block_on(prompts::handle_get(state, "plug__pick".into(), Some(args)))
        .expect_err("unknown named arg must surface PromptArgumentMismatch");

    let data = err.data.expect("structured error data");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("prompt_argument_mismatch"),
        "unknown named key → prompt_argument_mismatch envelope; got {data}",
    );
}

/// Phase 5 / US5.a — per-entry invocability matrix end-to-end.
///
/// Stages a single plugin with four entries spanning the full 2x2
/// matrix of `searchable` x `user_invocable` resolved values, then
/// proves the two read-surfaces (`search_skills` + `prompts/list`)
/// filter independently per the resolved frontmatter flags:
///
/// | Entry                | kind    | frontmatter                   | resolved searchable | resolved user_invocable | search_skills | prompts/list |
/// |----------------------|---------|-------------------------------|---------------------|-------------------------|---------------|--------------|
/// | `default-skill`      | skill   | (none)                        | true                | false                   | yes           | no           |
/// | `default-command`    | command | (none)                        | true                | true                    | yes           | yes          |
/// | `model-disabled`     | skill   | `disable-model-invocation: true` | false            | false                   | no            | no           |
/// | `user-invocable-skill` | skill | `user-invocable: true`        | true                | true                    | yes           | yes          |
///
/// Verifies that:
/// - All four entries are present in the DB (`index::skills::find`
///   bypasses both filters) with the correct resolved flag values.
/// - `search_skills::handle` returns exactly three (the dormant
///   `model-disabled` is filtered by `WHERE searchable = 1`).
/// - `PromptRegistry::descriptors()` returns exactly two (the two
///   `default-skill` + `model-disabled` entries are filtered by
///   `WHERE user_invocable = 1`).
///
/// This is the integration proof that the two filter clauses
/// (T353/T354) are independent and compose correctly across the
/// full matrix of resolved-default + explicit-opt-in/out combinations.
#[test]
fn matrix_plugin_filters_searches_and_prompts_per_flag_combination() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());

    // Bodies use distinct, query-tag-bearing descriptions so the
    // KNN search returns deterministic candidates against the stub
    // embedder. The body content is irrelevant — only the
    // frontmatter `name`/`description` and the flag fields drive
    // the filter assertions.
    let default_skill =
        "---\nname: default-skill\ndescription: matrix default skill no flags.\n---\nbody\n";
    let model_disabled = "---\nname: model-disabled\ndescription: matrix dormant entry.\ndisable-model-invocation: true\n---\nbody\n";
    let user_invocable_skill = "---\nname: user-invocable-skill\ndescription: matrix opt-in invocable skill.\nuser-invocable: true\n---\nbody\n";
    let default_command =
        "---\nname: default-command\ndescription: matrix default command no flags.\n---\nbody\n";

    stage_workspace(
        &tmp,
        &paths,
        &[
            ("default-skill", default_skill),
            ("model-disabled", model_disabled),
            ("user-invocable-skill", user_invocable_skill),
        ],
        &[("default-command", default_command)],
    );

    // ------------------------------------------------------------------
    // (1) DB layer: all four entries are present with the correct
    // resolved flags. `index::skills::find` does NOT filter on
    // searchable or user_invocable — it is the integrity baseline.
    // ------------------------------------------------------------------
    let conn = open_index(&paths);
    let find = |kind: EntryKind, name: &str| {
        tome::index::skills::find(&conn, "global", "acme", "plug", kind, name)
            .expect("find query")
            .unwrap_or_else(|| panic!("entry `{name}` missing from index"))
    };

    let default_skill_row = find(EntryKind::Skill, "default-skill");
    assert!(default_skill_row.enabled);
    assert!(
        default_skill_row.searchable,
        "skill with no flags → resolved searchable = true",
    );
    assert!(
        !default_skill_row.user_invocable,
        "skill with no flags → resolved user_invocable = false",
    );

    let default_command_row = find(EntryKind::Command, "default-command");
    assert!(default_command_row.enabled);
    assert!(
        default_command_row.searchable,
        "command with no flags → resolved searchable = true",
    );
    assert!(
        default_command_row.user_invocable,
        "command with no flags → resolved user_invocable = true",
    );

    let dormant_row = find(EntryKind::Skill, "model-disabled");
    assert!(dormant_row.enabled);
    assert!(
        !dormant_row.searchable,
        "`disable-model-invocation: true` flips resolved searchable to false",
    );
    assert!(
        !dormant_row.user_invocable,
        "skill default user_invocable=false stays unchanged",
    );

    let invocable_skill_row = find(EntryKind::Skill, "user-invocable-skill");
    assert!(invocable_skill_row.enabled);
    assert!(
        invocable_skill_row.searchable,
        "skill with `user-invocable: true` keeps searchable default = true",
    );
    assert!(
        invocable_skill_row.user_invocable,
        "`user-invocable: true` flips resolved user_invocable to true",
    );
    drop(conn);

    // ------------------------------------------------------------------
    // (2) search_skills surface: three results (dormant filtered).
    // Use a broad query — the stub embedder returns deterministic
    // identical vectors so all candidate rows tie; `searchable = 1`
    // filters BEFORE ranking. top_k = 10 is the schema default; the
    // catalog only has four entries so the cap doesn't bite.
    // ------------------------------------------------------------------
    let state_for_search = build_state_with_stub_entries(&paths, PromptRegistry::default());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let search_out = rt
        .block_on(search_skills::handle(
            state_for_search,
            search_skills::Input {
                query: "matrix".into(),
                top_k: Some(10),
                catalog: None,
                plugin: None,
                description_max_chars: Some(150),
            },
        ))
        .expect("search_skills ok");

    let search_names: std::collections::BTreeSet<String> =
        search_out.matches.iter().map(|m| m.name.clone()).collect();
    let expected_search: std::collections::BTreeSet<String> = [
        "default-skill".to_string(),
        "default-command".to_string(),
        "user-invocable-skill".to_string(),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        search_names, expected_search,
        "search_skills must surface exactly the three `searchable = 1` entries; \
         got: {search_names:?}",
    );
    assert!(
        !search_names.contains("model-disabled"),
        "dormant `disable-model-invocation: true` entry MUST be excluded from search; \
         got: {search_names:?}",
    );

    // ------------------------------------------------------------------
    // (3) prompts/list surface: two descriptors (the two
    // `user_invocable = 1` entries: the command + the opt-in skill).
    // The dormant entry AND the default skill MUST be absent.
    // ------------------------------------------------------------------
    let registry = build_registry(&paths);
    // Phase 9 / US3: drop the always-on reserved built-in (orthogonal to the
    // user-invocable filtering under test).
    let prompt_names: std::collections::BTreeSet<String> = registry
        .descriptors()
        .iter()
        .map(|d| d.name.clone())
        .filter(|n| n != "add-tome-conversion-skill")
        .collect();
    let expected_prompts: std::collections::BTreeSet<String> = [
        "plug__default-command".to_string(),
        "plug__user-invocable-skill".to_string(),
    ]
    .into_iter()
    .collect();
    assert_eq!(
        prompt_names, expected_prompts,
        "prompts/list must surface exactly the two `user_invocable = 1` entries; \
         got: {prompt_names:?}",
    );
    assert!(
        !prompt_names.contains("plug__model-disabled"),
        "dormant entry MUST NOT appear in prompts/list; got: {prompt_names:?}",
    );
    assert!(
        !prompt_names.contains("plug__default-skill"),
        "default skill (user_invocable=false) MUST NOT appear in prompts/list; \
         got: {prompt_names:?}",
    );
}

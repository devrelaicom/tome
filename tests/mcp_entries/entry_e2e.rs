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
                kind: EntryKind::Skill,
                metadata_only: false,
                raw: false,
                include_resource_bodies: false,
            },
        ))
        .expect("get_skill ok");

    // (3) Body rendered through the substitution pipeline.
    let content = output.content.as_deref().unwrap();
    assert!(
        content.contains("name=pipe-skill"),
        "TOME_SKILL_NAME (Stage 1) substituted; got: {content:?}",
    );
    assert!(
        content.contains("cat=acme"),
        "TOME_CATALOG_NAME (Stage 1) substituted; got: {content:?}",
    );
    assert!(
        !content.contains("${TOME_SKILL_NAME}"),
        "no Stage-1 references must survive in the rendered body; got: {content:?}",
    );
}

/// #331: `get_skill` in the DEFAULT (rendered) vs `raw: true` (no-substitution)
/// modes over the SAME `${TOME_*}`-bearing skill body.
///
/// - Default (`raw: false`): the Stage-1 built-in `${TOME_SKILL_NAME}` is
///   substituted (token gone, value present) and `substitutions_applied` is
///   `true` — byte-identical to the pre-#331 behaviour.
/// - Raw (`raw: true`): the literal `${TOME_SKILL_NAME}` token is preserved
///   verbatim and `substitutions_applied` is `false` — exactly what an
///   authoring/converting agent needs to see the source tokens.
///
/// Both modes still resolve the entry (`kind`, `path`) — raw mode only skips
/// the substitution render, not the fetch.
#[test]
fn get_skill_raw_mode_preserves_tokens_default_mode_substitutes() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());

    // A body referencing a Stage-1 built-in token. `${TOME_SKILL_NAME}`
    // resolves deterministically from the entry name in both fixtures, so the
    // rendered/raw distinction is unambiguous.
    let skill_body =
        "---\nname: raw-skill\ndescription: raw.\n---\nname=${TOME_SKILL_NAME} literal\n";
    stage_workspace(&tmp, &paths, &[("raw-skill", skill_body)], &[]);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    // (1) Default mode: substitution runs.
    let rendered = rt
        .block_on(get_skill::handle(
            build_state(&paths, PromptRegistry::default()),
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "raw-skill".into(),
                kind: EntryKind::Skill,
                metadata_only: false,
                raw: false,
                include_resource_bodies: false,
            },
        ))
        .expect("get_skill (rendered) ok");

    let rendered_content = rendered.content.as_deref().unwrap();
    assert!(
        rendered.substitutions_applied.unwrap(),
        "default mode must report substitutions_applied = true",
    );
    assert!(
        rendered_content.contains("name=raw-skill"),
        "default mode must substitute ${{TOME_SKILL_NAME}}; got: {rendered_content:?}",
    );
    assert!(
        !rendered_content.contains("${TOME_SKILL_NAME}"),
        "default mode must leave NO literal token behind; got: {rendered_content:?}",
    );

    // (2) Raw mode: substitution SKIPPED, literal token preserved.
    let raw = rt
        .block_on(get_skill::handle(
            build_state(&paths, PromptRegistry::default()),
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "raw-skill".into(),
                kind: EntryKind::Skill,
                metadata_only: false,
                raw: true,
                include_resource_bodies: false,
            },
        ))
        .expect("get_skill (raw) ok");

    let raw_content = raw.content.as_deref().unwrap();
    assert!(
        !raw.substitutions_applied.unwrap(),
        "raw mode must report substitutions_applied = false",
    );
    assert!(
        raw_content.contains("${TOME_SKILL_NAME}"),
        "raw mode must preserve the literal ${{TOME_SKILL_NAME}} token; got: {raw_content:?}",
    );
    assert!(
        !raw_content.contains("name=raw-skill"),
        "raw mode must NOT substitute the token; got: {raw_content:?}",
    );

    // Both modes resolve the SAME entry — raw only skips the render.
    assert_eq!(rendered.kind, raw.kind, "kind identical across modes");
    assert_eq!(rendered.path, raw.path, "path identical across modes");
}

/// #331 back-compat: an `Input` JSON that OMITS `raw` deserializes to
/// `raw == false` — existing callers keep the rendered default under
/// `#[serde(deny_unknown_fields)]` because `raw` carries `#[serde(default)]`.
#[test]
fn get_skill_input_omitting_raw_defaults_to_false() {
    let input: get_skill::Input =
        serde_json::from_value(json!({ "catalog": "acme", "plugin": "plug", "name": "s" }))
            .expect("legacy Input (no `raw`) must still deserialize");
    assert!(
        !input.raw,
        "omitting `raw` must default to false (rendered mode preserved)",
    );

    // And an explicit `raw: true` round-trips.
    let explicit: get_skill::Input = serde_json::from_value(
        json!({ "catalog": "acme", "plugin": "plug", "name": "s", "raw": true }),
    )
    .expect("explicit raw:true must deserialize");
    assert!(explicit.raw, "explicit `raw: true` must set the flag");
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

/// #289: a command entry is reachable through `get_skill` (it returns the
/// command body, the resolved `kind: command`, and the MCP `prompt_name`)
/// instead of the pre-#289 `unknown_skill` dead end; `get_skill` metadata-only
/// mode and `search_skills` surface the same `prompt_name`; AND — the central promise —
/// every surfaced `prompt_name` is fed BACK into the real `prompts/get` path
/// (`prompts::handle_get`) and resolves to THIS command's body. Driving the
/// surfaced name (never a hardcoded literal) through the live prompt router
/// makes "the name we hand you is invocable" non-bypassable: a future
/// name-derivation / collision-suffix regression must fail HERE rather than
/// passing because a literal was edited to match the bug.
///
/// All resolution goes through the REAL `PromptRegistry` built from the
/// on-disk index — the production SSOT path.
#[test]
fn command_reachable_via_get_skill_and_search_carries_prompt_name() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());

    // A body marker unique enough that a round-tripped prompt render can't
    // accidentally match another entry.
    let body_marker = "run the deploy now-289";
    let cmd_body =
        format!("---\nname: deploy\ndescription: Deploy the service.\n---\n{body_marker}\n");
    stage_workspace(&tmp, &paths, &[], &[("deploy", cmd_body.as_str())]);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    // (1) get_skill on the COMMAND must NOT be an `unknown_skill` dead end.
    // It resolves the command, returns its body, `kind: command`, and the
    // derived MCP prompt name.
    let output = rt
        .block_on(get_skill::handle(
            build_state(&paths, build_registry(&paths)),
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "deploy".into(),
                kind: EntryKind::Skill,
                metadata_only: false,
                raw: false,
                include_resource_bodies: false,
            },
        ))
        .expect("get_skill must resolve the command, not return unknown_skill");

    let output_content = output.content.as_deref().unwrap();
    assert!(
        output_content.contains(body_marker),
        "get_skill returns the command body; got: {output_content:?}",
    );
    assert_eq!(
        output.kind,
        EntryKind::Command,
        "get_skill reports the resolved kind as command",
    );
    let get_skill_prompt = output
        .prompt_name
        .clone()
        .expect("get_skill must surface a prompt_name for a user-invocable command");
    let output_resources = output.resources_paths.as_deref().unwrap();
    assert!(
        output_resources.is_empty(),
        "a command has no sibling-resource enumeration; got: {output_resources:?}",
    );

    // (2) get_skill (metadata_only), through the production handler against the
    // REAL registry, surfaces the same prompt_name (item 4 — not just the unit
    // wire-pin with a literal).
    let info = rt
        .block_on(get_skill::handle(
            build_state(&paths, build_registry(&paths)),
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "deploy".into(),
                kind: EntryKind::Command,
                metadata_only: true,
                raw: false,
                include_resource_bodies: false,
            },
        ))
        .expect("get_skill (metadata_only) ok for the command");
    let info_prompt = info.prompt_name.clone().expect(
        "get_skill (metadata_only) must surface a prompt_name for a user-invocable command",
    );

    // (3) search_skills surfaces the command with its prompt_name so the
    // ranked result is immediately actionable.
    let search_out = rt
        .block_on(search_skills::handle(
            build_state_with_stub_entries(&paths, build_registry(&paths)),
            search_skills::Input {
                query: "deploy".into(),
                top_k: Some(10),
                catalog: None,
                plugin: None,
                kind: None,
                rerank: None,
                min_score: None,
                description_max_chars: Some(150),
            },
        ))
        .expect("search_skills ok");

    let hit = search_out
        .matches
        .iter()
        .find(|m| m.name == "deploy" && m.kind == EntryKind::Command)
        .expect("search must rank the command");
    let search_prompt = hit
        .prompt_name
        .clone()
        .expect("search_skills must surface a prompt_name for a user-invocable command");

    // All three read surfaces must agree on the SAME prompt name (the SSOT).
    assert_eq!(
        get_skill_prompt, info_prompt,
        "get_skill (body) and get_skill (metadata_only) must surface the identical prompt_name",
    );
    assert_eq!(
        get_skill_prompt, search_prompt,
        "get_skill and search_skills must surface the identical prompt_name",
    );

    // (4) The CENTRAL PROMISE: feed the SURFACED name straight back into the
    // real `prompts/get` path and assert it renders THIS command's body. The
    // name is whatever the read tools produced (override + collision-suffix
    // included), NOT a literal — so a derivation regression fails here.
    let rendered = rt
        .block_on(prompts::handle_get(
            build_state(&paths, build_registry(&paths)),
            get_skill_prompt.clone(),
            None,
        ))
        .unwrap_or_else(|e| {
            panic!("surfaced prompt_name `{get_skill_prompt}` must resolve via prompts/get: {e:?}")
        });
    assert_eq!(rendered.messages.len(), 1, "single user-role message");
    let text = match &rendered.messages[0].content {
        rmcp::model::PromptMessageContent::Text { text } => text.clone(),
        other => panic!("expected text content, got {other:?}"),
    };
    assert!(
        text.contains(body_marker),
        "the surfaced prompt_name must render THIS command's body via prompts/get; got: {text:?}",
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
                kind: None,
                rerank: None,
                min_score: None,
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

// ---------------------------------------------------------------------------
// #333 — get_skill `include_resource_bodies`: byte-capped inline of small text
// resources as `{ path, content }`.
// ---------------------------------------------------------------------------

/// Write extra sibling resource files into an enabled skill's on-disk
/// directory (reachable via the cache symlink). The skill body was staged with
/// `stage_workspace`; these land alongside the `SKILL.md` so `get_skill`'s
/// resource walk enumerates them.
fn write_skill_resource(catalog_root: &Path, skill: &str, rel: &str, bytes: &[u8]) {
    let dir = catalog_root.join("plug").join("skills").join(skill);
    let path = dir.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

/// `include_resource_bodies: true` inlines small text resources as
/// `{ path, content }` (content byte-exact), skips a binary/non-UTF-8
/// resource (its path stays in `resources`, absent from `resource_bodies`),
/// and leaves the flag-off output free of the key.
#[test]
fn get_skill_inlines_small_text_resources_and_skips_binary() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());

    let skill_body = "---\nname: res-skill\ndescription: has resources.\n---\nbody\n";
    let catalog_root = stage_workspace(&tmp, &paths, &[("res-skill", skill_body)], &[]);

    // A UTF-8 text resource (inlined) + a binary resource (skipped). Invalid
    // UTF-8 bytes: a lone 0xFF is never valid UTF-8.
    write_skill_resource(
        &catalog_root,
        "res-skill",
        "notes.txt",
        b"hello resources\n",
    );
    write_skill_resource(
        &catalog_root,
        "res-skill",
        "blob.bin",
        &[0xFF, 0xFE, 0x00, 0x01],
    );

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    // Flag off → `resource_bodies` absent (None) but both resources still in `resources`.
    let off = rt
        .block_on(get_skill::handle(
            build_state(&paths, PromptRegistry::default()),
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "res-skill".into(),
                kind: EntryKind::Skill,
                metadata_only: false,
                raw: false,
                include_resource_bodies: false,
            },
        ))
        .expect("get_skill (flag off) ok");
    assert!(
        off.resource_bodies.is_none(),
        "flag off must leave resource_bodies None; got: {:?}",
        off.resource_bodies,
    );
    // Both files enumerated in `resources` regardless of the flag.
    let off_resources = off.resources_paths.as_deref().unwrap();
    assert!(off_resources.iter().any(|p| p.ends_with("notes.txt")));
    assert!(off_resources.iter().any(|p| p.ends_with("blob.bin")));

    // Flag on → the text resource is inlined; the binary one is skipped.
    let on = rt
        .block_on(get_skill::handle(
            build_state(&paths, PromptRegistry::default()),
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "res-skill".into(),
                kind: EntryKind::Skill,
                metadata_only: false,
                raw: false,
                include_resource_bodies: true,
            },
        ))
        .expect("get_skill (flag on) ok");

    // `resources` still lists BOTH files (resource_bodies is a parallel view).
    let on_resources = on.resources_paths.as_deref().unwrap();
    assert!(on_resources.iter().any(|p| p.ends_with("notes.txt")));
    assert!(on_resources.iter().any(|p| p.ends_with("blob.bin")));

    let bodies = on
        .resource_bodies
        .expect("flag on must populate resource_bodies");
    // Only the text file is inlined, with byte-exact content.
    assert_eq!(
        bodies.len(),
        1,
        "binary resource must be skipped; got: {bodies:?}"
    );
    assert!(bodies[0].path.ends_with("notes.txt"));
    assert_eq!(bodies[0].content, "hello resources\n");
    assert!(
        !bodies.iter().any(|b| b.path.ends_with("blob.bin")),
        "non-UTF-8 resource must NOT be inlined; got: {bodies:?}",
    );
}

/// An over-per-file-cap resource (> 64 KiB) is skipped even though it's valid
/// text; its path stays in `resources`.
#[test]
fn get_skill_skips_resource_over_per_file_cap() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());

    let skill_body = "---\nname: big-res\ndescription: oversized resource.\n---\nbody\n";
    let catalog_root = stage_workspace(&tmp, &paths, &[("big-res", skill_body)], &[]);

    // Small text (inlined) + a 65 KiB text file (> the 64 KiB per-file cap → skipped).
    write_skill_resource(&catalog_root, "big-res", "small.txt", b"tiny\n");
    let big = "A".repeat(65 * 1024);
    write_skill_resource(&catalog_root, "big-res", "huge.txt", big.as_bytes());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let out = rt
        .block_on(get_skill::handle(
            build_state(&paths, PromptRegistry::default()),
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "big-res".into(),
                kind: EntryKind::Skill,
                metadata_only: false,
                raw: false,
                include_resource_bodies: true,
            },
        ))
        .expect("get_skill ok");

    // Both still enumerated in `resources`.
    assert!(
        out.resources_paths
            .as_deref()
            .unwrap()
            .iter()
            .any(|p| p.ends_with("huge.txt"))
    );

    let bodies = out.resource_bodies.expect("resource_bodies present");
    assert_eq!(
        bodies.len(),
        1,
        "over-per-file-cap file must be skipped; got: {bodies:?}"
    );
    assert!(bodies[0].path.ends_with("small.txt"));
    assert!(
        !bodies.iter().any(|b| b.path.ends_with("huge.txt")),
        "over-cap resource must NOT be inlined",
    );
}

/// The whole-response TOTAL budget (1 MiB) caps the inlined set: a directory of
/// many ~50 KiB files inlines a prefix that fits, and the remaining paths still
/// appear in `resources`. The total inlined content never exceeds the budget.
#[test]
fn get_skill_total_budget_caps_inlined_set() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());

    let skill_body = "---\nname: many-res\ndescription: many resources.\n---\nbody\n";
    let catalog_root = stage_workspace(&tmp, &paths, &[("many-res", skill_body)], &[]);

    // 40 files × 50 KiB = 2000 KiB of text — each under the 64 KiB per-file cap,
    // but well over the 1 MiB (1024 KiB) whole-response budget in aggregate.
    // Zero-padded names so the sorted `resources` order is deterministic.
    let chunk = "B".repeat(50 * 1024);
    for i in 0..40 {
        write_skill_resource(
            &catalog_root,
            "many-res",
            &format!("part-{i:02}.txt"),
            chunk.as_bytes(),
        );
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let out = rt
        .block_on(get_skill::handle(
            build_state(&paths, PromptRegistry::default()),
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "many-res".into(),
                kind: EntryKind::Skill,
                metadata_only: false,
                raw: false,
                include_resource_bodies: true,
            },
        ))
        .expect("get_skill ok");

    // All 40 files are enumerated in `resources` (the walk isn't budget-gated).
    assert_eq!(
        out.resources_paths.as_deref().unwrap().len(),
        40,
        "all resources enumerated"
    );

    let bodies = out.resource_bodies.expect("resource_bodies present");
    // The budget stops inlining before all 40 — a strict prefix is inlined.
    assert!(
        bodies.len() < 40,
        "total budget must cap the inlined set below the full 40; got {}",
        bodies.len(),
    );
    assert!(!bodies.is_empty(), "at least the first files must inline");
    // Hard bound: total inlined content bytes never exceed the 1 MiB budget.
    let total: usize = bodies.iter().map(|b| b.content.len()).sum();
    assert!(
        total as u64 <= 1024 * 1024,
        "inlined total {total} bytes must not exceed the 1 MiB whole-response budget",
    );
    // 50 KiB chunks: 1 MiB / 50 KiB = 20 full files fit (20 * 50 = 1000 KiB ≤ 1024 KiB).
    assert_eq!(
        bodies.len(),
        20,
        "expected exactly 20 × 50 KiB files under a 1 MiB budget"
    );
}

/// A command entry never carries resource bodies — even with the flag set —
/// because commands have no per-entry resource directory (`resources` empty,
/// `resource_bodies` None).
#[test]
fn get_skill_command_omits_resource_bodies() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());

    let cmd_body = "---\nname: run-it\ndescription: a command.\n---\ndo the thing\n";
    stage_workspace(&tmp, &paths, &[], &[("run-it", cmd_body)]);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let out = rt
        .block_on(get_skill::handle(
            build_state(&paths, PromptRegistry::default()),
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "run-it".into(),
                kind: EntryKind::Skill,
                metadata_only: false,
                raw: false,
                include_resource_bodies: true,
            },
        ))
        .expect("get_skill ok for command");

    assert_eq!(out.kind, EntryKind::Command);
    assert!(
        out.resources_paths.as_deref().unwrap().is_empty(),
        "command has no resources"
    );
    assert!(
        out.resource_bodies.is_none(),
        "command must omit resource_bodies even with the flag set; got: {:?}",
        out.resource_bodies,
    );
}

/// #333 symlink-omission parity (mirrors
/// `get_skill::walk_resources_skips_symlinks`): a symlinked resource is
/// NEVER enumerated by `walk_dir` (lstat-refused), so it CANNOT be inlined by
/// the new `include_resource_bodies` read path — it appears in neither
/// `resources` nor `resource_bodies`, while the real sibling file is inlined.
/// Locks the by-construction symlink omission at the inline read site.
#[cfg(unix)]
#[test]
fn get_skill_inline_omits_symlinked_resource() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());

    let skill_body = "---\nname: link-res\ndescription: has a symlink resource.\n---\nbody\n";
    let catalog_root = stage_workspace(&tmp, &paths, &[("link-res", skill_body)], &[]);

    // A real resource file (inlined) + a symlink to an out-of-dir secret. The
    // symlink target lives OUTSIDE the skill dir so, if the skip regressed, the
    // symlink (not a coincidental real sibling) would be the thing enumerated.
    write_skill_resource(&catalog_root, "link-res", "real.txt", b"real content\n");
    let secret_tmp = TempDir::new().unwrap();
    let secret = secret_tmp.path().join("secret.txt");
    fs::write(&secret, "SECRET").unwrap();
    let skill_dir = catalog_root.join("plug").join("skills").join("link-res");
    std::os::unix::fs::symlink(&secret, skill_dir.join("link.txt")).unwrap();

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let out = rt
        .block_on(get_skill::handle(
            build_state(&paths, PromptRegistry::default()),
            get_skill::Input {
                catalog: "acme".into(),
                plugin: "plug".into(),
                name: "link-res".into(),
                kind: EntryKind::Skill,
                metadata_only: false,
                raw: false,
                include_resource_bodies: true,
            },
        ))
        .expect("get_skill ok");

    // The symlink is enumerated by neither the path list nor the inline view.
    let out_resources = out.resources_paths.as_deref().unwrap();
    assert!(
        !out_resources.iter().any(|p| p.ends_with("link.txt")),
        "symlinked resource must not be enumerated in `resources`; got: {out_resources:?}",
    );
    assert!(
        out_resources.iter().any(|p| p.ends_with("real.txt")),
        "the real sibling resource must be enumerated",
    );

    let bodies = out.resource_bodies.expect("resource_bodies present");
    assert!(
        !bodies.iter().any(|b| b.path.ends_with("link.txt")),
        "symlinked resource must NOT be inlined; got: {bodies:?}",
    );
    // Defence in depth: the secret content must never leak into any inlined body.
    assert!(
        !bodies.iter().any(|b| b.content.contains("SECRET")),
        "out-of-dir secret content must never appear in resource_bodies",
    );
    // The real file IS inlined with byte-exact content.
    assert_eq!(
        bodies.len(),
        1,
        "only the real resource inlines; got: {bodies:?}"
    );
    assert!(bodies[0].path.ends_with("real.txt"));
    assert_eq!(bodies[0].content, "real content\n");
}

/// #333 back-compat: an `Input` JSON omitting `include_resource_bodies`
/// deserializes to `false` under `deny_unknown_fields` (the `#[serde(default)]`).
#[test]
fn get_skill_input_omitting_include_resource_bodies_defaults_to_false() {
    let input: get_skill::Input =
        serde_json::from_value(json!({ "catalog": "acme", "plugin": "plug", "name": "s" }))
            .expect("legacy Input (no include_resource_bodies) must still deserialize");
    assert!(
        !input.include_resource_bodies,
        "omitting include_resource_bodies must default to false",
    );

    let explicit: get_skill::Input = serde_json::from_value(json!({
        "catalog": "acme", "plugin": "plug", "name": "s", "include_resource_bodies": true
    }))
    .expect("explicit include_resource_bodies:true must deserialize");
    assert!(explicit.include_resource_bodies);
}

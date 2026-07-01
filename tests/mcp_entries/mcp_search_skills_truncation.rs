//! Phase 5 / US4.c — `search_skills` MCP tool truncation, kind, and
//! `searchable` filter at the library API.
//!
//! Drives the real handler against a staged workspace + indexed plugin
//! using the StubEmbedder + StubReranker (no ONNX models needed).
//! Mirrors the `mcp_get_skill_info.rs` staging discipline: a single
//! tempdir hosts the `Paths` root, the catalog clone, and a symlink
//! wired up so `paths.cache_dir_for(url)` resolves into the same
//! on-disk directory that the lifecycle pipeline indexed.
//!
//! Covers `contracts/mcp-tools-p5.md` § `search_skills` (extended):
//!
//! - Default `description_max_chars = 150` truncates with `…` (U+2026).
//! - `description_max_chars` override (e.g. 50) honoured.
//! - Description shorter than cap returned verbatim.
//! - `kind` field present in each result (`skill` and `command`).
//! - `disable-model-invocation: true` excludes entries from results
//!   (FR-090: `WHERE searchable = 1`).
//! - `description_max_chars > MAX_DESCRIPTION_MAX_CHARS` rejected with
//!   `invalid_description_max_chars` MCP envelope.

use std::fs;
use std::path::Path;
use std::sync::Arc;

use tempfile::TempDir;
use tokio::sync::OnceCell;
use tome::embedding::Reranker;
use tome::embedding::registry::{ModelEntry, ModelKind};
use tome::embedding::stub::{StubEmbedder, StubReranker};
use tome::index::{self, OpenOptions};
use tome::mcp::prompts::PromptRegistry;
use tome::mcp::state::McpState;
use tome::mcp::tools::search_skills::{self, Input, MAX_DESCRIPTION_MAX_CHARS, NoResultsReason};
use tome::plugin::PluginId;
use tome::plugin::identity::EntryKind;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

use crate::common::{
    config_with_catalog, fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

// ---------------------------------------------------------------------------
// Fixture helpers (cloned from `tests/mcp_get_skill_info.rs` — symlink-
// cache wiring is non-trivial and the staging code stays test-local
// until the third caller).
// ---------------------------------------------------------------------------

fn build_deps<'a>(
    paths: &'a tome::paths::Paths,
    config: &'a tome::config::Config,
    embedder: &'a StubEmbedder,
    scope: &'a tome::workspace::Scope,
) -> LifecycleDeps<'a> {
    LifecycleDeps {
        paths,
        scope,
        config,
        embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    }
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

fn global() -> WorkspaceName {
    WorkspaceName::global()
}

/// Stage a workspace with one plugin enabled. `skills` and `commands`
/// are `(name, body)` tuples — the body is written verbatim to
/// `SKILL.md` / `<name>.md` so callers can shape the frontmatter
/// (description length, `disable-model-invocation`, etc.) freely.
fn stage_workspace(
    skills: &[(&str, &str)],
    commands: &[(&str, &str)],
) -> (TempDir, tome::paths::Paths) {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);

    let plugin_dir = catalog_root.join("plug");
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
        r#"{"name": "plug", "version": "1.0.0"}"#,
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

    let embedder = StubEmbedder::new();
    let scope = tome::workspace::Scope(global());
    let deps = build_deps(&paths, &config, &embedder, &scope);
    let id: PluginId = "acme/plug".parse().unwrap();
    // FF1: enrolment + cache symlink before enable — resolve_plugin_dir now
    // reads workspace_catalogs, not the in-memory Config.
    seed_catalog_enrolment(&paths, &catalog_root, "acme");
    lifecycle::enable(&id, &deps).expect("enable plugin");
    (tmp, paths)
}

/// Insert a `workspace_catalogs` row for `global` and symlink the
/// hashed cache dir to the on-disk catalog directory so
/// `paths.cache_dir_for(url)` resolves into a real layout.
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

/// Stub embedder `ModelEntry`. The `McpState` field requires a
/// `&'static ModelEntry`, and `search_skills`'s pipeline pulls the
/// `MetaSeed` (name + version) from it for drift detection. Using the
/// real `bge-small-en-v1.5` entry would mismatch the index — which
/// was indexed by `StubEmbedder` (`model_name() == "stub-embedder"`)
/// — and surface as `embedder_drift`. So we declare a static stub
/// entry whose name/version match the stub's reported identity.
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

/// Build the `Arc<McpState>` the handler expects. The prompt registry
/// is empty — `search_skills` doesn't consume it; we still wire it so
/// the state shape stays valid.
fn build_state(paths: &tome::paths::Paths) -> Arc<McpState> {
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
        prompt_registry: Arc::new(std::sync::RwLock::new(Arc::new(PromptRegistry::default()))),
        host_harness: None,
        last_search_ranks: std::sync::Mutex::new(std::collections::HashMap::new()),
    })
}

fn invoke(state: Arc<McpState>, input: Input) -> Result<search_skills::Output, rmcp::ErrorData> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(search_skills::handle(state, input))
}

fn make_input(query: &str, description_max_chars: u32) -> Input {
    Input {
        query: query.into(),
        top_k: Some(10),
        catalog: None,
        plugin: None,
        description_max_chars: Some(description_max_chars),
    }
}

/// Build a body with a description of exactly `n` ASCII characters so
/// truncation tests have a known input length.
fn long_skill_body(name: &str, description_len: usize) -> String {
    let description: String = "a".repeat(description_len);
    format!("---\nname: {name}\ndescription: {description}\n---\nbody\n")
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn default_description_max_chars_truncates_at_150_with_ellipsis() {
    // 300-char description; default cap is 150 → 150 chars + `…`.
    let body = long_skill_body("longish", 300);
    let (_tmp, paths) = stage_workspace(&[("longish", body.as_str())], &[]);
    let state = build_state(&paths);

    // Round-trip via JSON so the serde default fires — that's the
    // production wire path, not the in-struct default.
    let raw = serde_json::json!({"query": "longish"});
    let input: Input = serde_json::from_value(raw).expect("deserialise default cap");
    assert_eq!(
        input.description_max_chars, None,
        "description_max_chars absent from JSON → None (resolved to 150 during handle)"
    );

    let out = invoke(state, input).expect("search ok");
    assert!(!out.matches.is_empty(), "must surface at least one match");

    let m = &out.matches[0];
    let chars: usize = m.description.chars().count();
    assert_eq!(
        chars, 151,
        "default truncation yields 150 content chars + 1 ellipsis (`…`)"
    );
    assert!(
        m.description.ends_with('\u{2026}'),
        "truncated descriptions must end with U+2026 (`…`); got: {:?}",
        m.description,
    );
}

#[test]
fn override_description_max_chars_honoured() {
    let body = long_skill_body("longish", 300);
    let (_tmp, paths) = stage_workspace(&[("longish", body.as_str())], &[]);
    let state = build_state(&paths);

    let out = invoke(state, make_input("longish", 50)).expect("search ok");
    assert!(!out.matches.is_empty());
    let m = &out.matches[0];
    assert_eq!(
        m.description.chars().count(),
        51,
        "override cap of 50 yields 50 content chars + 1 ellipsis"
    );
    assert!(m.description.ends_with('\u{2026}'));
}

#[test]
fn description_shorter_than_cap_returned_verbatim() {
    // Description is only 12 chars; default cap of 150 must not touch
    // it, and no ellipsis must be appended.
    let body = "---\nname: short\ndescription: hello, world\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("short", body)], &[]);
    let state = build_state(&paths);

    let out = invoke(state, make_input("short", 150)).expect("search ok");
    assert!(!out.matches.is_empty());
    let m = &out.matches[0];
    assert_eq!(
        m.description, "hello, world",
        "short descriptions must round-trip verbatim, no ellipsis appended"
    );
    assert!(!m.description.contains('\u{2026}'));
}

#[test]
fn skill_match_carries_kind_skill() {
    let body = "---\nname: just-a-skill\ndescription: A skill row.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("just-a-skill", body)], &[]);
    let state = build_state(&paths);

    let out = invoke(state, make_input("just-a-skill", 150)).expect("search ok");
    assert!(!out.matches.is_empty());
    assert!(
        matches!(out.matches[0].kind, EntryKind::Skill),
        "skill row must surface with kind = Skill, got: {:?}",
        out.matches[0].kind,
    );
}

#[test]
fn command_match_carries_kind_command() {
    // A command (default user-invocable=true, default searchable=true)
    // must surface in search results with `kind: command` per FR-091.
    let cmd_body = "---\nname: fix-issue\ndescription: Fix a GitHub issue.\n---\nGo fix it.\n";
    let (_tmp, paths) = stage_workspace(&[], &[("fix-issue", cmd_body)]);
    let state = build_state(&paths);

    let out = invoke(state, make_input("fix-issue", 150)).expect("search ok");
    assert!(
        !out.matches.is_empty(),
        "command rows must be returned by search by default (searchable=true)"
    );
    assert!(
        out.matches
            .iter()
            .any(|m| matches!(m.kind, EntryKind::Command)),
        "at least one result must carry kind = Command, got: {:?}",
        out.matches.iter().map(|m| m.kind).collect::<Vec<_>>(),
    );
}

#[test]
fn disable_model_invocation_excluded_from_results() {
    // Two entries: one searchable, one with disable-model-invocation:
    // true. The KNN candidate set must include ONLY the searchable
    // row regardless of query similarity.
    let searchable_body = "---\nname: searchable\ndescription: a normal skill.\n---\nbody\n";
    let opted_out_body = "---
name: opted-out
description: a hidden skill.
disable-model-invocation: true
---
body
";
    let (_tmp, paths) = stage_workspace(
        &[
            ("searchable", searchable_body),
            ("opted-out", opted_out_body),
        ],
        &[],
    );
    let state = build_state(&paths);

    // Query the opted-out name directly. Even a perfect text hit must
    // not surface the row because `WHERE searchable = 1` filters it
    // out (FR-090).
    let out = invoke(state, make_input("opted-out", 150)).expect("search ok");
    let names: Vec<&str> = out.matches.iter().map(|m| m.name.as_str()).collect();
    assert!(
        !names.contains(&"opted-out"),
        "disable-model-invocation: true entries MUST be excluded by `WHERE searchable = 1`; got: {names:?}",
    );
    assert!(
        names.contains(&"searchable"),
        "regular searchable entries must still surface; got: {names:?}",
    );
}

#[test]
fn description_max_chars_above_sanity_cap_returns_invalid_envelope() {
    // No staged workspace needed — the validator fires before any
    // index or config touch. Build a minimal state.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);
    fs::write(&paths.global_config_file, "[catalogs]\n").unwrap();
    let state = build_state(&paths);

    let bad = MAX_DESCRIPTION_MAX_CHARS + 1;
    let err = invoke(state, make_input("anything", bad))
        .expect_err("description_max_chars above sanity cap must reject");
    let data = err.data.expect("structured error envelope");
    assert_eq!(
        data.get("code").and_then(|c| c.as_str()),
        Some("invalid_description_max_chars"),
        "expected `invalid_description_max_chars` code in data, got: {data}",
    );
    assert_eq!(
        data.get("max").and_then(|n| n.as_u64()),
        Some(u64::from(MAX_DESCRIPTION_MAX_CHARS)),
        "expected max hint in data, got: {data}",
    );
}

#[test]
fn description_max_chars_at_sanity_cap_accepted() {
    // Exactly at MAX_DESCRIPTION_MAX_CHARS must NOT trigger the
    // validator — only strictly above does (mirrors the
    // MAX_QUERY_CHARS boundary discipline from Phase 4 US5.a).
    let body = "---\nname: short\ndescription: small\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("short", body)], &[]);
    let state = build_state(&paths);

    let result = invoke(state, make_input("short", MAX_DESCRIPTION_MAX_CHARS));
    // The boundary case may succeed or fail for OTHER reasons; the
    // point is that it must NOT fail with `invalid_description_max_chars`.
    if let Err(err) = result
        && let Some(data) = err.data
        && let Some(code) = data.get("code").and_then(|c| c.as_str())
    {
        assert_ne!(
            code, "invalid_description_max_chars",
            "exactly MAX_DESCRIPTION_MAX_CHARS must NOT trigger the validator",
        );
    }
}

#[test]
fn description_max_chars_zero_yields_empty_description() {
    // Edge case: caller passes 0 (legal — opt-in to fully empty
    // descriptions in the result). The truncator returns "" and never
    // appends an ellipsis (defensive — ellipsis at 0 makes no sense).
    let body = "---\nname: short\ndescription: hello, world\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("short", body)], &[]);
    let state = build_state(&paths);

    let out = invoke(state, make_input("short", 0)).expect("search ok");
    assert!(!out.matches.is_empty());
    assert_eq!(
        out.matches[0].description, "",
        "description_max_chars = 0 must yield empty string with no ellipsis"
    );
}

#[test]
fn truncation_at_multibyte_char_boundary_does_not_split_codepoint() {
    // US4.d test-gap fix: multi-byte UTF-8 characters at the truncation
    // boundary must not be split. Description with 100 emoji (4 bytes
    // each in UTF-8) + truncate at 50 chars → output should be 50 emoji
    // + ellipsis, not 50 chars worth of garbled bytes.
    //
    // The bug we're guarding against: a byte-based truncation would
    // slice mid-codepoint and produce invalid UTF-8 OR a U+FFFD
    // replacement. The char_indices-based implementation (US4.d C-2)
    // walks char boundaries and slices at a valid offset.
    let emoji = "🎯"; // 4 UTF-8 bytes, 1 char
    let description: String = emoji.repeat(100); // 400 bytes, 100 chars
    let body = format!("---\nname: emoji\ndescription: {description}\n---\nbody\n");
    let (_tmp, paths) = stage_workspace(&[("emoji", &body)], &[]);
    let state = build_state(&paths);

    let out = invoke(state, make_input("emoji", 50)).expect("search ok");
    assert!(!out.matches.is_empty());
    let truncated = &out.matches[0].description;
    // 50 emoji + 1 ellipsis = 51 chars. Verify char count, not byte count.
    assert_eq!(
        truncated.chars().count(),
        51,
        "truncation at multibyte boundary must produce 50 chars + 1 ellipsis (51 total), got {} chars",
        truncated.chars().count()
    );
    // Must be valid UTF-8 (would already error during deserialisation
    // if not, but assert explicitly via successful chars() iteration).
    assert!(truncated.ends_with('\u{2026}'), "must end with ellipsis");
    let prefix_emoji_count = truncated.chars().filter(|c| *c == '🎯').count();
    assert_eq!(prefix_emoji_count, 50, "must contain exactly 50 emoji");
}

/// Task 11: `[mcp] description_max_chars` in config.toml is used when the
/// per-call `description_max_chars` is absent (`None`).
///
/// The input is deserialized from a JSON value that **omits** the
/// `description_max_chars` key — mirroring the real MCP wire path and proving
/// that `#[serde(default)]` → `None` → config fallback resolves correctly.
/// The sanity cap on the RESOLVED value (after config fallback) applies if the
/// config-supplied value exceeds `MAX_DESCRIPTION_MAX_CHARS`.
#[test]
fn config_description_max_chars_used_when_call_arg_absent() {
    let body = long_skill_body("toolong", 300);
    let (_tmp, paths) = stage_workspace(&[("toolong", body.as_str())], &[]);

    // Write description_max_chars = 50 to config — no per-call arg.
    std::fs::write(
        &paths.global_config_file,
        "[catalogs]\n\n[mcp]\ndescription_max_chars = 50\n",
    )
    .unwrap();

    let state = build_state(&paths);

    // Deserialise from JSON that OMITS description_max_chars — this is the real
    // MCP wire path (the key is simply absent in the JSON payload).  The
    // #[serde(default)] attribute must yield None, which the handler then
    // resolves to the config value (50).
    let raw = serde_json::json!({"query": "toolong", "top_k": 10});
    let input: Input = serde_json::from_value(raw).expect("deserialise input without cap key");
    assert_eq!(
        input.description_max_chars, None,
        "description_max_chars absent from JSON must deserialise to None (serde default)"
    );

    let out = invoke(state, input).expect("search ok");
    assert!(!out.matches.is_empty(), "expected matches");
    let chars = out.matches[0].description.chars().count();
    assert_eq!(
        chars, 51,
        "config description_max_chars=50 should give 50 chars + ellipsis (51 total), got {chars}"
    );
    assert!(
        out.matches[0].description.ends_with('\u{2026}'),
        "truncated description must end with ellipsis"
    );
}

// ---------------------------------------------------------------------------
// #285 — empty/weak-result signal (corpus_size / scoring / reranker_drift /
// no_results_reason / hint).
// ---------------------------------------------------------------------------

/// Build an `McpState` whose resolved scope is an arbitrary named
/// workspace (not `global`). Used by the "populated index, no scoped match"
/// case: the whole-index `corpus_size` stays > 0 while the scoped KNN join
/// (`workspace_skills` for this name) yields zero rows.
fn build_state_for_scope(paths: &tome::paths::Paths, workspace: &str) -> Arc<McpState> {
    let base = build_state(paths);
    // `McpState` fields are public; clone the shared handles and swap the scope.
    Arc::new(McpState {
        embedder: base.embedder.clone(),
        reranker: OnceCell::new_with(Some(Arc::new(StubReranker::new()) as Arc<dyn Reranker>)),
        scope: ResolvedScope {
            scope: Scope(WorkspaceName::parse(workspace).expect("valid workspace name")),
            source: ScopeSource::Flag,
            project_root: None,
        },
        paths: paths.clone(),
        embedder_entry: base.embedder_entry,
        embedder_seed: base.embedder_seed.clone(),
        reranker_entry: base.reranker_entry,
        prompt_registry: base.prompt_registry.clone(),
        host_harness: base.host_harness.clone(),
        last_search_ranks: std::sync::Mutex::new(std::collections::HashMap::new()),
    })
}

#[test]
fn non_empty_search_reports_corpus_size_and_scoring() {
    // #285: a normal populated-index search carries the always-present
    // `corpus_size` (> 0) and `scoring` (`reranked`, since the StubReranker is
    // wired) — and NONE of the empty-result signal fields.
    let body = "---\nname: findme\ndescription: A findable skill.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("findme", body)], &[]);
    let state = build_state(&paths);

    let out = invoke(state, make_input("findme", 150)).expect("search ok");
    assert!(!out.matches.is_empty(), "expected at least one match");
    assert!(
        out.corpus_size > 0,
        "populated index must report corpus_size > 0, got {}",
        out.corpus_size
    );
    assert_eq!(
        out.scoring, "reranked",
        "a reranked search must report scoring = reranked (StubReranker wired), got {:?}",
        out.scoring
    );
    // Signal fields are absent on the common (non-empty) path.
    assert!(
        out.no_results_reason.is_none(),
        "no_results_reason must be absent on a non-empty result"
    );
    assert!(
        out.hint.is_none(),
        "hint must be absent on a non-empty result"
    );
    assert!(
        out.reranker_drift.is_none(),
        "no drift expected when the stub seed matches the index"
    );
}

#[test]
fn empty_corpus_search_reports_index_empty_reason_and_reindex_hint() {
    // #285: an empty index (zero searchable entries) returns
    // corpus_size == 0 with `no_results_reason: index_empty` + a reindex hint.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);
    // Bootstrap an EMPTY index with the STUB seeds so the drift check is clean
    // (a fresh `open_index_for_read` would seed the real BGE registry identity,
    // tripping embedder drift against the stub state).
    let _conn = open_index(&paths);
    let state = build_state(&paths);

    let out = invoke(state, make_input("anything", 150)).expect("search ok on empty index");
    assert!(
        out.matches.is_empty(),
        "an empty index must return zero matches, got {}",
        out.matches.len()
    );
    assert_eq!(
        out.corpus_size, 0,
        "empty index must report corpus_size == 0"
    );
    assert_eq!(
        out.no_results_reason,
        Some(NoResultsReason::IndexEmpty),
        "empty index must report no_results_reason = index_empty"
    );
    let hint = out.hint.expect("empty index must carry a hint");
    assert!(
        hint.contains("reindex"),
        "empty-index hint must mention reindex; got: {hint:?}"
    );
}

#[test]
fn empty_scope_with_content_elsewhere_reports_index_empty_not_no_match() {
    // #285 review fix: the WHOLE index has content (a skill under `global`) but
    // the RESOLVED scope has zero enrolled/searchable skills — the scoped KNN
    // returns nothing. This is an `index_empty`-for-this-scope situation: the
    // fix is to reindex / enable a plugin FOR THIS SCOPE, NOT to rephrase.
    //
    // The `corpus_size` on the Output is the SCOPE-EFFECTIVE searchable count
    // (== 0 here), NOT the whole-index count — so the discriminant is
    // self-consistent (`corpus_size == 0` ⇔ `index_empty`). Before the fix the
    // handler used the whole-index count and wrongly emitted `no_match`.
    let body = "---\nname: elsewhere\ndescription: Indexed under global only.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("elsewhere", body)], &[]);
    // Query under a DIFFERENT (empty) workspace: the whole-index corpus is
    // non-empty, but the scoped join for this workspace yields zero rows.
    let state = build_state_for_scope(&paths, "no-such-workspace");

    let out = invoke(state, make_input("elsewhere", 150)).expect("search ok");
    assert!(
        out.matches.is_empty(),
        "a scope with no enrolled skills must return zero matches, got {}",
        out.matches.len()
    );
    assert_eq!(
        out.corpus_size, 0,
        "corpus_size must be the SCOPE-EFFECTIVE count (0 for an empty scope), \
         NOT the whole-index count; got {}",
        out.corpus_size
    );
    assert_eq!(
        out.no_results_reason,
        Some(NoResultsReason::IndexEmpty),
        "an empty scope (even with content in another scope) must report index_empty, \
         not no_match — the fix is to reindex/enable for this scope, not to rephrase"
    );
    let hint = out.hint.expect("index_empty must carry a hint");
    assert!(
        hint.contains("reindex"),
        "index_empty hint must point at reindex/enable-for-this-scope; got: {hint:?}"
    );
}

/// #285 review note: the `no_match` reason (populated scope, zero matches) is
/// NOT reachable through this handler today. The MCP path forces
/// `strict: false` / `min_score: None`, so the KNN's nearest-neighbour rows
/// are never filtered below a threshold — a non-empty scope therefore ALWAYS
/// yields ≥1 match. `matches.is_empty()` on the MCP path thus implies the
/// scope had zero searchable rows (`index_empty`). The `no_match` branch is
/// retained for correctness (it represents a legitimate Output state a future
/// score-floored path could produce) and its WIRE SHAPE is pinned in
/// `mcp_search_skills_json_shape::empty_matches_wire_shape_no_match`. This
/// test documents the invariant that closes the gap the review flagged:
/// through the real handler, an empty result is always `index_empty`.
#[test]
fn non_strict_handler_never_reports_no_match_for_populated_scope() {
    // A populated scope, queried in-scope: must return matches (never empty),
    // so the `no_match` branch is not taken.
    let body = "---\nname: present\ndescription: Present in this scope.\n---\nbody\n";
    let (_tmp, paths) = stage_workspace(&[("present", body)], &[]);
    let state = build_state(&paths);

    let out = invoke(state, make_input("present", 150)).expect("search ok");
    assert!(
        !out.matches.is_empty(),
        "a populated scope on the non-strict MCP path must never return zero matches"
    );
    assert!(
        out.no_results_reason.is_none(),
        "a non-empty result must not carry a no_results_reason"
    );
    assert!(
        out.corpus_size >= out.matches.len() as u64,
        "scope-effective corpus_size ({}) must be >= the returned match count ({})",
        out.corpus_size,
        out.matches.len()
    );
}

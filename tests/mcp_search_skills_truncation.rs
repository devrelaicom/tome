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

mod common;

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
use tome::mcp::tools::search_skills::{self, Input, MAX_DESCRIPTION_MAX_CHARS};
use tome::plugin::PluginId;
use tome::plugin::identity::EntryKind;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, WorkspaceName};

use common::{
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
        reranker_entry: &STUB_RERANKER_ENTRY,
        prompt_registry: Arc::new(PromptRegistry::default()),
        host_harness: None,
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
        top_k: 10,
        catalog: None,
        plugin: None,
        description_max_chars,
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
        input.description_max_chars, 150,
        "default description_max_chars must be 150 per FR-092"
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

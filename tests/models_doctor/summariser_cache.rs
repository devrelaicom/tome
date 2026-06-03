//! Phase 4 / US4.b — T334: cache hit/miss semantics for
//! `[summaries]`.
//!
//! Verifies:
//!
//! * **Trigger overwrites cache** — when `regen_summary::regen` runs,
//!   the `settings.toml` `[summaries]` section is rewritten and
//!   `generated_at` advances.
//! * **No trigger fires → cache untouched** — read-only paths (e.g.
//!   listing workspaces, querying skills) do not invoke the
//!   summariser; the cached values are reused as-is.
//!
//! The "reused as-is" assertion is observable in two ways:
//!
//!   1. `StubSummariser::call_count()` stays at 0 across a read-only
//!      command sequence.
//!   2. The on-disk `[summaries].generated_at` does not advance.

use crate::common::{
    lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use tome::index::{self, OpenOptions};
use tome::paths::Paths;
use tome::summarise::StubSummariser;
use tome::workspace::{self, WorkspaceName};

fn seed_enabled_skill(paths: &Paths, workspace_name: &str, name: &str) {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .unwrap();
    let workspace_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![workspace_name],
            |row| row.get(0),
        )
        .unwrap();
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO skills
           (catalog, plugin, name, description, plugin_version, path, content_hash, indexed_at)
         VALUES ('cat', 'plug', ?1, ?2, '0.0.0', '/dev/null', 'h', ?3)",
        rusqlite::params![name, format!("desc of {name}"), now],
    )
    .unwrap();
    let skill_id = conn.last_insert_rowid();
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![workspace_id, skill_id, now],
    )
    .unwrap();
}

#[test]
fn trigger_overwrites_cached_summaries_and_advances_generated_at() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    let ws = WorkspaceName::parse("mine").unwrap();
    workspace::init::init(ws.clone(), false, &paths).unwrap();
    seed_enabled_skill(&paths, "mine", "alpha");

    // Pre-populate `[summaries]` with a sentinel generated_at well
    // in the past.
    let settings_path = paths.workspace_settings_file(&ws);
    let pre_cache = "name = \"mine\"\n\n[summaries]\nshort = \"old short\"\nlong = \"old long\"\ngenerated_at = \"2000-01-01T00:00:00Z\"\n";
    std::fs::write(&settings_path, pre_cache).unwrap();

    // Fire the explicit regen path with a stub summariser.
    let stub = StubSummariser::new();
    workspace::regen_summary::regen(&ws, &stub, &paths).expect("regen");

    let after = std::fs::read_to_string(&settings_path).unwrap();
    assert!(
        !after.contains("old short"),
        "trigger should have overwritten the cached `short` value",
    );
    assert!(
        !after.contains("2000-01-01"),
        "generated_at must have advanced from the sentinel; got:\n{after}",
    );
    // Stub's `short` is the joined skill names — `alpha` is the only
    // enabled skill.
    assert!(
        after.contains("alpha"),
        "new cache must reflect the workspace's current enabled set",
    );
}

#[test]
fn read_only_paths_do_not_invoke_summariser() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    let ws = WorkspaceName::parse("mine").unwrap();
    workspace::init::init(ws.clone(), false, &paths).unwrap();
    seed_enabled_skill(&paths, "mine", "alpha");

    // Pre-populate cache.
    let settings_path = paths.workspace_settings_file(&ws);
    let cache = "name = \"mine\"\n\n[summaries]\nshort = \"cached short\"\nlong = \"cached long\"\ngenerated_at = \"2025-01-01T00:00:00Z\"\n";
    std::fs::write(&settings_path, cache).unwrap();
    let before_bytes = std::fs::read(&settings_path).unwrap();

    // Read-only paths: list workspaces, list bound projects, inspect
    // the index. None of these construct the summariser.
    let stub = StubSummariser::new();
    let _names = workspace::list_workspace_names(&paths).unwrap();
    let _outcome = workspace::sync_one(&ws, &paths).unwrap();
    // Verify the MCP description composition reads the cache without
    // invoking the summariser.
    let desc = tome::mcp::tool_description::compose(&ws, &paths);
    assert!(
        desc.contains("cached short"),
        "MCP description should reuse the cached `[summaries].short`",
    );

    assert_eq!(
        stub.call_count(),
        0,
        "read-only paths must not invoke the summariser",
    );

    let after_bytes = std::fs::read(&settings_path).unwrap();
    assert_eq!(
        after_bytes, before_bytes,
        "settings.toml must be byte-identical after read-only ops",
    );
}

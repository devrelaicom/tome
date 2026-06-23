//! Phase 4 / F11c-2 → US4.b — T098l + T098r: FR-385 workspace_skills
//! forward-progress at the summariser boundary.
//!
//! Invariant: the `workspace_skills` row INSERT / DELETE that records a
//! per-workspace enable / disable commits in its own transaction BEFORE
//! the summariser is invoked. A summariser failure must NOT roll back
//! the enrolment mutation, and the prior cached summary must survive.
//!
//! ## US4.b unhide
//!
//! The substantive forward-progress coverage lives in
//! [`tests/summariser_forward_progress.rs`]. This file's two tests
//! light up the F11c-2 placeholder against the US4.b wiring:
//!
//!   1. `workspace_skills_commits_before_summariser_failure` —
//!      seeds a workspace_skills row, invokes the trigger with a
//!      failing summariser, asserts the row survives.
//!   2. `cached_summary_survives_summariser_failure` — pre-populates
//!      `[summaries]` and asserts the bytes are unchanged after a
//!      failing trigger.
//!
//! The substantive coverage lives in
//! `tests/summariser_forward_progress.rs`; this file is the named
//! marker for the F11c-2 → US4.b unhide.

use crate::common::{
    lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use tome::error::{ShortOrLong, SummariserFailureKind, TomeError};
use tome::index::{self, OpenOptions};
use tome::paths::Paths;
use tome::summarise::{
    PluginSummariesInput, Summariser, SummariserOutput, regenerate_for_trigger_with_summariser,
};
use tome::workspace::{self, WorkspaceName};

struct FailingSummariser;

impl Summariser for FailingSummariser {
    fn summarise(
        &self,
        _input: &PluginSummariesInput,
        _long_max_chars: usize,
    ) -> Result<SummariserOutput, TomeError> {
        Err(TomeError::SummariserFailure {
            kind: SummariserFailureKind::OutputEmpty {
                which: ShortOrLong::Short,
            },
        })
    }
}

fn seed_enabled_skill(paths: &Paths, workspace_name: &str) {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
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
         VALUES ('cat', 'plug', 's', 'd', '0.0.0', '/dev/null', 'h', ?1)",
        rusqlite::params![now],
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
fn workspace_skills_commits_before_summariser_failure() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    let ws = WorkspaceName::parse("mine").unwrap();
    workspace::init::init(ws.clone(), false, &paths).unwrap();
    seed_enabled_skill(&paths, "mine");

    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .unwrap();
    let before: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_skills AS ws
             JOIN workspaces AS w ON w.id = ws.workspace_id
             WHERE w.name = 'mine'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(before, 1);
    drop(conn);

    let failing = FailingSummariser;
    let err = regenerate_for_trigger_with_summariser(
        &ws,
        &failing,
        &paths,
        tome::summarise::LONG_MAX_CHARS,
    )
    .expect_err("trigger should fail");
    assert!(matches!(err, TomeError::SummariserFailure { .. }));

    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .unwrap();
    let after: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_skills AS ws
             JOIN workspaces AS w ON w.id = ws.workspace_id
             WHERE w.name = 'mine'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        after, before,
        "FR-385: workspace_skills row must survive a failing summariser",
    );
}

#[test]
fn cached_summary_survives_summariser_failure() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    let ws = WorkspaceName::parse("mine").unwrap();
    workspace::init::init(ws.clone(), false, &paths).unwrap();
    seed_enabled_skill(&paths, "mine");

    let settings_path = paths.workspace_settings_file(&ws);
    let prior = "name = \"mine\"\n\n[summaries]\nshort = \"keep me\"\nlong = \"keep me long\"\ngenerated_at = \"2025-06-15T00:00:00Z\"\n";
    std::fs::write(&settings_path, prior).unwrap();
    let prior_bytes = std::fs::read(&settings_path).unwrap();

    let failing = FailingSummariser;
    let _ = regenerate_for_trigger_with_summariser(
        &ws,
        &failing,
        &paths,
        tome::summarise::LONG_MAX_CHARS,
    )
    .expect_err("trigger should fail");

    let after_bytes = std::fs::read(&settings_path).unwrap();
    assert_eq!(
        after_bytes, prior_bytes,
        "FR-385: cached `[summaries]` must survive a failing summariser",
    );
}

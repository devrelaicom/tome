//! Phase 4 / US4.b — T333: FR-385 forward-progress at the summariser
//! boundary.
//!
//! Invariant: the `workspace_skills` mutation commits in its OWN
//! transaction BEFORE the summariser is invoked. A summariser failure
//! must NOT roll back the enrolment mutation, and the prior cached
//! `[summaries]` section in `settings.toml` must survive intact.
//!
//! Tests drive the contract through the library API:
//!
//! 1. Pre-populate a workspace with `[summaries]` (the "prior cache").
//! 2. Bind a [`FailingSummariser`] via [`SummariserOverrideGuard`].
//! 3. Call the trigger.
//! 4. Assert: trigger returns `SummariserFailure { OutputEmpty }`.
//! 5. Assert: the prior `[summaries]` bytes survive.
//!
//! The trigger is invoked via
//! [`tome::summarise::regenerate_for_trigger_with_summariser`] (the DI
//! seam US4.b ships). The wiring at `commands::plugin::enable::run`
//! already calls this after `lifecycle::enable` returns Ok — so the
//! workspace_skills row exists at the time the summariser is called.

use crate::common::{
    lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use tome::error::{SummariserFailureKind, TomeError};
use tome::index::{self, OpenOptions};
use tome::paths::Paths;
use tome::summarise::{
    PluginSummariesInput, Summariser, SummariserOutput, regenerate_for_trigger_with_summariser,
};
use tome::workspace::{self, WorkspaceName};

/// Always returns `SummariserFailure { OutputEmpty }`. Mirrors the
/// `StubEmbedder::with_force_fail_after(0)` pattern.
struct FailingSummariser;

impl Summariser for FailingSummariser {
    fn summarise(
        &self,
        _input: &PluginSummariesInput,
        _long_max_chars: usize,
    ) -> Result<SummariserOutput, TomeError> {
        Err(TomeError::SummariserFailure {
            kind: SummariserFailureKind::OutputEmpty {
                which: tome::error::ShortOrLong::Short,
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
        },
    )
    .expect("open central DB");
    let workspace_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![workspace_name],
            |row| row.get(0),
        )
        .expect("lookup workspace_id");
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO skills
           (catalog, plugin, name, description, plugin_version, path, content_hash, indexed_at)
         VALUES ('cat', 'plug', 's1', 'd', '0.0.0', '/dev/null', 'h', ?1)",
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
fn summariser_failure_preserves_prior_cached_summary() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    let ws = WorkspaceName::parse("mine").unwrap();
    workspace::init::init(ws.clone(), false, &paths).unwrap();
    seed_enabled_skill(&paths, "mine");

    // Pre-populate `[summaries]` with the "prior cache" we expect to
    // survive a failing trigger.
    let settings_path = paths.workspace_settings_file(&ws);
    let prior = "name = \"mine\"\n\n[summaries]\nshort = \"the prior short summary\"\nlong = \"the prior long summary\"\ngenerated_at = \"2025-01-01T00:00:00Z\"\n";
    std::fs::write(&settings_path, prior).unwrap();
    let prior_bytes = std::fs::read(&settings_path).unwrap();

    // Trigger with a failing summariser.
    let failing = FailingSummariser;
    let err = regenerate_for_trigger_with_summariser(
        &ws,
        &failing,
        &paths,
        tome::summarise::LONG_MAX_CHARS,
    )
    .expect_err("must fail");
    match err {
        TomeError::SummariserFailure { kind } => {
            assert!(
                matches!(kind, SummariserFailureKind::OutputEmpty { .. }),
                "expected OutputEmpty, got {kind:?}",
            );
        }
        other => panic!("expected SummariserFailure, got {other:?}"),
    }

    // Prior cache must be byte-identical — the failing trigger must
    // not write a half-state. (regen_summary writes settings.toml
    // ONLY after summarise() returns Ok, so this is the cleanest
    // observable of FR-385.)
    let after_bytes = std::fs::read(&settings_path).unwrap();
    assert_eq!(
        after_bytes, prior_bytes,
        "settings.toml must be untouched when summariser fails",
    );
}

#[test]
fn summariser_failure_keeps_workspace_skills_committed() {
    // The library API path is symmetric: workspace_skills was
    // committed by a prior `enable`/seed step; the trigger doesn't
    // own the DB transaction. A failed trigger must therefore leave
    // the row in place.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    let ws = WorkspaceName::parse("mine").unwrap();
    workspace::init::init(ws.clone(), false, &paths).unwrap();
    seed_enabled_skill(&paths, "mine");

    // Confirm the row exists before the trigger.
    let before = enabled_skill_count(&paths, "mine");
    assert_eq!(before, 1);

    let failing = FailingSummariser;
    let _ = regenerate_for_trigger_with_summariser(
        &ws,
        &failing,
        &paths,
        tome::summarise::LONG_MAX_CHARS,
    )
    .expect_err("must fail with OutputEmpty");

    // Row still in place — FR-385 forward-progress.
    let after = enabled_skill_count(&paths, "mine");
    assert_eq!(
        after, before,
        "workspace_skills row must survive a failing summariser",
    );
}

fn enabled_skill_count(paths: &Paths, workspace_name: &str) -> i64 {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM workspace_skills AS ws
         JOIN workspaces AS w ON w.id = ws.workspace_id
         WHERE w.name = ?1",
        rusqlite::params![workspace_name],
        |row| row.get(0),
    )
    .unwrap()
}

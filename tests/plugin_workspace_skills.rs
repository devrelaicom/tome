//! Phase 4 / F11c-2 — per-workspace skill enrolment isolation.
//!
//! Targets FR-380 / FR-381 / FR-383: enable/disable of a plugin in one
//! workspace MUST NOT affect another workspace's enrolment of the same
//! plugin, and disabling in any workspace MUST NOT delete the underlying
//! `skills` rows (they are content-addressed and shared — retention is
//! mandatory for cheap cross-workspace re-enable).
//!
//! These tests drive the library API directly with `StubEmbedder` so
//! they run fast and deterministically. The `seed_workspace` helper
//! stands in for what `tome workspace add` will own in production (US2).

mod common;

use std::sync::Arc;
use std::sync::Barrier;

use common::{
    fabricate_models, lifecycle_paths, seed_workspace, stage_sample_catalog_in_db,
    stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::index::{self, OpenOptions};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{Scope, WorkspaceName};

/// Convenience: count `workspace_skills` rows for the given workspace name.
fn workspace_skill_count(paths: &tome::paths::Paths, workspace: &str) -> i64 {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open index");
    conn.query_row(
        "SELECT COUNT(*) FROM workspace_skills AS ws
         JOIN workspaces AS w ON w.id = ws.workspace_id
         WHERE w.name = ?1",
        rusqlite::params![workspace],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

/// Count of total `skills` rows for a `(catalog, plugin)`. This is the
/// content-addressed surface; FR-383 requires it survives every disable.
fn skills_row_count(paths: &tome::paths::Paths, catalog: &str, plugin: &str) -> i64 {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open index");
    conn.query_row(
        "SELECT COUNT(*) FROM skills WHERE catalog = ?1 AND plugin = ?2",
        rusqlite::params![catalog, plugin],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

fn build_deps<'a>(
    paths: &'a tome::paths::Paths,
    scope: &'a Scope,
    config: &'a tome::config::Config,
    embedder: &'a StubEmbedder,
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

// ---------------------------------------------------------------------------
// FR-380: workspace isolation on enable.
// ---------------------------------------------------------------------------

#[test]
fn enable_in_workspace_a_does_not_affect_workspace_b() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // FF1: enrol the catalog in the DB (+ stage the clone) for BOTH
    // workspaces, the on-disk shape `tome catalog add` produces. `second`
    // must exist before its enrolment, so it is seeded first.
    stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");
    seed_workspace(&paths, "second");
    stage_sample_catalog_in_db(&paths, "second", "sample-plugin-catalog");
    let config = tome::config::Config::default();

    let embedder = StubEmbedder::new();
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();

    // Enable only in `global`.
    let global_scope = Scope(WorkspaceName::global());
    let deps_global = build_deps(&paths, &global_scope, &config, &embedder);
    lifecycle::enable(&id, &deps_global).expect("enable in global");

    assert_eq!(
        workspace_skill_count(&paths, "global"),
        4,
        "global must have 4 enrolment rows",
    );
    assert_eq!(
        workspace_skill_count(&paths, "second"),
        0,
        "second must have 0 enrolment rows — enable in `global` is workspace-scoped (FR-380)",
    );
}

// ---------------------------------------------------------------------------
// FR-381 + FR-383: disable in one workspace leaves the other untouched
// AND retains the underlying `skills` rows.
// ---------------------------------------------------------------------------

#[test]
fn disable_in_a_does_not_delete_shared_skills_row() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // FF1: enrol the catalog in the DB (+ stage the clone) for BOTH
    // workspaces, the on-disk shape `tome catalog add` produces. `second`
    // must exist before its enrolment, so it is seeded first.
    stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");
    seed_workspace(&paths, "second");
    stage_sample_catalog_in_db(&paths, "second", "sample-plugin-catalog");
    let config = tome::config::Config::default();
    let embedder = StubEmbedder::new();
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();

    // Enable in BOTH workspaces.
    let global_scope = Scope(WorkspaceName::global());
    let second_scope = Scope(WorkspaceName::parse("second").expect("workspace name"));

    lifecycle::enable(&id, &build_deps(&paths, &global_scope, &config, &embedder))
        .expect("enable global");
    lifecycle::enable(&id, &build_deps(&paths, &second_scope, &config, &embedder))
        .expect("enable second");

    assert_eq!(workspace_skill_count(&paths, "global"), 4);
    assert_eq!(workspace_skill_count(&paths, "second"), 4);
    assert_eq!(
        skills_row_count(&paths, "sample-plugin-catalog", "plugin-alpha"),
        4,
        "shared skills rows must equal the on-disk skill count",
    );

    // Disable in `global` only.
    lifecycle::disable(
        &id,
        &paths,
        &global_scope,
        stub_embedder_seed(),
        stub_reranker_seed(),
        stub_summariser_seed(),
    )
    .expect("disable global");

    // (global) enrolment is gone.
    assert_eq!(
        workspace_skill_count(&paths, "global"),
        0,
        "global enrolment must be cleared after disable",
    );
    // (second) enrolment is intact.
    assert_eq!(
        workspace_skill_count(&paths, "second"),
        4,
        "second's enrolment must NOT be affected by disable-in-global (FR-381)",
    );
    // Shared `skills` rows still present (FR-383 retention rule).
    assert_eq!(
        skills_row_count(&paths, "sample-plugin-catalog", "plugin-alpha"),
        4,
        "FR-383: `skills` rows MUST survive a per-workspace disable",
    );
}

// ---------------------------------------------------------------------------
// Concurrent enables in two workspaces — both must commit independently.
// ---------------------------------------------------------------------------

#[test]
fn concurrent_enables_of_same_plugin_in_two_workspaces_both_succeed() {
    use tome::error::TomeError;

    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // FF1: enrol the catalog in the DB (+ stage the clone) for BOTH
    // workspaces, the on-disk shape `tome catalog add` produces. `second`
    // must exist before its enrolment, so it is seeded first.
    stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");
    seed_workspace(&paths, "second");
    stage_sample_catalog_in_db(&paths, "second", "sample-plugin-catalog");
    let config = tome::config::Config::default();

    // One shared StubEmbedder so both threads see deterministic vectors.
    let embedder = Arc::new(StubEmbedder::new());
    let id_str = "sample-plugin-catalog/plugin-alpha";

    let barrier = Arc::new(Barrier::new(2));

    // Spawn one thread per workspace. Both call `lifecycle::enable`
    // which acquires the same advisory lock; only one runs at a time.
    // The looser holder gets `IndexBusy` and retries up to 3 times —
    // mirroring F11c-1's catalog/cross-workspace pattern.
    let mut handles = Vec::with_capacity(2);
    for workspace in ["global", "second"] {
        let b = Arc::clone(&barrier);
        let e = Arc::clone(&embedder);
        let paths_clone = paths.clone();
        let config_clone = config.clone();
        let id_owned: PluginId = id_str.parse().unwrap();
        let workspace_owned = workspace.to_owned();
        handles.push(std::thread::spawn(move || {
            let scope = Scope(WorkspaceName::parse(&workspace_owned).expect("workspace name"));
            b.wait();
            // Retry-on-IndexBusy loop. Three attempts is enough — the
            // lock window is short (one transaction, no model load).
            let mut last_err = None;
            for _ in 0..3 {
                let deps = LifecycleDeps {
                    paths: &paths_clone,
                    scope: &scope,
                    config: &config_clone,
                    embedder: e.as_ref(),
                    embedder_seed: stub_embedder_seed(),
                    reranker_seed: stub_reranker_seed(),
                    summariser_seed: stub_summariser_seed(),
                    allow_model_download: false,
                };
                match lifecycle::enable(&id_owned, &deps) {
                    Ok(o) => return Ok(o),
                    Err(TomeError::IndexBusy) => {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(other) => {
                        last_err = Some(other);
                        break;
                    }
                }
            }
            Err(last_err.unwrap_or(TomeError::IndexIntegrityCheckFailure(
                "exhausted retries".into(),
            )))
        }));
    }

    let outcomes: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().expect("thread join"))
        .collect();

    for outcome in &outcomes {
        assert!(
            outcome.is_ok(),
            "both threads must succeed (eventually), got {outcome:?}",
        );
    }

    // Both workspaces hold their own 4 enrolment rows.
    assert_eq!(workspace_skill_count(&paths, "global"), 4);
    assert_eq!(workspace_skill_count(&paths, "second"), 4);
    // Shared `skills` rows still equals 4 — content-addressed surface
    // is not duplicated per-workspace.
    assert_eq!(
        skills_row_count(&paths, "sample-plugin-catalog", "plugin-alpha"),
        4,
    );
}

// ---------------------------------------------------------------------------
// FR-383: total disable across all workspaces still leaves `skills` rows.
// ---------------------------------------------------------------------------

#[test]
fn skills_rows_persist_after_disable_in_all_workspaces() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // FF1: enrol the catalog in the DB (+ stage the clone) for BOTH
    // workspaces, the on-disk shape `tome catalog add` produces. `second`
    // must exist before its enrolment, so it is seeded first.
    stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");
    seed_workspace(&paths, "second");
    stage_sample_catalog_in_db(&paths, "second", "sample-plugin-catalog");
    let config = tome::config::Config::default();
    let embedder = StubEmbedder::new();
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();

    let global_scope = Scope(WorkspaceName::global());
    let second_scope = Scope(WorkspaceName::parse("second").expect("workspace name"));

    lifecycle::enable(&id, &build_deps(&paths, &global_scope, &config, &embedder))
        .expect("enable global");
    lifecycle::enable(&id, &build_deps(&paths, &second_scope, &config, &embedder))
        .expect("enable second");

    // Disable in BOTH — exhaustive cleanup of enrolments.
    lifecycle::disable(
        &id,
        &paths,
        &global_scope,
        stub_embedder_seed(),
        stub_reranker_seed(),
        stub_summariser_seed(),
    )
    .expect("disable global");
    lifecycle::disable(
        &id,
        &paths,
        &second_scope,
        stub_embedder_seed(),
        stub_reranker_seed(),
        stub_summariser_seed(),
    )
    .expect("disable second");

    // No `workspace_skills` rows remain for this plugin in either workspace.
    assert_eq!(workspace_skill_count(&paths, "global"), 0);
    assert_eq!(workspace_skill_count(&paths, "second"), 0);

    // FR-383: `skills` rows MUST survive. v1 has no GC pass; the
    // content-addressed surface is permanently retained until a
    // future explicit `tome prune` ships (Phase 5+).
    assert_eq!(
        skills_row_count(&paths, "sample-plugin-catalog", "plugin-alpha"),
        4,
        "FR-383: `skills` rows must outlive every per-workspace disable",
    );
}

//! Concurrent-bind coverage for `bind_project` (Phase 4 / US1.d-1 / T163).
//!
//! `tests/workspace_use_binding.rs::concurrent_bind_two_threads_last_wins`
//! already establishes that the central-DB advisory lock serialises two
//! concurrent writers. This file extends the coverage to:
//!
//! 1. Three threads racing different workspaces — exactly one survives in
//!    `workspace_projects`, and the marker on disk names the winner.
//! 2. Two threads racing the SAME workspace — both succeed (idempotent),
//!    single row, marker reflects the workspace.
//!
//! Note: a "third thread holds the lock for N ms via injected sleep"
//! style test isn't viable here without modifying production code. The
//! row-count + marker-agreement assertions already prove serialisation
//! transitively — any unserialised execution would either deadlock or
//! corrupt the UNIQUE constraint on `project_path`.

use std::sync::{Arc, Barrier};

use crate::common::{HarnessModulesGuard, lifecycle_paths, seed_workspace};
use tempfile::TempDir;
use tome::workspace::WorkspaceName;
use tome::workspace::binding::{self, BindDeps};

/// Open the index for read-only assertions.
fn open_db(paths: &tome::paths::Paths) -> rusqlite::Connection {
    tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: crate::common::stub_embedder_seed(),
            reranker: crate::common::stub_reranker_seed(),
            summariser: crate::common::stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open index")
}

// ---------------------------------------------------------------------------
// 1. Three threads, three different workspaces. Exactly one survives.
// ---------------------------------------------------------------------------

#[test]
fn concurrent_bind_three_threads_different_workspaces_one_wins() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![]);

    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(&tmp.path().join(".tome"));
    std::fs::create_dir_all(&paths.root).unwrap();
    for ws in ["ws-a", "ws-b", "ws-c"] {
        seed_workspace(&paths, ws);
    }

    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let home = tmp.path().join("fake-home");
    std::fs::create_dir_all(&home).unwrap();

    let barrier = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();
    for ws_name in ["ws-a", "ws-b", "ws-c"] {
        let project_t = project.clone();
        let paths_t = paths.clone();
        let home_t = home.clone();
        let barrier_t = barrier.clone();
        let h = std::thread::spawn(move || {
            let deps = BindDeps {
                paths: &paths_t,
                home_root: &home_t,
            };
            let name = WorkspaceName::parse(ws_name).unwrap();
            barrier_t.wait();
            binding::bind_project(&project_t, name, false, &deps)
        });
        handles.push(h);
    }

    let mut results = Vec::new();
    for h in handles {
        results.push(h.join().expect("thread panicked"));
    }

    // At least one must succeed; the others may succeed (serialised
    // through the lock) or fail with `IndexBusy` (50). Both shapes are
    // tolerated — what matters is the post-condition.
    let successes = results.iter().filter(|r| r.is_ok()).count();
    assert!(successes >= 1, "at least one bind must succeed");
    for r in &results {
        if let Err(e) = r {
            assert_eq!(
                e.exit_code(),
                50,
                "non-success must be IndexBusy; got {e:?}",
            );
        }
    }

    let canonical = project.canonicalize().unwrap();

    // Exactly one workspace_projects row.
    let conn = open_db(&paths);
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_projects WHERE project_path = ?1",
            rusqlite::params![canonical.to_string_lossy().into_owned()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "exactly one row must survive a concurrent race");

    // The marker names the SAME workspace as the surviving row.
    let surviving: String = conn
        .query_row(
            "SELECT w.name
             FROM workspace_projects AS wp
             JOIN workspaces AS w ON w.id = wp.workspace_id
             WHERE wp.project_path = ?1",
            rusqlite::params![canonical.to_string_lossy().into_owned()],
            |row| row.get(0),
        )
        .expect("read surviving workspace name");
    let cfg = std::fs::read_to_string(canonical.join(".tome").join("config.toml"))
        .expect("read marker config");
    let line = format!("workspace = \"{surviving}\"");
    assert!(
        cfg.contains(&line),
        "marker must name surviving workspace `{surviving}`; got: {cfg}",
    );

    drop(tmp);
}

// ---------------------------------------------------------------------------
// 2. Two threads, SAME workspace. Both succeed (idempotent).
// ---------------------------------------------------------------------------

#[test]
fn concurrent_bind_same_workspace_idempotent() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![]);

    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(&tmp.path().join(".tome"));
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "ws-a");

    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let home = tmp.path().join("fake-home");
    std::fs::create_dir_all(&home).unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let project_t = project.clone();
        let paths_t = paths.clone();
        let home_t = home.clone();
        let barrier_t = barrier.clone();
        let h = std::thread::spawn(move || {
            let deps = BindDeps {
                paths: &paths_t,
                home_root: &home_t,
            };
            let name = WorkspaceName::parse("ws-a").unwrap();
            barrier_t.wait();
            binding::bind_project(&project_t, name, false, &deps)
        });
        handles.push(h);
    }

    let mut results = Vec::new();
    for h in handles {
        results.push(h.join().expect("thread panicked"));
    }

    // Both should be `Ok` — same workspace, advisory lock serialises
    // the writers, second one no-ops on the workspace_id comparison.
    // `IndexBusy` is tolerated as well (the second writer may bail
    // without taking the lock if the timeout elapses, depending on
    // platform / load).
    for r in &results {
        if let Err(e) = r {
            assert_eq!(
                e.exit_code(),
                50,
                "non-success must be IndexBusy; got {e:?}",
            );
        }
    }
    let successes = results.iter().filter(|r| r.is_ok()).count();
    assert!(
        successes >= 1,
        "at least one same-workspace bind must succeed",
    );

    let canonical = project.canonicalize().unwrap();

    // Exactly one row.
    let conn = open_db(&paths);
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_projects WHERE project_path = ?1",
            rusqlite::params![canonical.to_string_lossy().into_owned()],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "same-workspace race must yield exactly one row");

    // Marker reflects ws-a.
    let cfg = std::fs::read_to_string(canonical.join(".tome").join("config.toml"))
        .expect("read marker config");
    assert!(
        cfg.contains("workspace = \"ws-a\""),
        "marker must name ws-a; got: {cfg}",
    );

    drop(tmp);
}

//! Phase 4 / US2.b — `tome workspace remove <name> [--force]` library-API tests.
//!
//! Exercises [`tome::workspace::remove::remove`] directly. The CLI binary
//! surface is a thin emit wrapper around the library API; CLI exit-code
//! coverage is enforced by `tests/exit_codes.rs`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

use crate::common::{
    lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use time::OffsetDateTime;
use tome::error::TomeError;
use tome::index::{self, OpenOptions};
use tome::workspace::{self, WorkspaceName};

fn parse(name: &str) -> WorkspaceName {
    WorkspaceName::parse(name).expect("valid workspace name")
}

fn open_central(paths: &tome::paths::Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open central DB")
}

fn workspace_exists(paths: &tome::paths::Paths, name: &str) -> bool {
    let conn = open_central(paths);
    conn.query_row(
        "SELECT 1 FROM workspaces WHERE name = ?1",
        rusqlite::params![name],
        |_| Ok(()),
    )
    .is_ok()
}

/// Seed a `workspace_projects` row for `(workspace_name, project_path)`
/// AND write the project's marker `.tome/config.toml` so step 1 sees a
/// healthy binding to tear down.
fn seed_bound_project(paths: &tome::paths::Paths, workspace_name: &str, project_root: &Path) {
    std::fs::create_dir_all(project_root.join(".tome")).expect("create .tome");
    std::fs::write(
        project_root.join(".tome").join("config.toml"),
        format!("workspace = \"{workspace_name}\"\n"),
    )
    .expect("write project config.toml");
    let conn = open_central(paths);
    let workspace_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![workspace_name],
            |row| row.get(0),
        )
        .expect("lookup workspace_id");
    let now = OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO workspace_projects (project_path, workspace_id, bound_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![
            project_root.to_string_lossy().to_string(),
            workspace_id,
            now
        ],
    )
    .expect("seed workspace_projects");
}

fn count_workspace_projects_for(paths: &tome::paths::Paths, workspace_name: &str) -> u32 {
    let conn = open_central(paths);
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_projects wp
             JOIN workspaces w ON w.id = wp.workspace_id
             WHERE w.name = ?1",
            rusqlite::params![workspace_name],
            |row| row.get(0),
        )
        .unwrap_or(0);
    u32::try_from(n.max(0)).unwrap_or(0)
}

#[test]
fn remove_global_refused_exits_15() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // Bootstrap the DB so `global` exists.
    workspace::init::init(parse("seed"), false, &paths).expect("init to bootstrap");

    let err = workspace::remove::remove(parse("global"), false, &paths, tmp.path()).unwrap_err();
    assert!(
        matches!(err, TomeError::WorkspaceNameInvalid { .. }),
        "expected WorkspaceNameInvalid, got {err:?}",
    );
    assert_eq!(err.exit_code(), 15);
    // Display mentions "reserved".
    let msg = err.to_string();
    assert!(
        msg.contains("reserved"),
        "Display should mention `reserved`; got {msg}",
    );

    // Cross-check: `global` still exists.
    assert!(workspace_exists(&paths, "global"));
}

#[test]
fn remove_nonexistent_workspace_exits_13() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // Bootstrap so the DB exists; do NOT init "missing-ws".
    workspace::init::init(parse("seed"), false, &paths).expect("init to bootstrap");

    let err =
        workspace::remove::remove(parse("missing-ws"), false, &paths, tmp.path()).unwrap_err();
    assert!(
        matches!(err, TomeError::WorkspaceNotFound { .. }),
        "expected WorkspaceNotFound, got {err:?}",
    );
    assert_eq!(err.exit_code(), 13);
}

#[test]
fn remove_no_bound_projects_no_force_happy_path() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");
    let mine_dir = paths.workspace_dir(&parse("mine"));
    assert!(mine_dir.is_dir(), "init should have landed the directory");

    let outcome =
        workspace::remove::remove(parse("mine"), false, &paths, tmp.path()).expect("remove");
    assert_eq!(outcome.removed.as_str(), "mine");
    assert_eq!(outcome.bound_projects_torn_down, 0);
    assert!(outcome.catalog_caches_cleaned.is_empty());
    assert!(
        outcome.orphaned_paths.is_empty(),
        "happy path should leave no orphans, got {:?}",
        outcome.orphaned_paths
    );

    // DB: `mine` is gone; `global` survives.
    assert!(!workspace_exists(&paths, "mine"));
    assert!(workspace_exists(&paths, "global"));

    // Directory: central dir gone.
    assert!(
        !mine_dir.exists(),
        "{} should have been removed",
        mine_dir.display(),
    );
}

#[test]
fn remove_with_bound_projects_without_force_exits_16() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");

    let project_a = tmp.path().join("project-a");
    let project_b = tmp.path().join("project-b");
    seed_bound_project(&paths, "mine", &project_a);
    seed_bound_project(&paths, "mine", &project_b);

    let err = workspace::remove::remove(parse("mine"), false, &paths, tmp.path()).unwrap_err();
    assert_eq!(err.exit_code(), 16);
    let TomeError::WorkspaceHasBoundProjects {
        name,
        count,
        projects,
    } = err
    else {
        panic!("expected WorkspaceHasBoundProjects, got different variant");
    };
    assert_eq!(name, "mine");
    assert_eq!(count, 2);
    // Path ordering follows `ORDER BY project_path` — lexicographic.
    let mut sorted = projects.clone();
    sorted.sort();
    assert_eq!(projects, sorted);
    assert!(projects[0].contains("project-a"));
    assert!(projects[1].contains("project-b"));

    // No state change: `mine` still in DB, bindings still present, project
    // markers still on disk.
    assert!(workspace_exists(&paths, "mine"));
    assert_eq!(count_workspace_projects_for(&paths, "mine"), 2);
    assert!(project_a.join(".tome/config.toml").is_file());
    assert!(project_b.join(".tome/config.toml").is_file());
}

#[test]
fn remove_with_bound_projects_force_cascades() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");

    let project_a = tmp.path().join("project-a");
    let project_b = tmp.path().join("project-b");
    seed_bound_project(&paths, "mine", &project_a);
    seed_bound_project(&paths, "mine", &project_b);

    let outcome =
        workspace::remove::remove(parse("mine"), true, &paths, tmp.path()).expect("remove");
    assert_eq!(outcome.bound_projects_torn_down, 2);
    assert!(outcome.catalog_caches_cleaned.is_empty());
    assert!(
        outcome.orphaned_paths.is_empty(),
        "happy cascade should leave no orphans, got {:?}",
        outcome.orphaned_paths
    );

    // DB rows: workspaces / workspace_projects / workspace_catalogs /
    // workspace_skills all gone for `mine`.
    let conn = open_central(&paths);
    let workspaces_remaining: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspaces WHERE name = ?1",
            rusqlite::params!["mine"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(workspaces_remaining, 0);
    assert_eq!(count_workspace_projects_for(&paths, "mine"), 0);

    // On-disk: each project's `.tome/` was removed by Step 2.
    assert!(
        !project_a.join(".tome").exists(),
        "Step 2 should remove project-a/.tome",
    );
    assert!(
        !project_b.join(".tome").exists(),
        "Step 2 should remove project-b/.tome",
    );
    // Project directories themselves remain — Step 2 only removes the
    // marker dir.
    assert!(project_a.is_dir());
    assert!(project_b.is_dir());

    // Central workspace directory: gone.
    assert!(
        !paths.workspace_dir(&parse("mine")).exists(),
        "Step 4 should remove the central workspace directory",
    );
}

/// Re-running `remove --force` on an already-gone workspace exits 13 —
/// the workspace row is absent, so step 3 sees no row. This documents
/// the "idempotent re-remove" semantics: orphan-cleanup is NOT done by
/// re-running `remove`; it's done by `tome doctor` (US5).
#[test]
fn remove_idempotent_re_remove_exits_13() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");
    workspace::remove::remove(parse("mine"), true, &paths, tmp.path()).expect("first remove");

    // Re-run: workspace already gone.
    let err = workspace::remove::remove(parse("mine"), true, &paths, tmp.path()).unwrap_err();
    assert_eq!(err.exit_code(), 13);
    assert!(matches!(err, TomeError::WorkspaceNotFound { .. }));
}

/// Concurrent `remove` + `workspace use` (binding) for the same
/// workspace via `Barrier::new(2)`. The advisory lockfile is
/// non-blocking — `index::acquire_lock` returns
/// [`TomeError::IndexBusy`] (exit 50) on contention rather than waiting
/// the loser out. So exactly one of the two writers wins the lock; the
/// other surfaces `IndexBusy`. If the winner happens to be `remove`,
/// the loser may also see `WorkspaceNotFound` (exit 13) once it
/// retries; we accept either of those two outcomes for the loser.
#[test]
fn remove_concurrent_with_workspace_use_one_wins() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("mine"), false, &paths).expect("init");

    let barrier = Arc::new(Barrier::new(2));
    let paths_a = paths.clone();
    let paths_b = paths.clone();
    let home_root_a = tmp.path().to_path_buf();
    let project = tmp.path().join("contender");
    std::fs::create_dir_all(&project).expect("create contender dir");

    let barrier_a = Arc::clone(&barrier);
    let barrier_b = Arc::clone(&barrier);
    let project_for_bind = project.clone();
    let home_for_bind = tmp.path().to_path_buf();

    let h_remove = std::thread::spawn(move || {
        barrier_a.wait();
        workspace::remove::remove(parse("mine"), true, &paths_a, &home_root_a)
    });
    let h_bind = std::thread::spawn(move || {
        barrier_b.wait();
        let deps = tome::workspace::BindDeps {
            paths: &paths_b,
            home_root: &home_for_bind,
        };
        tome::workspace::bind_project(&project_for_bind, parse("mine"), false, &deps)
    });

    let res_remove = h_remove.join().expect("join remove");
    let res_bind = h_bind.join().expect("join bind");

    let remove_ok = res_remove.is_ok();
    let bind_ok = res_bind.is_ok();

    // At least one must have succeeded — the advisory lock guarantees
    // serialised access; whichever acquires first runs to completion.
    assert!(
        remove_ok || bind_ok,
        "at least one of remove/bind must succeed; got remove={:?}, bind={:?}",
        res_remove.as_ref().err(),
        res_bind.as_ref().err(),
    );

    // No race in which BOTH succeed — they touch overlapping state under
    // the same advisory lock. If one is `Ok`, the other failed; it must
    // be either `IndexBusy` (lost the non-blocking try-lock) or
    // `WorkspaceNotFound` (lock acquired after the workspace was
    // removed).
    match (remove_ok, bind_ok) {
        (true, false) => {
            let err = res_bind.unwrap_err();
            let code = err.exit_code();
            assert!(
                code == 13 || code == 50,
                "bind loser should surface WorkspaceNotFound (13) or IndexBusy (50); got {err:?} (code {code})",
            );
            assert!(!workspace_exists(&paths, "mine"));
        }
        (false, true) => {
            // Bind acquired the lock first → workspace still present.
            // Remove may have failed with IndexBusy (couldn't acquire
            // the lock) OR may have observed the post-bind state. Since
            // we passed force=true and a bound project would be torn
            // down, the only failure mode is IndexBusy.
            let err = res_remove.unwrap_err();
            assert_eq!(
                err.exit_code(),
                50,
                "remove loser with force=true should surface IndexBusy (50); got {err:?}",
            );
            assert!(workspace_exists(&paths, "mine"));
            assert_eq!(count_workspace_projects_for(&paths, "mine"), 1);
        }
        (true, true) => {
            // Both succeeded — only possible if one acquired and
            // released the lock fast enough for the other to acquire
            // it afterwards. In that case bind ran first (workspace
            // present, binding installed) and remove ran second
            // (cascaded the binding away).
            assert!(!workspace_exists(&paths, "mine"));
            assert_eq!(count_workspace_projects_for(&paths, "mine"), 0);
        }
        (false, false) => unreachable!("the assertion above guarantees at least one Ok"),
    }
}

/// Happy-path outcome carries an empty `orphaned_paths` vector. Covers
/// the wire-shape field — the actual orphan-injection scenario is
/// recoverable via `tome doctor` (US5) and is exercised in that suite.
#[test]
fn remove_happy_path_has_no_orphans() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("solo"), false, &paths).expect("init");
    let outcome =
        workspace::remove::remove(parse("solo"), false, &paths, tmp.path()).expect("remove");
    let expected: Vec<PathBuf> = Vec::new();
    assert_eq!(outcome.orphaned_paths, expected);
}

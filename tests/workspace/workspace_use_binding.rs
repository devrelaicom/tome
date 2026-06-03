//! Integration tests for `tome workspace use <name>` (Phase 4 / US1.a).
//!
//! Covers the project-binding flow in `src/workspace/binding.rs` plus the
//! CLI surface in `src/commands/workspace/use_.rs`. The harness sync seam
//! (`commands::harness::sync_for_project_root`) is a stub in US1.a — these
//! tests don't assert on its behaviour beyond "no panic".
//!
//! Test mix per the project convention (heavy-state via library API,
//! light/error paths via CLI binary):
//!
//! - 1, 7 — CLI binary (real `Some(exit_code)` semantics).
//! - 2 — library API (pure-function check).
//! - 3, 4, 5, 6, 8 — library API (DB + filesystem state).

use std::path::{Path, PathBuf};
use std::sync::Barrier;
use std::time::Duration;

use crate::common::{
    ToolEnv, lifecycle_paths, paths_for, seed_workspace, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};
use tempfile::TempDir;
use tome::index::{self, OpenOptions};
use tome::workspace::WorkspaceName;
use tome::workspace::binding::{self, BindDeps};

/// Wire the binding library with a project root under `project_dir` and a
/// home root under `home_dir`. Returns the resolved `Paths` so test
/// bodies can hand it to `BindDeps`.
fn wire(env: &ToolEnv) -> tome::paths::Paths {
    let paths = paths_for(env);
    std::fs::create_dir_all(&paths.root).expect("create tome root");
    paths
}

/// Read `workspace_projects.workspace_id` for the given project_path.
fn read_binding_row(paths: &tome::paths::Paths, project_path: &Path) -> Option<(i64, String, i64)> {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open index");
    let pp = project_path.to_string_lossy().into_owned();
    conn.query_row(
        "SELECT wp.workspace_id, w.name, wp.bound_at
         FROM workspace_projects AS wp
         JOIN workspaces AS w ON w.id = wp.workspace_id
         WHERE wp.project_path = ?1",
        rusqlite::params![pp.as_str()],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        },
    )
    .ok()
}

fn read_last_used_at(paths: &tome::paths::Paths, workspace: &str) -> i64 {
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
        "SELECT last_used_at FROM workspaces WHERE name = ?1",
        rusqlite::params![workspace],
        |row| row.get(0),
    )
    .expect("read last_used_at")
}

// ---------------------------------------------------------------------------
// 0. Library API: refuses a non-UTF8 project path with TomeError::Io.
//    Phase 4 / US1.d-2a R-B1.
// ---------------------------------------------------------------------------

// Gated to Linux because macOS APFS rejects illegal byte sequences at
// the syscall layer (`Os { code: 92, kind: Uncategorized, message:
// "Illegal byte sequence" }` from `mkdir(2)`), so we can't even fabricate
// the bad filename to drive the test. The production code path under
// test (the `to_str().is_none()` check in `bind_project`) is platform-
// independent — Linux coverage is sufficient to pin the behaviour.
#[cfg(target_os = "linux")]
#[test]
fn refuses_non_utf8_project_path() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "my-workspace");

    // Build a project directory whose final path component is not
    // valid UTF-8. `\xff\xfe` is a UTF-16 BOM that is not valid UTF-8.
    let bad_name = OsString::from_vec(b"bad\xff\xfename".to_vec());
    let bad_dir = tmp.path().join(bad_name);
    std::fs::create_dir_all(&bad_dir).expect("create non-UTF8 dir");

    let home = tmp.path().join("fake-home");
    std::fs::create_dir_all(&home).unwrap();
    let deps = BindDeps {
        paths: &paths,
        home_root: &home,
    };
    let name = WorkspaceName::parse("my-workspace").unwrap();

    let err = binding::bind_project(&bad_dir, name, false, &deps)
        .expect_err("non-UTF8 project path must be refused");
    assert_eq!(
        err.exit_code(),
        7,
        "expected TomeError::Io (exit 7); got {err:?}",
    );
}

// ---------------------------------------------------------------------------
// 1. CLI binary: refuses to bind when CWD is the user's home directory.
// ---------------------------------------------------------------------------

#[test]
fn cwd_is_home_refuses_with_exit_2() {
    let env = ToolEnv::new();
    let paths = wire(&env);
    seed_workspace(&paths, "my-workspace");

    // Run from $HOME (the ToolEnv's isolated home). Without --force, the
    // binding must refuse with exit 2.
    let output = env
        .cmd()
        .current_dir(env.home_path())
        .args(["workspace", "use", "my-workspace"])
        .output()
        .expect("spawn tome");
    assert!(!output.status.success(), "expected failure exit");
    assert_eq!(output.status.code(), Some(2), "expected exit 2");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--force"),
        "stderr must mention --force; got: {stderr}",
    );
    assert!(
        stderr.contains("home directory"),
        "stderr must name the offending condition; got: {stderr}",
    );
}

// ---------------------------------------------------------------------------
// 2. Library API: refuses when cwd is /.
// ---------------------------------------------------------------------------

#[test]
fn cwd_is_root_refuses_with_exit_2() {
    let some_home = PathBuf::from("/some/home/dir");
    let err = binding::is_project_root_acceptable(Path::new("/"), &some_home)
        .expect_err("must refuse on /");
    assert_eq!(err.exit_code(), 2);
}

// ---------------------------------------------------------------------------
// 3. Library API: nonexistent workspace → exit 13.
// ---------------------------------------------------------------------------

#[test]
fn nonexistent_workspace_exits_13() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();

    let home = tmp.path().join("fake-home");
    std::fs::create_dir_all(&home).unwrap();
    let deps = BindDeps {
        paths: &paths,
        home_root: &home,
    };

    let name = WorkspaceName::parse("missing").unwrap();
    let err = binding::bind_project(&project, name, false, &deps).expect_err("must fail");
    assert_eq!(err.exit_code(), 13, "want WorkspaceNotFound; got {err:?}");
}

// ---------------------------------------------------------------------------
// 4. Library API happy path: marker materialises and DB row appears.
// ---------------------------------------------------------------------------

#[test]
fn happy_path_creates_marker_and_row() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "my-workspace");

    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();

    let home = tmp.path().join("fake-home");
    std::fs::create_dir_all(&home).unwrap();
    let deps = BindDeps {
        paths: &paths,
        home_root: &home,
    };
    let name = WorkspaceName::parse("my-workspace").unwrap();
    let outcome = binding::bind_project(&project, name, false, &deps).expect("bind");

    assert!(outcome.created_marker);
    assert!(outcome.rebind_from.is_none());

    // Marker on disk.
    let marker_dir = project.canonicalize().unwrap().join(".tome");
    assert!(marker_dir.is_dir(), "marker dir must exist");
    let cfg = std::fs::read_to_string(marker_dir.join("config.toml")).expect("read config.toml");
    assert!(
        cfg.contains("workspace = \"my-workspace\""),
        "expected workspace line in config.toml; got: {cfg}"
    );

    // DB row.
    let row = read_binding_row(&paths, &project.canonicalize().unwrap())
        .expect("workspace_projects row must exist");
    assert_eq!(row.1, "my-workspace");
}

// ---------------------------------------------------------------------------
// 5. Library API: idempotent rebind to same workspace.
// ---------------------------------------------------------------------------

#[test]
fn idempotent_rebind_same_workspace() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "my-workspace");

    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();

    let home = tmp.path().join("fake-home");
    std::fs::create_dir_all(&home).unwrap();
    let deps = BindDeps {
        paths: &paths,
        home_root: &home,
    };
    let name = WorkspaceName::parse("my-workspace").unwrap();

    let first = binding::bind_project(&project, name.clone(), false, &deps).expect("bind 1");
    assert!(first.created_marker);
    assert!(first.rebind_from.is_none());

    let row1 = read_binding_row(&paths, &project.canonicalize().unwrap()).unwrap();

    // Wait one second so bound_at can advance at second-granularity.
    std::thread::sleep(Duration::from_secs(1));

    let second = binding::bind_project(&project, name, false, &deps).expect("bind 2");
    assert!(!second.created_marker, "marker exists on the second call");
    assert!(second.rebind_from.is_none(), "same workspace = no rebind");

    let row2 = read_binding_row(&paths, &project.canonicalize().unwrap()).unwrap();
    assert_eq!(row1.0, row2.0, "workspace_id stays the same");
    assert!(row2.2 > row1.2, "bound_at must advance on re-UPSERT");
}

// ---------------------------------------------------------------------------
// 6. Library API: rebind to a DIFFERENT workspace reports rebind_from.
// ---------------------------------------------------------------------------

#[test]
fn rebind_to_different_workspace_upserts() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "ws-a");
    seed_workspace(&paths, "ws-b");

    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();

    let home = tmp.path().join("fake-home");
    std::fs::create_dir_all(&home).unwrap();
    let deps = BindDeps {
        paths: &paths,
        home_root: &home,
    };

    let a = WorkspaceName::parse("ws-a").unwrap();
    let b = WorkspaceName::parse("ws-b").unwrap();

    let _first = binding::bind_project(&project, a, false, &deps).expect("bind ws-a");
    let second = binding::bind_project(&project, b, false, &deps).expect("bind ws-b");

    assert_eq!(
        second.rebind_from.as_ref().map(|n| n.as_str()),
        Some("ws-a"),
        "rebind_from must name the prior workspace",
    );
    assert_eq!(second.workspace.as_str(), "ws-b");

    // Only one row for that project_path.
    let project_c = project.canonicalize().unwrap();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .unwrap();
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_projects WHERE project_path = ?1",
            rusqlite::params![project_c.to_string_lossy().into_owned()],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(n, 1, "UPSERT must keep one row per project_path");
}

// ---------------------------------------------------------------------------
// 7. Library API: last_used_at advances on bind.
// ---------------------------------------------------------------------------

#[test]
fn last_used_at_advances_on_bind() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "my-workspace");

    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let home = tmp.path().join("fake-home");
    std::fs::create_dir_all(&home).unwrap();
    let deps = BindDeps {
        paths: &paths,
        home_root: &home,
    };

    let name = WorkspaceName::parse("my-workspace").unwrap();
    binding::bind_project(&project, name.clone(), false, &deps).expect("bind 1");
    let lua1 = read_last_used_at(&paths, "my-workspace");

    std::thread::sleep(Duration::from_secs(1));

    binding::bind_project(&project, name, false, &deps).expect("bind 2");
    let lua2 = read_last_used_at(&paths, "my-workspace");

    assert!(
        lua2 > lua1,
        "last_used_at must advance: was {lua1}, now {lua2}",
    );
}

// ---------------------------------------------------------------------------
// 8. Library API: concurrent bind to different workspaces — last writer
//    wins, no panic, marker agrees with the DB row.
// ---------------------------------------------------------------------------

#[test]
fn concurrent_bind_two_threads_last_wins() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "ws-a");
    seed_workspace(&paths, "ws-b");

    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let home = tmp.path().join("fake-home");
    std::fs::create_dir_all(&home).unwrap();

    let barrier = std::sync::Arc::new(Barrier::new(2));
    let project_a = project.clone();
    let project_b = project.clone();
    let paths_a = paths.clone();
    let paths_b = paths.clone();
    let home_a = home.clone();
    let home_b = home.clone();
    let b_a = barrier.clone();
    let b_b = barrier.clone();

    let h_a = std::thread::spawn(move || {
        let deps = BindDeps {
            paths: &paths_a,
            home_root: &home_a,
        };
        let name = WorkspaceName::parse("ws-a").unwrap();
        b_a.wait();
        binding::bind_project(&project_a, name, false, &deps)
    });
    let h_b = std::thread::spawn(move || {
        let deps = BindDeps {
            paths: &paths_b,
            home_root: &home_b,
        };
        let name = WorkspaceName::parse("ws-b").unwrap();
        b_b.wait();
        binding::bind_project(&project_b, name, false, &deps)
    });

    // At least one must succeed; the other may either succeed (queued
    // through the lock) or fail with IndexBusy. Both are tolerated.
    let r_a = h_a.join().expect("thread a panicked");
    let r_b = h_b.join().expect("thread b panicked");

    match (&r_a, &r_b) {
        (Ok(_), Ok(_)) => {} // serialised
        (Ok(_), Err(e)) | (Err(e), Ok(_)) => {
            assert_eq!(e.exit_code(), 50, "non-success must be IndexBusy");
        }
        (Err(_), Err(_)) => panic!("both threads failed; expected at least one to succeed"),
    }

    // The DB row references some workspace; the marker config.toml
    // names the same workspace.
    let project_c = project.canonicalize().unwrap();
    let row = read_binding_row(&paths, &project_c).expect("a row must exist");
    let cfg = std::fs::read_to_string(project_c.join(".tome").join("config.toml"))
        .expect("read marker config.toml");
    let cfg_line = format!("workspace = \"{}\"", row.1);
    assert!(
        cfg.contains(&cfg_line),
        "marker workspace must agree with DB row `{}`; got: {cfg}",
        row.1,
    );
}

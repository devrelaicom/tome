//! Atomic-marker-landing semantics for `bind_project` (Phase 4 /
//! US1.d-1 / T162).
//!
//! Most atomicity guarantees of the underlying helper
//! (`tome::util::atomic_dir`) are already exhaustively tested in
//! `tests/atomic_dir.rs`. This file pins the bind-specific surface:
//!
//! 1. After a rebind, the `.tome.old/` aside is cleaned up best-effort.
//! 2. The contract-stable staging prefix (`.tome.tmp.`) is unchanged —
//!    US5's doctor `--fix` sweep depends on this string.
//! 3. The bind's marker is replaced atomically: there is never an
//!    observable state where neither the old nor the new `.tome/` exists.
//!
//! SIGINT-mid-populate semantics are deliberately not exercised here —
//! injecting signal handlers from cargo's parallel test process races
//! every other test in the binary (same discipline as
//! `tests/atomicity_enable.rs`). The TempDir-cleanup-on-populate-err
//! guarantee that proxies for SIGINT is covered by
//! `tests/atomic_dir.rs::populate_failure_drops_staging_dir`.
//!
//! Unhide target for fuller SIGINT-mid-bind coverage: when US5 ships
//! doctor `--fix` orphan cleanup, that suite can drive a bind, kill the
//! process between marker-stage and rename, and verify the doctor sweep
//! recovers the orphan staging dir.

mod common;

use std::sync::Mutex;

use common::{HarnessModulesGuard, lifecycle_paths, seed_workspace};
use tempfile::TempDir;
use tome::util::atomic_dir::STAGING_PREFIX;
use tome::workspace::WorkspaceName;
use tome::workspace::binding::{self, BindDeps};

static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// 1. Rebind cleans up `.tome.old/` after the new marker lands.
// ---------------------------------------------------------------------------

#[test]
fn binding_marker_replace_cleans_up_old_sibling() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![]);

    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(&tmp.path().join(".tome"));
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

    binding::bind_project(&project, a, false, &deps).expect("bind ws-a");
    binding::bind_project(&project, b, false, &deps).expect("bind ws-b");

    let canonical = project.canonicalize().unwrap();

    // The new marker is in place.
    let cfg = std::fs::read_to_string(canonical.join(".tome").join("config.toml")).unwrap();
    assert!(
        cfg.contains("workspace = \"ws-b\""),
        "new marker must reflect ws-b; got: {cfg}",
    );

    // The aside sibling has been cleaned up. `land_directory_with_replace`
    // builds the aside name via `with_file_name`, so for `.tome` it
    // becomes `.tome.old`.
    let aside = canonical.join(".tome.old");
    assert!(
        !aside.exists(),
        "rebind must clean up .tome.old sibling after success; still present at {}",
        aside.display(),
    );

    drop(tmp);
}

// ---------------------------------------------------------------------------
// 2. The staging prefix is contractually pinned. US5's doctor `--fix`
//    will sweep `<project>/.tome.tmp.*` orphans — if this string changes
//    that sweep stops matching.
// ---------------------------------------------------------------------------

#[test]
fn staged_tmp_dir_prefix_is_documented() {
    assert_eq!(
        STAGING_PREFIX, ".tome.tmp.",
        "STAGING_PREFIX is contractually pinned for US5 doctor cleanup",
    );
}

// ---------------------------------------------------------------------------
// 3. The marker replace operation is observably atomic: there is no
//    intermediate state where the marker is partially populated.
//
//    Since `land_directory_with_replace` is a single rename of the
//    fully-populated staging dir over the (already aside-renamed) old
//    target, any external observer either sees the new `.tome/` (full
//    contents) or the freshly-renamed `.tome.old/`, never a half-populated
//    `.tome/`. The strongest assertion we can make in a single-thread
//    test is that after `bind_project` returns, the `.tome/` is fully
//    populated AND no `.tome.tmp.*` staging siblings remain.
// ---------------------------------------------------------------------------

#[test]
fn bind_leaves_no_staging_or_aside_residue() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![]);

    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(&tmp.path().join(".tome"));
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "ws-a");

    let project = tmp.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let home = tmp.path().join("fake-home");
    std::fs::create_dir_all(&home).unwrap();
    let deps = BindDeps {
        paths: &paths,
        home_root: &home,
    };

    let name = WorkspaceName::parse("ws-a").unwrap();
    binding::bind_project(&project, name, false, &deps).expect("bind");

    let canonical = project.canonicalize().unwrap();

    // No `.tome.tmp.*` siblings.
    let stragglers: Vec<_> = std::fs::read_dir(&canonical)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(STAGING_PREFIX))
        .collect();
    assert!(
        stragglers.is_empty(),
        "no .tome.tmp.* siblings must remain after a successful bind; found: {:?}",
        stragglers.iter().map(|e| e.path()).collect::<Vec<_>>(),
    );

    // The marker is fully populated — config.toml present with the
    // workspace line.
    let cfg_path = canonical.join(".tome").join("config.toml");
    let cfg = std::fs::read_to_string(&cfg_path).expect("read config.toml");
    assert!(
        cfg.contains("workspace = \"ws-a\""),
        "marker must be fully populated; got: {cfg}",
    );

    drop(tmp);
}

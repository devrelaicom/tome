//! Phase 5 / US2.b — data-directory lazy-creation tests.
//!
//! `tests/substitution_builtins.rs` already exercises:
//! - `plugin_data_directory_exists_on_disk_after_substitution`
//! - `plugin_data_path_sanitises_catalog_and_plugin_components`
//! - `create_dir_all_failure_surfaces_plugin_data_dir_creation_failed`
//!
//! This file picks up the gaps:
//! - Idempotence (second render against same context creates no harm)
//! - `${TOME_WORKSPACE_DATA}` lazy creation (mirror of the plugin-data case)
//! - Concurrent two-thread `create_dir_all` race (NFR-012)
//! - Workspace-rename relocation: plugin-data follows the rename target
//! - Workspace-rename when no plugin-data existed pre-rename: silent no-op
//!
//! The first three exercise the substitution layer's `ensure_*_data`
//! paths via the production `render()` entry; the last two reach into
//! `workspace::rename::rename` to prove the FR-025 contract.
//!
//! Per the `OVERRIDE_MUTEX` pattern from Phase 4 / US3.c-1
//! `tests/harness_sync_stub.rs`, the rename tests do NOT install any
//! substitution-layer override — they exercise the real-on-disk
//! relocation path. The first three tests serialise against
//! `OVERRIDE_MUTEX` because they would race other tests installing
//! `WORKSPACE_DATA_DIR_OVERRIDE`.

mod common;

use std::path::PathBuf;
use std::sync::{Arc, Barrier, Mutex, MutexGuard, OnceLock};

use common::{PluginDataDirGuard, WorkspaceDataDirGuard, lifecycle_paths};
use tempfile::TempDir;
use time::OffsetDateTime;
use tome::substitution::{self, SubstitutionContext, SubstitutionContextBuilder};
use tome::workspace::{self, WorkspaceName};

static OVERRIDE_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

fn lock_overrides() -> MutexGuard<'static, ()> {
    OVERRIDE_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

fn parse(name: &str) -> WorkspaceName {
    WorkspaceName::parse(name).expect("valid workspace name")
}

fn ctx_builder(home: &std::path::Path) -> SubstitutionContextBuilder {
    let paths = lifecycle_paths(home);
    SubstitutionContext::builder()
        .catalog_name("c")
        .plugin_name("p")
        .plugin_version("1.0.0")
        .entry_name("e")
        .entry_path(PathBuf::from("/x/e.md"))
        .entry_dir(PathBuf::from("/x"))
        .plugin_root_dir(PathBuf::from("/x"))
        .plugin_data_dir(PathBuf::from("/x/plugin-data"))
        .workspace_name("global")
        .workspace_data_dir(PathBuf::from("/x/workspace-data"))
        .clock(OffsetDateTime::UNIX_EPOCH)
        .paths(paths)
}

// --- Lazy creation: idempotence + workspace-data ----------------------------

#[test]
fn plugin_data_lazy_create_is_idempotent_across_renders() {
    let _lock = lock_overrides();
    let tmp = TempDir::new().unwrap();
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let ctx = ctx_builder(tmp.path()).build().unwrap();

    let first = substitution::render("p=${TOME_PLUGIN_DATA}", &ctx).unwrap();
    let dir_path = std::path::PathBuf::from(first.strip_prefix("p=").unwrap());
    assert!(dir_path.is_dir(), "first render should lazy-create");

    // Second render against the same context — no-op on the FS side.
    let second = substitution::render("p=${TOME_PLUGIN_DATA}", &ctx).unwrap();
    assert_eq!(first, second);
    assert!(
        dir_path.is_dir(),
        "directory should still exist after second render",
    );
}

#[test]
fn workspace_data_directory_exists_on_disk_after_substitution() {
    let _lock = lock_overrides();
    let tmp = TempDir::new().unwrap();
    // Real path computation: WORKSPACE_DATA_DIR_OVERRIDE NOT installed
    // — the production `ensure_workspace_data` path runs and
    // create_dir_all's the path.
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let ctx = ctx_builder(tmp.path()).build().unwrap();

    let out = substitution::render("w=${TOME_WORKSPACE_DATA}", &ctx).unwrap();
    let real_path = std::path::Path::new(&out[2..]);
    assert!(
        real_path.is_dir(),
        "expected {} to exist after substitution",
        real_path.display(),
    );
    // Path anchored under <home>/.tome/workspaces/global/plugin-data/c/p
    let expected_suffix = PathBuf::from("workspaces")
        .join("global")
        .join("plugin-data")
        .join("c")
        .join("p");
    assert!(
        real_path.ends_with(&expected_suffix),
        "{} did not end with {}",
        real_path.display(),
        expected_suffix.display(),
    );
}

#[test]
fn workspace_data_lazy_create_is_idempotent_across_renders() {
    let _lock = lock_overrides();
    let tmp = TempDir::new().unwrap();
    let _p = PluginDataDirGuard::install(tmp.path().join("pd"));
    let ctx = ctx_builder(tmp.path()).build().unwrap();

    let first = substitution::render("w=${TOME_WORKSPACE_DATA}", &ctx).unwrap();
    let dir_path = std::path::PathBuf::from(first.strip_prefix("w=").unwrap());
    assert!(dir_path.is_dir());
    let second = substitution::render("w=${TOME_WORKSPACE_DATA}", &ctx).unwrap();
    assert_eq!(first, second);
    assert!(dir_path.is_dir());
}

#[test]
fn concurrent_renders_race_safely_against_create_dir_all() {
    // NFR-012: two threads invoking the same lazy-creation path against
    // the same target dir must both succeed. `create_dir_all` is
    // kernel-atomic and idempotent under concurrent retrievals; this
    // test pins the production behaviour.
    let _lock = lock_overrides();
    let tmp = TempDir::new().unwrap();
    let _w = WorkspaceDataDirGuard::install(tmp.path().join("wd"));
    let paths = lifecycle_paths(tmp.path());

    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let b = barrier.clone();
        let paths_clone = paths.clone();
        handles.push(std::thread::spawn(move || {
            let ctx = SubstitutionContext::builder()
                .catalog_name("c")
                .plugin_name("p")
                .plugin_version("1.0.0")
                .entry_name("e")
                .entry_path(PathBuf::from("/x/e.md"))
                .entry_dir(PathBuf::from("/x"))
                .plugin_root_dir(PathBuf::from("/x"))
                .plugin_data_dir(PathBuf::from("/x/plugin-data"))
                .workspace_name("global")
                .workspace_data_dir(PathBuf::from("/x/workspace-data"))
                .clock(OffsetDateTime::UNIX_EPOCH)
                .paths(paths_clone)
                .build()
                .unwrap();
            b.wait();
            substitution::render("p=${TOME_PLUGIN_DATA}", &ctx).expect("render should succeed")
        }));
    }
    let outputs: Vec<String> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(outputs[0], outputs[1]);
    let dir_path = std::path::PathBuf::from(outputs[0].strip_prefix("p=").unwrap());
    assert!(
        dir_path.is_dir(),
        "expected {} to exist after concurrent renders",
        dir_path.display(),
    );
}

// --- Workspace rename relocation (FR-025) -----------------------------------
//
// These tests reach into `workspace::rename` directly. They do NOT
// install any substitution override — the actual on-disk relocation
// path is what's under test.

#[test]
fn rename_relocates_plugin_data_subtree_to_new_workspace_dir() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("alpha"), false, &paths).expect("init alpha");

    // Pre-seed plugin-data subtree under alpha — simulates a previous
    // substitution pass that materialised <TOME_WORKSPACE_DATA>.
    let alpha_pd = paths.workspace_dir(&parse("alpha")).join("plugin-data");
    let alpha_pd_file = alpha_pd
        .join("catalog-x")
        .join("plugin-y")
        .join("notes.txt");
    std::fs::create_dir_all(alpha_pd_file.parent().unwrap()).unwrap();
    std::fs::write(&alpha_pd_file, b"persisted by user plugin").unwrap();

    workspace::rename::rename(parse("alpha"), parse("beta"), &paths).expect("rename");

    // Source plugin-data dir is gone.
    assert!(
        !alpha_pd.exists(),
        "alpha plugin-data should be gone after rename",
    );

    // The renamed workspace's plugin-data tree carries the original
    // contents byte-for-byte.
    let beta_pd_file = paths
        .workspace_dir(&parse("beta"))
        .join("plugin-data")
        .join("catalog-x")
        .join("plugin-y")
        .join("notes.txt");
    assert!(
        beta_pd_file.is_file(),
        "{} should exist",
        beta_pd_file.display()
    );
    let contents = std::fs::read(&beta_pd_file).unwrap();
    assert_eq!(contents, b"persisted by user plugin");
}

#[test]
fn rename_without_plugin_data_subtree_is_silent_no_op_on_relocation() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    workspace::init::init(parse("alpha"), false, &paths).expect("init alpha");

    // No plugin-data subtree pre-seeded — this is the workspace that
    // has never been touched by a substitution-bearing entry.
    let alpha_pd = paths.workspace_dir(&parse("alpha")).join("plugin-data");
    assert!(!alpha_pd.exists());

    // Rename succeeds without surfacing WorkspaceDataDirWriteFailed.
    workspace::rename::rename(parse("alpha"), parse("beta"), &paths).expect("rename");

    // The renamed workspace dir exists; the plugin-data subdir is still
    // absent (the rename has no work to do for plugin-data).
    let beta_dir = paths.workspace_dir(&parse("beta"));
    assert!(beta_dir.is_dir());
    assert!(!beta_dir.join("plugin-data").exists());
}

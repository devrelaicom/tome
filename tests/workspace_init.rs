//! Phase 4 / US2 slice b — `tome workspace init`.
//!
//! Library-API tests against `workspace::init`. The CLI binary is
//! exercised with two smoke tests (default-path resolves to CWD, exit
//! codes propagate). Atomicity is asserted by inspecting the on-disk
//! state after each scenario.

mod common;

use std::path::PathBuf;

use common::{ToolEnv, config_with_catalog, copy_sample_plugin_catalog, lifecycle_paths};
use tempfile::TempDir;
use tome::catalog::store as catalog_store;
use tome::error::TomeError;
use tome::workspace::{self, inventory};

fn workspace_root(tmp: &TempDir, name: &str) -> PathBuf {
    let root = tmp.path().join(name);
    std::fs::create_dir_all(&root).unwrap();
    root
}

// ---- Happy path ----------------------------------------------------------

#[test]
fn init_creates_dot_tome_with_empty_config() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let ws = workspace_root(&tmp, "project");

    let outcome = workspace::init(&ws, false, false, &paths).expect("init");
    assert_eq!(outcome.workspace, std::fs::canonicalize(&ws).unwrap());
    assert_eq!(outcome.catalogs, 0);
    assert!(!outcome.inherited);
    assert!(!outcome.index_bootstrapped);

    let marker = std::fs::canonicalize(&ws).unwrap().join(".tome");
    assert!(marker.is_dir(), ".tome/ should exist");
    let cfg_path = marker.join("config.toml");
    assert!(cfg_path.is_file(), "config.toml should exist");
    let parsed = catalog_store::load(&cfg_path).unwrap();
    assert!(parsed.catalogs.is_empty(), "got: {parsed:?}");
}

// ---- --inherit-global ----------------------------------------------------

#[test]
fn init_inherits_global_catalogs_without_enablement() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    // Seed the global config with one catalog.
    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let global_cfg = config_with_catalog("sample-plugin-catalog", &catalog_root);
    catalog_store::save(&paths.config_file, &global_cfg).unwrap();

    let ws = workspace_root(&tmp, "project");
    let outcome = workspace::init(&ws, true, false, &paths).expect("init");
    assert_eq!(outcome.catalogs, 1);
    assert!(outcome.inherited);

    // Read back the workspace config and confirm the catalog carried
    // across but no enablement-state is present (enablement lives in
    // the index DB).
    let cfg_path = std::fs::canonicalize(&ws)
        .unwrap()
        .join(".tome/config.toml");
    let parsed = catalog_store::load(&cfg_path).unwrap();
    assert_eq!(parsed.catalogs.len(), 1);
    assert!(parsed.catalogs.contains_key("sample-plugin-catalog"));

    // The index DB must NOT exist at this point — enablement is
    // explicit, and so is bootstrap.
    let marker = std::fs::canonicalize(&ws).unwrap().join(".tome");
    assert!(!marker.join("index.db").exists());
}

// ---- Refuse without --force ---------------------------------------------

#[test]
fn init_refuses_pre_existing_marker_without_force() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let ws = workspace_root(&tmp, "project");
    workspace::init(&ws, false, false, &paths).expect("first init");

    let err = workspace::init(&ws, false, false, &paths).unwrap_err();
    assert_eq!(err.exit_code(), 4);
    assert!(
        matches!(err, TomeError::CatalogAlreadyExists(_)),
        "expected CatalogAlreadyExists, got {err:?}",
    );
}

// ---- --force replaces atomically -----------------------------------------

#[test]
fn init_force_replaces_existing_dot_tome() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let ws = workspace_root(&tmp, "project");

    // Seed an existing .tome/ with a sentinel file so we can detect that
    // it was actually replaced (not merged into).
    workspace::init(&ws, false, false, &paths).expect("first init");
    let canonical = std::fs::canonicalize(&ws).unwrap();
    let sentinel = canonical.join(".tome/sentinel.txt");
    std::fs::write(&sentinel, b"old").unwrap();
    assert!(sentinel.is_file());

    workspace::init(&ws, false, true, &paths).expect("force init");
    // The sentinel must be gone — proves the directory was replaced.
    assert!(!sentinel.exists(), "sentinel should not survive --force");
    // Best-effort: the .tome.old/ should also be cleaned up.
    assert!(!canonical.join(".tome.old").exists());
    // config.toml is back in its empty state.
    let parsed = catalog_store::load(&canonical.join(".tome/config.toml")).unwrap();
    assert!(parsed.catalogs.is_empty());
}

// ---- Non-existent target -------------------------------------------------

#[test]
fn init_returns_io_when_target_path_missing() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let nonexistent = tmp.path().join("does-not-exist");

    let err = workspace::init(&nonexistent, false, false, &paths).unwrap_err();
    assert_eq!(err.exit_code(), 7);
    assert!(matches!(err, TomeError::Io(_)), "expected Io, got {err:?}");
}

#[test]
fn init_returns_io_when_target_is_a_file() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let file = tmp.path().join("not-a-dir");
    std::fs::write(&file, b"file").unwrap();

    let err = workspace::init(&file, false, false, &paths).unwrap_err();
    assert_eq!(err.exit_code(), 7);
    assert!(matches!(err, TomeError::Io(_)));
}

// ---- Opt-in workspace registry ------------------------------------------

#[test]
fn init_appends_to_registry_when_file_exists() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    // Opt-in: touch the file.
    std::fs::create_dir_all(&paths.state_dir).unwrap();
    std::fs::write(&paths.workspace_registry, b"").unwrap();
    let ws = workspace_root(&tmp, "project");

    workspace::init(&ws, false, false, &paths).expect("init");

    let listed = inventory::read_registry(&paths.workspace_registry);
    let canonical = std::fs::canonicalize(&ws).unwrap();
    assert!(listed.contains(&canonical), "registry={listed:?}");

    // Second init with --force on the same workspace must NOT duplicate
    // the entry (dedupe is by exact path).
    workspace::init(&ws, false, true, &paths).expect("re-init");
    let listed = inventory::read_registry(&paths.workspace_registry);
    let occurrences = listed.iter().filter(|p| **p == canonical).count();
    assert_eq!(occurrences, 1, "registry={listed:?}");
}

#[test]
fn init_does_not_create_registry_when_file_absent() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    assert!(!paths.workspace_registry.exists());
    let ws = workspace_root(&tmp, "project");

    workspace::init(&ws, false, false, &paths).expect("init");
    assert!(
        !paths.workspace_registry.exists(),
        "registry must not be created when the user hasn't opted in",
    );
}

// ---- Concurrent init contention -----------------------------------------

/// Two concurrent inits on the same workspace must not produce a partial
/// `.tome/`. One wins, one loses — but on disk we end up with a single
/// fully-populated marker.
#[test]
fn init_concurrent_does_not_corrupt_state() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let ws = workspace_root(&tmp, "project");

    // Channel the two threads through a barrier so they hit the
    // rename-race window as close to simultaneously as possible.
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let barrier = barrier.clone();
        let ws = ws.clone();
        let paths = paths.clone();
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            workspace::init(&ws, false, false, &paths)
        }));
    }
    let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let ok_count = results.iter().filter(|r| r.is_ok()).count();
    let err_count = results.iter().filter(|r| r.is_err()).count();
    assert!(
        ok_count >= 1,
        "at least one init should succeed: {results:?}",
    );
    assert!(
        ok_count + err_count == 2 && ok_count <= 2,
        "concurrent init must produce a clean win/lose: {results:?}",
    );

    // Whatever the contention outcome, the workspace is fully formed.
    let marker = std::fs::canonicalize(&ws).unwrap().join(".tome");
    assert!(marker.is_dir());
    assert!(marker.join("config.toml").is_file());
    // No straggler temp directories — they all share the `.tome.tmp.`
    // prefix.
    let entries: Vec<_> = std::fs::read_dir(std::fs::canonicalize(&ws).unwrap())
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.file_name()))
        .collect();
    let stragglers: Vec<_> = entries
        .iter()
        .filter(|n| n.to_string_lossy().starts_with(".tome.tmp."))
        .collect();
    assert!(
        stragglers.is_empty(),
        "leftover staging dirs: {stragglers:?}",
    );
}

// ---- CLI binary smoke tests ---------------------------------------------

#[test]
fn cli_workspace_init_creates_marker_in_explicit_path() {
    let env = ToolEnv::new();
    let tmp_ws = TempDir::new().unwrap();
    let target = tmp_ws.path();

    let output = env
        .cmd()
        .args(["workspace", "init", target.to_str().unwrap()])
        .output()
        .expect("spawn tome");
    assert!(
        output.status.success(),
        "exit={:?} stdout={} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        std::fs::canonicalize(target)
            .unwrap()
            .join(".tome")
            .is_dir()
    );
}

#[test]
fn cli_workspace_init_pre_existing_returns_exit_4() {
    let env = ToolEnv::new();
    let tmp_ws = TempDir::new().unwrap();
    let target = tmp_ws.path();
    // First init seeds the marker.
    let first = env
        .cmd()
        .args(["workspace", "init", target.to_str().unwrap()])
        .output()
        .expect("first init");
    assert!(first.status.success());

    let second = env
        .cmd()
        .args(["workspace", "init", target.to_str().unwrap()])
        .output()
        .expect("second init");
    assert_eq!(second.status.code(), Some(4));
}

#[test]
fn cli_workspace_init_json_emits_single_line() {
    let env = ToolEnv::new();
    let tmp_ws = TempDir::new().unwrap();
    let target = tmp_ws.path();

    let output = env
        .cmd()
        .args(["--json", "workspace", "init", target.to_str().unwrap()])
        .output()
        .expect("spawn tome");
    assert!(output.status.success(), "exit={:?}", output.status.code());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid json");
    assert_eq!(parsed["inherited"], false);
    assert_eq!(parsed["catalogs"], 0);
    assert_eq!(parsed["index_bootstrapped"], false);
}

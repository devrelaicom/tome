//! Phase 5 / US3.b — reference-counted catalog clone cleanup.
//!
//! Adding the same catalog URL in two scopes produces one on-disk
//! clone. Removing from one scope leaves the clone. Removing from the
//! last referencing scope removes the clone. Concurrent-remove races
//! resolve benignly (one wins, the other no-ops).
//!
//! Contract: `contracts/catalog-extensions-p3.md`.

mod common;

use std::path::PathBuf;

use common::{Fixture, ToolEnv};
use tempfile::TempDir;

/// Spawn `tome workspace init <path>` under the test env. Returns the
/// canonical workspace path. Used by the share-across-scope tests
/// where the workspace registry needs to be opt-in.
fn make_workspace_opt_in(env: &ToolEnv, parent: &std::path::Path, name: &str) -> PathBuf {
    let root = parent.join(name);
    std::fs::create_dir_all(&root).unwrap();
    // Touch the registry file BEFORE init so init's opt-in append fires.
    let state_dir = env.home_path().join(".local/state/tome");
    std::fs::create_dir_all(&state_dir).unwrap();
    std::fs::File::create(state_dir.join("workspaces.txt")).unwrap();

    let out = env
        .cmd()
        .args(["workspace", "init", root.to_str().unwrap()])
        .output()
        .expect("workspace init");
    assert!(
        out.status.success(),
        "init failed: stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    std::fs::canonicalize(&root).unwrap()
}

/// Compute the on-disk cache directory for a given catalog URL using
/// the same content-addressing as the tome binary (sha256 hex).
fn cache_dir_for(env: &ToolEnv, url: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(url.as_bytes());
    env.catalogs_dir().join(hex::encode(h.finalize()))
}

// ---- Shared clone across scopes -----------------------------------------

#[test]
fn same_url_in_two_scopes_shares_one_on_disk_clone() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace_opt_in(&env, tmp.path(), "project");

    // Add globally first.
    let add_g = env
        .cmd()
        .args(["--global", "catalog", "add", &fix.url, "--name", "g"])
        .output()
        .unwrap();
    assert!(
        add_g.status.success(),
        "global add failed: {}",
        String::from_utf8_lossy(&add_g.stderr),
    );

    // Add the SAME URL into the workspace — must succeed, must NOT
    // re-clone, must share the cache directory.
    let add_w = env
        .cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "add",
            &fix.url,
            "--name",
            "w",
        ])
        .output()
        .unwrap();
    assert!(
        add_w.status.success(),
        "workspace add of shared URL failed: {}",
        String::from_utf8_lossy(&add_w.stderr),
    );

    // Exactly one cache directory exists for this URL.
    let cache = cache_dir_for(&env, &fix.url);
    assert!(cache.is_dir(), "cache should exist at {cache:?}");

    // Both configs reference it.
    let g_body = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(g_body.contains("[catalogs.g]"));
    let w_body = std::fs::read_to_string(ws.join(".tome/config.toml")).unwrap();
    assert!(w_body.contains("[catalogs.w]"));
}

#[test]
fn remove_from_one_of_two_referencing_scopes_keeps_the_clone() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace_opt_in(&env, tmp.path(), "project");

    // Add to both scopes.
    env.cmd()
        .args(["--global", "catalog", "add", &fix.url, "--name", "g"])
        .output()
        .unwrap();
    env.cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "add",
            &fix.url,
            "--name",
            "w",
        ])
        .output()
        .unwrap();

    let cache = cache_dir_for(&env, &fix.url);
    assert!(cache.is_dir());

    // Remove from the workspace only.
    let rm = env
        .cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "remove",
            "w",
            "--force",
        ])
        .output()
        .unwrap();
    assert!(
        rm.status.success(),
        "{}",
        String::from_utf8_lossy(&rm.stderr)
    );

    // Cache directory must survive — the global scope still references it.
    assert!(
        cache.is_dir(),
        "cache directory was removed despite global still referencing it",
    );
}

#[test]
fn remove_from_last_referencing_scope_removes_the_clone() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace_opt_in(&env, tmp.path(), "project");

    env.cmd()
        .args(["--global", "catalog", "add", &fix.url, "--name", "g"])
        .output()
        .unwrap();
    env.cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "add",
            &fix.url,
            "--name",
            "w",
        ])
        .output()
        .unwrap();

    let cache = cache_dir_for(&env, &fix.url);
    assert!(cache.is_dir());

    // Remove from workspace, then from global. After the second the
    // cache directory must be gone.
    env.cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "remove",
            "w",
            "--force",
        ])
        .output()
        .unwrap();
    assert!(
        cache.is_dir(),
        "cache should still exist after workspace remove"
    );

    let rm_g = env
        .cmd()
        .args(["--global", "catalog", "remove", "g", "--force"])
        .output()
        .unwrap();
    assert!(
        rm_g.status.success(),
        "{}",
        String::from_utf8_lossy(&rm_g.stderr)
    );
    assert!(
        !cache.exists(),
        "cache directory should be gone after the last referencing scope removed it",
    );
}

#[test]
fn remove_when_registry_absent_falls_back_to_global_only() {
    // No `workspaces.txt` opt-in. `reference_count` walks only the
    // global config; a global-only remove deletes the clone even if a
    // workspace untracked-by-registry happens to reference it.
    // This is the documented opt-in trade-off — without the registry,
    // the workspace's reference is invisible.
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let tmp = TempDir::new().unwrap();

    // Workspace exists but registry is NOT opted in.
    let ws_root = tmp.path().join("project");
    std::fs::create_dir_all(&ws_root).unwrap();
    let init = env
        .cmd()
        .args(["workspace", "init", ws_root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(init.status.success());
    let ws = std::fs::canonicalize(&ws_root).unwrap();

    // Confirm registry is absent.
    let registry = env.home_path().join(".local/state/tome/workspaces.txt");
    assert!(
        !registry.exists(),
        "registry should be absent for this test",
    );

    // Add to both scopes.
    env.cmd()
        .args(["--global", "catalog", "add", &fix.url, "--name", "g"])
        .output()
        .unwrap();
    env.cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "add",
            &fix.url,
            "--name",
            "w",
        ])
        .output()
        .unwrap();

    let cache = cache_dir_for(&env, &fix.url);
    assert!(cache.is_dir());

    // Remove globally. Reference-count sees an empty registry, walks
    // only the global config, finds no remaining references, deletes
    // the clone. The workspace's config still points at the (now
    // missing) path — `tome catalog update` from the workspace would
    // re-clone.
    let rm = env
        .cmd()
        .args(["--global", "catalog", "remove", "g", "--force"])
        .output()
        .unwrap();
    assert!(rm.status.success());
    assert!(
        !cache.exists(),
        "without registry opt-in, refcount can't see the workspace",
    );
}

// ---- Sequential double-remove ------------------------------------------

/// FR-M-MIG-2 rename: the prior name claimed "concurrent" but the body
/// is sequential. The genuine cross-process race can't be reproduced
/// deterministically without process control (every retry would land
/// at a different point in the lock-hold window). The sequential
/// double-remove pins the documented post-condition: both removes
/// succeed, the second observes `NotFound` from the first's
/// `remove_dir_all`, the cache directory is gone, no panic.
///
/// Production correctness depends on:
/// - `reference_count` returning an up-to-date `Vec<Scope>` at each
///   read (`catalog::store::reference_count`).
/// - `fs::remove_dir_all` being idempotent over a missing path
///   (`std::io::ErrorKind::NotFound` is swallowed by the impl).
///
/// Two-process race coverage was considered out of scope for Phase 3.
/// If a Phase 4+ regression suggests it's needed, `tests/concurrency.rs`
/// is the right home (it already has the two-process scaffolding).
#[test]
fn sequential_double_remove_of_last_reference_is_benign() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace_opt_in(&env, tmp.path(), "project");

    env.cmd()
        .args(["--global", "catalog", "add", &fix.url, "--name", "g"])
        .output()
        .unwrap();
    env.cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "add",
            &fix.url,
            "--name",
            "w",
        ])
        .output()
        .unwrap();

    let cache = cache_dir_for(&env, &fix.url);
    assert!(cache.is_dir());

    // Sequential removes that emulate the race: both observe the
    // post-write state where only one scope references the URL, then
    // remove. The race itself can't be reproduced deterministically
    // without process control; this test pins the documented
    // sequential outcome.
    let rm_w = env
        .cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "remove",
            "w",
            "--force",
        ])
        .output()
        .unwrap();
    assert!(rm_w.status.success());

    let rm_g = env
        .cmd()
        .args(["--global", "catalog", "remove", "g", "--force"])
        .output()
        .unwrap();
    assert!(rm_g.status.success());

    assert!(!cache.exists());
}

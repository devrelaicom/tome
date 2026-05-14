//! Integration tests for the Phase 3 additions to `Paths::resolve()` —
//! `state_dir`, `mcp_log`, `mcp_log_prev`, `workspace_registry` — plus the
//! Scope-aware accessor methods (`config_file_for`, `index_db_for`,
//! `index_lock_for`, `workspace_marker_dir`).
//!
//! `Paths::resolve()` reads `HOME` and the `XDG_*_HOME` vars, so this
//! suite runs single-threaded against a `Mutex` (mirrors the Phase 2
//! pattern in `tests/paths_phase2.rs`).

use std::path::PathBuf;
use std::sync::Mutex;

use tome::paths::Paths;
use tome::workspace::Scope;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct EnvGuard {
    keys: Vec<&'static str>,
    prior: Vec<(&'static str, Option<std::ffi::OsString>)>,
}

impl EnvGuard {
    fn set(keys_values: &[(&'static str, &str)]) -> Self {
        let prior = keys_values
            .iter()
            .map(|(k, _)| (*k, std::env::var_os(k)))
            .collect();
        for (k, v) in keys_values {
            // SAFETY: Tests guard env mutation behind ENV_LOCK; no other
            // threads observe the transient state.
            unsafe {
                std::env::set_var(k, v);
            }
        }
        Self {
            keys: keys_values.iter().map(|(k, _)| *k).collect(),
            prior,
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (i, key) in self.keys.iter().enumerate() {
            // SAFETY: under ENV_LOCK.
            unsafe {
                match self.prior.get(i).and_then(|(_, v)| v.clone()) {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

#[test]
fn resolve_places_state_paths_under_xdg_state_home() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[
        ("HOME", "/tmp/fake-home"),
        ("XDG_CONFIG_HOME", "/tmp/fake-cfg"),
        ("XDG_DATA_HOME", "/tmp/fake-data"),
        ("XDG_STATE_HOME", "/tmp/fake-state"),
    ]);

    let p = Paths::resolve().expect("resolve");
    assert_eq!(p.state_dir, PathBuf::from("/tmp/fake-state/tome"));
    assert_eq!(p.mcp_log, PathBuf::from("/tmp/fake-state/tome/mcp.log"));
    assert_eq!(
        p.mcp_log_prev,
        PathBuf::from("/tmp/fake-state/tome/mcp.log.1"),
    );
    assert_eq!(
        p.workspace_registry,
        PathBuf::from("/tmp/fake-state/tome/workspaces.txt"),
    );
}

#[test]
fn resolve_falls_back_to_default_xdg_state_home() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[
        ("HOME", "/tmp/fake-home"),
        ("XDG_CONFIG_HOME", ""),
        ("XDG_DATA_HOME", ""),
        ("XDG_STATE_HOME", ""),
    ]);

    let p = Paths::resolve().expect("resolve");
    assert_eq!(
        p.state_dir,
        PathBuf::from("/tmp/fake-home/.local/state/tome"),
    );
    assert_eq!(
        p.mcp_log,
        PathBuf::from("/tmp/fake-home/.local/state/tome/mcp.log"),
    );
}

#[test]
fn resolve_rejects_relative_xdg_state_home() {
    // Mirrors the Phase 1 rule: a relative `XDG_*_HOME` is ignored in
    // favour of the HOME-derived default. Confirms the same filter is
    // wired for `XDG_STATE_HOME`.
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[
        ("HOME", "/tmp/fake-home"),
        ("XDG_STATE_HOME", "relative/path"),
    ]);

    let p = Paths::resolve().expect("resolve");
    assert_eq!(
        p.state_dir,
        PathBuf::from("/tmp/fake-home/.local/state/tome"),
    );
}

#[test]
fn scope_global_accessors_match_resolved_fields() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[
        ("HOME", "/tmp/h"),
        ("XDG_CONFIG_HOME", "/tmp/cfg"),
        ("XDG_DATA_HOME", "/tmp/data"),
        ("XDG_STATE_HOME", "/tmp/state"),
    ]);

    let p = Paths::resolve().expect("resolve");
    assert_eq!(p.config_file_for(&Scope::Global), p.config_file);
    assert_eq!(p.index_db_for(&Scope::Global), p.index_db);
    assert_eq!(p.index_lock_for(&Scope::Global), p.index_lock);
}

#[test]
fn scope_workspace_accessors_route_into_dot_tome() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[("HOME", "/tmp/h"), ("XDG_DATA_HOME", "/tmp/d")]);

    let p = Paths::resolve().expect("resolve");
    let root = PathBuf::from("/tmp/proj");
    let ws = Scope::Workspace(root.clone());
    assert_eq!(p.config_file_for(&ws), root.join(".tome/config.toml"));
    assert_eq!(p.index_db_for(&ws), root.join(".tome/index.db"));
    assert_eq!(p.index_lock_for(&ws), root.join(".tome/index.lock"));
}

#[test]
fn workspace_marker_dir_is_dot_tome_under_root() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[("HOME", "/tmp/h"), ("XDG_DATA_HOME", "/tmp/d")]);

    let p = Paths::resolve().expect("resolve");
    let root = PathBuf::from("/abs/workspace");
    assert_eq!(
        p.workspace_marker_dir(&root),
        PathBuf::from("/abs/workspace/.tome")
    );
}

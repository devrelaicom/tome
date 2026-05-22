//! Phase 4 / F2a coverage for the new `Paths` accessors:
//!
//! - `<home>/.tome/logs/{mcp.log, mcp.log.1}` log paths
//! - `<home>/.tome/workspaces/<name>/{settings.toml, RULES.md}` workspace
//!   accessors
//! - `<project>/.tome/{config.toml, RULES.md}` project-marker associated
//!   functions
//!
//! Phase 3's `_for(&Scope)` accessors are gone — every read/write
//! operates on the single central paths. F11 will reintroduce
//! per-workspace catalog/skill isolation via the central DB's
//! `workspace_catalogs` / `workspace_skills` junction tables.

use std::path::PathBuf;
use std::sync::Mutex;

use tome::paths::Paths;
use tome::workspace::WorkspaceName;

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
fn resolve_places_log_paths_under_root_logs_dir() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[("HOME", "/tmp/fake-home")]);

    let p = Paths::resolve().expect("resolve");
    assert_eq!(p.logs_dir, PathBuf::from("/tmp/fake-home/.tome/logs"));
    assert_eq!(
        p.mcp_log,
        PathBuf::from("/tmp/fake-home/.tome/logs/mcp.log")
    );
    assert_eq!(
        p.mcp_log_prev,
        PathBuf::from("/tmp/fake-home/.tome/logs/mcp.log.1"),
    );
}

#[test]
fn resolve_places_workspaces_under_root() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[("HOME", "/tmp/h")]);

    let p = Paths::resolve().expect("resolve");
    assert_eq!(p.workspaces_dir, PathBuf::from("/tmp/h/.tome/workspaces"));
}

#[test]
fn workspace_accessors_route_under_workspaces_dir() {
    let _guard = ENV_LOCK.lock().unwrap();
    let _e = EnvGuard::set(&[("HOME", "/tmp/h")]);

    let p = Paths::resolve().expect("resolve");
    let name = WorkspaceName::global();
    assert_eq!(
        p.workspace_dir(&name),
        PathBuf::from("/tmp/h/.tome/workspaces/global"),
    );
    assert_eq!(
        p.workspace_settings_file(&name),
        PathBuf::from("/tmp/h/.tome/workspaces/global/settings.toml"),
    );
    assert_eq!(
        p.workspace_rules_file(&name),
        PathBuf::from("/tmp/h/.tome/workspaces/global/RULES.md"),
    );
}

#[test]
fn project_marker_accessors_are_independent_of_self() {
    let project = PathBuf::from("/abs/project");
    assert_eq!(
        Paths::project_marker_dir(&project),
        PathBuf::from("/abs/project/.tome"),
    );
    assert_eq!(
        Paths::project_marker_config(&project),
        PathBuf::from("/abs/project/.tome/config.toml"),
    );
    assert_eq!(
        Paths::project_marker_rules(&project),
        PathBuf::from("/abs/project/.tome/RULES.md"),
    );
}

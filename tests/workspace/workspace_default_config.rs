//! Task 8: `[workspace] default` in `~/.tome/config.toml` is consulted
//! between the `TOME_WORKSPACE` env and the project-marker walk.
//!
//! Tests run under a mutex (ENV_LOCK from workspace_resolution.rs
//! is local, so we keep our own) and reset env + CWD on drop.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::common::{paths_for, seed_workspace, ToolEnv};
use tome::cli::GlobalScopeArgs;
use tome::workspace::resolution::resolve;
use tome::workspace::ScopeSource;

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct Guard {
    _lock: std::sync::MutexGuard<'static, ()>,
    prior_env: Option<std::ffi::OsString>,
    prior_cwd: PathBuf,
}

impl Guard {
    fn new() -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior_env = std::env::var_os("TOME_WORKSPACE");
        let prior_cwd = std::env::current_dir().expect("cwd");
        // SAFETY: ENV_LOCK serialises
        unsafe { std::env::remove_var("TOME_WORKSPACE") };
        Self {
            _lock: lock,
            prior_env,
            prior_cwd,
        }
    }

    fn chdir(&self, p: &Path) {
        std::env::set_current_dir(p).expect("chdir");
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.prior_cwd).ok();
        // SAFETY: ENV_LOCK still held at drop
        unsafe {
            match &self.prior_env {
                Some(v) => std::env::set_var("TOME_WORKSPACE", v),
                None => std::env::remove_var("TOME_WORKSPACE"),
            }
        }
    }
}

fn no_flag() -> GlobalScopeArgs {
    GlobalScopeArgs::default()
}

/// `[workspace] default = "work"` in config.toml is picked up when no
/// --workspace flag or TOME_WORKSPACE env is set and no project marker is
/// present.
#[test]
fn config_default_workspace_used_without_flag_or_env() {
    let guard = Guard::new();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // Seed a "work" workspace so membership check passes.
    seed_workspace(&paths, "work");

    // Write [workspace] default = "work" to global config.toml
    std::fs::write(
        &paths.global_config_file,
        "[workspace]\ndefault = \"work\"\n",
    )
    .unwrap();

    // CWD is somewhere with NO .tome/config.toml marker.
    let isolated = tempfile::TempDir::new().unwrap();
    guard.chdir(isolated.path());

    let r = resolve(&no_flag(), &paths).expect("resolve");
    assert_eq!(r.scope.name().as_str(), "work");
    assert_eq!(r.source, ScopeSource::Config, "must be resolved from Config");
    assert!(r.project_root.is_none());
}

/// `--workspace` flag overrides `[workspace] default` in config.
#[test]
fn flag_overrides_config_default() {
    let guard = Guard::new();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    seed_workspace(&paths, "work");
    seed_workspace(&paths, "other");

    std::fs::write(
        &paths.global_config_file,
        "[workspace]\ndefault = \"work\"\n",
    )
    .unwrap();

    let isolated = tempfile::TempDir::new().unwrap();
    guard.chdir(isolated.path());

    let r = resolve(
        &GlobalScopeArgs {
            workspace: Some("other".to_owned()),
        },
        &paths,
    )
    .expect("resolve");
    assert_eq!(r.scope.name().as_str(), "other");
    assert_eq!(r.source, ScopeSource::Flag);
}

/// `TOME_WORKSPACE` env overrides `[workspace] default` in config.
#[test]
fn env_overrides_config_default() {
    let guard = Guard::new();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    seed_workspace(&paths, "work");
    seed_workspace(&paths, "from-env");

    std::fs::write(
        &paths.global_config_file,
        "[workspace]\ndefault = \"work\"\n",
    )
    .unwrap();

    // Set TOME_WORKSPACE to override.
    // SAFETY: ENV_LOCK held by Guard
    unsafe { std::env::set_var("TOME_WORKSPACE", "from-env") };

    let isolated = tempfile::TempDir::new().unwrap();
    guard.chdir(isolated.path());

    let r = resolve(&no_flag(), &paths).expect("resolve");
    assert_eq!(r.scope.name().as_str(), "from-env");
    assert_eq!(r.source, ScopeSource::Env);
}

/// An unknown `[workspace] default` surfaces `WorkspaceNotFound` (exit 13).
#[test]
fn config_default_unknown_workspace_returns_13() {
    let guard = Guard::new();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // Seed DB (so "ghost" fails the membership check, not the no-DB check)
    seed_workspace(&paths, "work");

    std::fs::write(
        &paths.global_config_file,
        "[workspace]\ndefault = \"ghost\"\n",
    )
    .unwrap();

    let isolated = tempfile::TempDir::new().unwrap();
    guard.chdir(isolated.path());

    let err = resolve(&no_flag(), &paths).expect_err("unknown workspace");
    assert_eq!(err.exit_code(), 13, "WorkspaceNotFound expected");
}

/// When `[workspace] default` is absent, fall through to global fallback.
#[test]
fn no_config_default_falls_through_to_global() {
    let guard = Guard::new();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // No config.toml written at all.

    let isolated = tempfile::TempDir::new().unwrap();
    guard.chdir(isolated.path());

    let r = resolve(&no_flag(), &paths).expect("resolve");
    assert!(r.scope.is_global());
    assert_eq!(r.source, ScopeSource::GlobalFallback);
}

//! `workspace::resolution::resolve` exercises every priority branch
//! plus the documented error modes. Resolution reads `TOME_WORKSPACE`
//! and the current working directory, so each test mutates global
//! process state — serialise via a `Mutex` (mirrors the
//! `tests/paths_phase{2,3}.rs` pattern).

use std::path::PathBuf;
use std::sync::Mutex;

use tempfile::TempDir;
use tome::cli::GlobalScopeArgs;
use tome::error::TomeError;
use tome::workspace::resolution::resolve;
use tome::workspace::{Scope, ScopeSource};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// RAII guard that restores `TOME_WORKSPACE` + CWD on drop. Holds the
/// process-wide env mutex; only one instance can exist at a time.
struct ResolveEnv {
    _guard: std::sync::MutexGuard<'static, ()>,
    prior_env: Option<std::ffi::OsString>,
    prior_cwd: PathBuf,
}

impl ResolveEnv {
    fn new() -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior_env = std::env::var_os("TOME_WORKSPACE");
        let prior_cwd = std::env::current_dir().expect("cwd");
        // SAFETY: protected by ENV_LOCK; no other threads observe the
        // transient state.
        unsafe {
            std::env::remove_var("TOME_WORKSPACE");
        }
        Self {
            _guard: guard,
            prior_env,
            prior_cwd,
        }
    }

    fn set_env(&self, value: &str) {
        // SAFETY: under ENV_LOCK.
        unsafe {
            std::env::set_var("TOME_WORKSPACE", value);
        }
    }

    fn unset_env(&self) {
        // SAFETY: under ENV_LOCK.
        unsafe {
            std::env::remove_var("TOME_WORKSPACE");
        }
    }

    fn chdir(&self, to: &std::path::Path) {
        std::env::set_current_dir(to).expect("chdir");
    }
}

impl Drop for ResolveEnv {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.prior_cwd).ok();
        // SAFETY: under ENV_LOCK.
        unsafe {
            match &self.prior_env {
                Some(v) => std::env::set_var("TOME_WORKSPACE", v),
                None => std::env::remove_var("TOME_WORKSPACE"),
            }
        }
    }
}

fn args_default() -> GlobalScopeArgs {
    GlobalScopeArgs::default()
}

fn make_workspace(tmp: &TempDir, name: &str) -> PathBuf {
    let root = tmp.path().join(name);
    std::fs::create_dir_all(root.join(".tome")).expect("mkdir .tome");
    // Canonicalise to the same shape `resolve` will return.
    std::fs::canonicalize(&root).expect("canonicalise")
}

#[test]
fn workspace_flag_takes_priority() {
    let env = ResolveEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace(&tmp, "flag-wins");

    // Even with the env var set to a (different) valid workspace and
    // CWD inside yet another workspace, the flag must win.
    let env_ws = make_workspace(&tmp, "env-ws");
    let cwd_ws = make_workspace(&tmp, "cwd-ws");
    env.set_env(env_ws.to_str().unwrap());
    env.chdir(&cwd_ws);

    let args = GlobalScopeArgs {
        workspace: Some(ws.clone()),
        global: false,
    };
    let r = resolve(&args).expect("resolve");
    assert_eq!(r.scope, Scope::Workspace(ws));
    assert_eq!(r.source, ScopeSource::Flag);
}

#[test]
fn global_flag_overrides_workspace_walk() {
    let env = ResolveEnv::new();
    let tmp = TempDir::new().unwrap();
    let cwd_ws = make_workspace(&tmp, "cwd-ws");
    env.chdir(&cwd_ws);

    let args = GlobalScopeArgs {
        workspace: None,
        global: true,
    };
    let r = resolve(&args).expect("resolve");
    assert_eq!(r.scope, Scope::Global);
    assert_eq!(r.source, ScopeSource::GlobalFlag);
}

#[test]
fn env_var_works_when_no_flag_set() {
    let env = ResolveEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace(&tmp, "env-source");
    env.set_env(ws.to_str().unwrap());

    let r = resolve(&args_default()).expect("resolve");
    assert_eq!(r.scope, Scope::Workspace(ws));
    assert_eq!(r.source, ScopeSource::Env);
}

#[test]
fn cwd_walk_finds_marker_in_parent() {
    let env = ResolveEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace(&tmp, "cwd-walk");
    let nested = ws.join("a/b/c");
    std::fs::create_dir_all(&nested).unwrap();
    env.chdir(&nested);

    let r = resolve(&args_default()).expect("resolve");
    assert_eq!(r.scope, Scope::Workspace(ws));
    assert_eq!(r.source, ScopeSource::CwdWalk);
}

#[test]
fn falls_back_to_global_when_nothing_set() {
    let env = ResolveEnv::new();
    let tmp = TempDir::new().unwrap();
    // CWD is inside `tmp` but `tmp` has no `.tome/` and its parents
    // shouldn't either (the TempDir root is a fresh dir under /tmp).
    env.unset_env();
    env.chdir(tmp.path());

    let r = resolve(&args_default()).expect("resolve");
    assert_eq!(r.scope, Scope::Global);
    assert_eq!(r.source, ScopeSource::GlobalFallback);
}

#[test]
fn conflict_returns_exit_72() {
    let env = ResolveEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace(&tmp, "conflict-ws");
    env.chdir(tmp.path()); // CWD has no `.tome/`, isolates the test

    let args = GlobalScopeArgs {
        workspace: Some(ws),
        global: true,
    };
    let err = resolve(&args).expect_err("expected conflict");
    assert!(matches!(err, TomeError::WorkspaceConflict));
    assert_eq!(err.exit_code(), 72);
}

#[test]
fn env_var_pointing_nowhere_returns_exit_71() {
    let env = ResolveEnv::new();
    let tmp = TempDir::new().unwrap();
    let bogus = tmp.path().join("does-not-exist");
    env.set_env(bogus.to_str().unwrap());
    env.chdir(tmp.path());

    let err = resolve(&args_default()).expect_err("expected workspace not found");
    assert!(matches!(err, TomeError::WorkspaceNotFound { .. }));
    assert_eq!(err.exit_code(), 71);
}

#[test]
fn env_var_without_marker_returns_exit_71() {
    let env = ResolveEnv::new();
    let tmp = TempDir::new().unwrap();
    // Path exists but has no `.tome/` subdir.
    let bare = tmp.path().join("no-marker");
    std::fs::create_dir_all(&bare).unwrap();
    env.set_env(bare.to_str().unwrap());
    env.chdir(tmp.path());

    let err = resolve(&args_default()).expect_err("expected workspace not found");
    assert!(matches!(err, TomeError::WorkspaceNotFound { .. }));
    assert_eq!(err.exit_code(), 71);
}

#[test]
fn flag_pointing_nowhere_returns_exit_71() {
    let env = ResolveEnv::new();
    let tmp = TempDir::new().unwrap();
    env.chdir(tmp.path());

    let args = GlobalScopeArgs {
        workspace: Some(tmp.path().join("does-not-exist")),
        global: false,
    };
    let err = resolve(&args).expect_err("expected workspace not found");
    assert!(matches!(err, TomeError::WorkspaceNotFound { .. }));
    assert_eq!(err.exit_code(), 71);
}

#[test]
fn nested_workspace_wins_over_outer() {
    let env = ResolveEnv::new();
    let tmp = TempDir::new().unwrap();
    let outer = make_workspace(&tmp, "outer");
    let inner = outer.join("inner");
    std::fs::create_dir_all(inner.join(".tome")).unwrap();
    let inner_canon = std::fs::canonicalize(&inner).unwrap();
    env.chdir(&inner_canon);

    let r = resolve(&args_default()).expect("resolve");
    // First-hit-wins walking upward; the inner workspace dominates.
    assert_eq!(r.scope, Scope::Workspace(inner_canon));
    assert_eq!(r.source, ScopeSource::CwdWalk);
}

#[test]
fn empty_env_var_is_treated_as_unset() {
    let env = ResolveEnv::new();
    let tmp = TempDir::new().unwrap();
    env.set_env("");
    env.chdir(tmp.path());

    let r = resolve(&args_default()).expect("resolve");
    assert_eq!(r.scope, Scope::Global);
    assert_eq!(r.source, ScopeSource::GlobalFallback);
}

//! Phase 4 / F10: `workspace::resolution::resolve` against a central
//! `workspaces` registry. Each test bootstraps a fresh `Paths` rooted at
//! a `TempDir`, seeds the central DB with the workspace names the test
//! cares about, and exercises the resolver's flag → env → marker walk →
//! global-fallback ladder.
//!
//! Mutates `TOME_WORKSPACE` and CWD; serialised via an `ENV_LOCK` mutex
//! so concurrent tests in this binary don't race.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::common::{stub_embedder_seed, stub_reranker_seed, stub_summariser_seed};
use tempfile::TempDir;
use tome::cli::GlobalScopeArgs;
use tome::error::TomeError;
use tome::index::{OpenOptions, open};
use tome::paths::Paths;
use tome::workspace::resolution::resolve;
use tome::workspace::{ScopeSource, WorkspaceName};

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
        unsafe {
            std::env::set_var("TOME_WORKSPACE", value);
        }
    }

    fn chdir(&self, to: &Path) {
        std::env::set_current_dir(to).expect("chdir");
    }
}

impl Drop for ResolveEnv {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.prior_cwd).ok();
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

fn workspace_flag(name: &str) -> GlobalScopeArgs {
    GlobalScopeArgs {
        workspace: Some(name.to_owned()),
    }
}

/// Build a `Paths` rooted at a fresh `TempDir` and bootstrap the central
/// index DB with the supplied workspace names (in addition to the seeded
/// privileged `global`). The TempDir is returned so its lifetime ties to
/// the test.
fn fresh_paths_with_seeded_workspaces(extra: &[&str]) -> (TempDir, Paths) {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::from_root(tmp.path().to_path_buf());
    std::fs::create_dir_all(&paths.root).expect("mkdir root");
    let opts = OpenOptions {
        embedder: stub_embedder_seed(),
        reranker: stub_reranker_seed(),
        summariser: stub_summariser_seed(),
    };
    let conn = open(&paths.index_db, &opts).expect("bootstrap index db");
    for &name in extra {
        conn.execute(
            "INSERT INTO workspaces (name, created_at, last_used_at) VALUES (?1, ?2, ?2)",
            rusqlite::params![name, 1_700_000_000_i64],
        )
        .expect("seed workspace");
    }
    drop(conn);
    (tmp, paths)
}

/// Build a `Paths` rooted at a fresh `TempDir` WITHOUT bootstrapping the
/// central index DB. Exercises the privileged-default specialisation
/// (no DB on disk = only `global` is valid).
fn fresh_paths_no_db() -> (TempDir, Paths) {
    let tmp = TempDir::new().expect("tempdir");
    let paths = Paths::from_root(tmp.path().to_path_buf());
    (tmp, paths)
}

#[test]
fn flag_resolves_to_seeded_workspace() {
    let env = ResolveEnv::new();
    let (tmp, paths) = fresh_paths_with_seeded_workspaces(&["foo"]);
    env.chdir(tmp.path());

    let r = resolve(&workspace_flag("foo"), &paths).expect("resolve");
    assert_eq!(r.scope.name().as_str(), "foo");
    assert_eq!(r.source, ScopeSource::Flag);
    assert!(r.project_root.is_none());
}

#[test]
fn flag_with_unknown_workspace_returns_13() {
    let env = ResolveEnv::new();
    let (tmp, paths) = fresh_paths_with_seeded_workspaces(&[]);
    env.chdir(tmp.path());

    let err = resolve(&workspace_flag("ghost"), &paths).unwrap_err();
    match err {
        TomeError::WorkspaceNotFound { name } => assert_eq!(name, "ghost"),
        other => panic!("expected WorkspaceNotFound, got {other:?}"),
    }
}

#[test]
fn flag_global_with_seeded_db_resolves() {
    let env = ResolveEnv::new();
    let (tmp, paths) = fresh_paths_with_seeded_workspaces(&[]);
    env.chdir(tmp.path());

    let r = resolve(&workspace_flag("global"), &paths).expect("resolve global");
    assert!(r.scope.is_global());
    assert_eq!(r.source, ScopeSource::Flag);
}

#[test]
fn flag_global_when_db_missing_succeeds_via_privileged_default() {
    let env = ResolveEnv::new();
    let (tmp, paths) = fresh_paths_no_db();
    env.chdir(tmp.path());

    // Privileged default — even without an on-disk DB, `global` is valid.
    let r = resolve(&workspace_flag("global"), &paths).expect("resolve global");
    assert!(r.scope.is_global());
    assert_eq!(r.source, ScopeSource::Flag);
}

#[test]
fn flag_named_when_db_missing_returns_13() {
    let env = ResolveEnv::new();
    let (tmp, paths) = fresh_paths_no_db();
    env.chdir(tmp.path());

    // Non-`global` name + no central DB on disk → 13.
    let err = resolve(&workspace_flag("foo"), &paths).unwrap_err();
    assert!(matches!(err, TomeError::WorkspaceNotFound { .. }));
    assert_eq!(err.exit_code(), 13);
}

#[test]
fn env_var_works_when_no_flag_set() {
    let env = ResolveEnv::new();
    let (tmp, paths) = fresh_paths_with_seeded_workspaces(&["env-ws"]);
    env.chdir(tmp.path());
    env.set_env("env-ws");

    let r = resolve(&args_default(), &paths).expect("resolve");
    assert_eq!(r.scope.name().as_str(), "env-ws");
    assert_eq!(r.source, ScopeSource::Env);
}

#[test]
fn flag_overrides_env_var() {
    let env = ResolveEnv::new();
    let (tmp, paths) = fresh_paths_with_seeded_workspaces(&["from-env", "from-flag"]);
    env.chdir(tmp.path());
    env.set_env("from-env");

    let r = resolve(&workspace_flag("from-flag"), &paths).expect("resolve");
    assert_eq!(r.scope.name().as_str(), "from-flag");
    assert_eq!(r.source, ScopeSource::Flag);
}

#[test]
fn project_marker_resolves_with_project_root() {
    let env = ResolveEnv::new();
    let (_tmp, paths) = fresh_paths_with_seeded_workspaces(&["proj-ws"]);

    // Project lives somewhere OUTSIDE the Tome root (a real user project
    // would). Use a sibling of `tmp`.
    let project_parent = TempDir::new().unwrap();
    let project = project_parent.path().join("myproj");
    std::fs::create_dir_all(project.join(".tome")).unwrap();
    std::fs::write(
        project.join(".tome/config.toml"),
        "workspace = \"proj-ws\"\n",
    )
    .unwrap();
    let project_canon = project.canonicalize().unwrap();
    env.chdir(&project_canon);

    let r = resolve(&args_default(), &paths).expect("resolve");
    assert_eq!(r.scope.name().as_str(), "proj-ws");
    assert_eq!(r.source, ScopeSource::ProjectMarker);
    assert_eq!(r.project_root.as_deref(), Some(project_canon.as_path()));
}

#[test]
fn project_marker_malformed_returns_70() {
    let env = ResolveEnv::new();
    let (_tmp, paths) = fresh_paths_with_seeded_workspaces(&[]);

    let project_parent = TempDir::new().unwrap();
    let project = project_parent.path().join("broken");
    std::fs::create_dir_all(project.join(".tome")).unwrap();
    std::fs::write(
        project.join(".tome/config.toml"),
        // No `workspace` key; deserialiser-driven failure.
        "garbage = \"x\"\n",
    )
    .unwrap();
    env.chdir(&project.canonicalize().unwrap());

    let err = resolve(&args_default(), &paths).unwrap_err();
    match err {
        TomeError::WorkspaceMalformed { path, .. } => {
            assert!(path.ends_with(".tome/config.toml"), "{}", path.display());
        }
        other => panic!("expected WorkspaceMalformed, got {other:?}"),
    }
}

#[test]
fn project_marker_naming_unknown_workspace_returns_13() {
    let env = ResolveEnv::new();
    let (_tmp, paths) = fresh_paths_with_seeded_workspaces(&[]);

    let project_parent = TempDir::new().unwrap();
    let project = project_parent.path().join("good-shape");
    std::fs::create_dir_all(project.join(".tome")).unwrap();
    std::fs::write(project.join(".tome/config.toml"), "workspace = \"ghost\"\n").unwrap();
    env.chdir(&project.canonicalize().unwrap());

    let err = resolve(&args_default(), &paths).unwrap_err();
    match err {
        TomeError::WorkspaceNotFound { name } => assert_eq!(name, "ghost"),
        other => panic!("expected WorkspaceNotFound, got {other:?}"),
    }
}

#[test]
fn falls_back_to_global_when_nothing_set() {
    let env = ResolveEnv::new();
    let (_tmp, paths) = fresh_paths_with_seeded_workspaces(&[]);
    // CWD is in a fresh TempDir with no .tome/ in any ancestor.
    let isolated = TempDir::new().unwrap();
    env.chdir(isolated.path());

    let r = resolve(&args_default(), &paths).expect("resolve");
    assert!(r.scope.is_global());
    assert_eq!(r.source, ScopeSource::GlobalFallback);
    assert!(r.project_root.is_none());
}

#[test]
fn invalid_name_via_flag_returns_15() {
    let env = ResolveEnv::new();
    let (tmp, paths) = fresh_paths_with_seeded_workspaces(&[]);
    env.chdir(tmp.path());

    let err = resolve(&workspace_flag("!bad"), &paths).unwrap_err();
    assert!(matches!(err, TomeError::WorkspaceNameInvalid { .. }));
    assert_eq!(err.exit_code(), 15);
}

#[test]
fn invalid_name_via_env_returns_15() {
    let env = ResolveEnv::new();
    let (tmp, paths) = fresh_paths_with_seeded_workspaces(&[]);
    env.chdir(tmp.path());
    env.set_env("..");

    let err = resolve(&args_default(), &paths).unwrap_err();
    assert!(matches!(err, TomeError::WorkspaceNameInvalid { .. }));
    assert_eq!(err.exit_code(), 15);
}

#[test]
fn empty_env_var_is_treated_as_unset() {
    let env = ResolveEnv::new();
    let (_tmp, paths) = fresh_paths_with_seeded_workspaces(&[]);
    let isolated = TempDir::new().unwrap();
    env.chdir(isolated.path());
    env.set_env("");

    let r = resolve(&args_default(), &paths).expect("resolve");
    assert!(r.scope.is_global());
    assert_eq!(r.source, ScopeSource::GlobalFallback);
}

/// The privileged `global` constant matches both the workspace name on
/// disk and the resolver's fallback — round-trip sanity test.
#[test]
fn global_constant_round_trip() {
    let n = WorkspaceName::global();
    assert_eq!(n.as_str(), WorkspaceName::GLOBAL);
}

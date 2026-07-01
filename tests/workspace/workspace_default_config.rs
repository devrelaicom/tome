//! Task 8: `[workspace] default` in `~/.tome/config.toml` is consulted
//! between the `TOME_WORKSPACE` env and the project-marker walk.
//!
//! Tests run under a mutex (ENV_LOCK from workspace_resolution.rs
//! is local, so we keep our own) and reset env + CWD on drop.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::common::{ToolEnv, paths_for, seed_workspace};
use tome::cli::GlobalScopeArgs;
use tome::workspace::ScopeSource;
use tome::workspace::resolution::{resolve, resolve_lenient};

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
    assert_eq!(
        r.source,
        ScopeSource::Config,
        "must be resolved from Config"
    );
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

// ---- Issue #302: `[workspace] default` shadowing a project marker ---------

/// A `[workspace] default` win WITH a project marker present in the CWD ancestry
/// records the shadowed marker dir on `overridden_project_marker`. The
/// resolution RESULT is unchanged: scope stays the config default, `source`
/// stays `Config`, and `project_root` stays `None` (detection only).
#[test]
fn config_default_win_with_marker_records_overridden_marker() {
    let guard = Guard::new();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    seed_workspace(&paths, "work");
    std::fs::write(
        &paths.global_config_file,
        "[workspace]\ndefault = \"work\"\n",
    )
    .unwrap();

    // A project dir WITH its own `.tome/config.toml` marker. The marker is never
    // parsed on the Config-wins path (only `try_exists` runs), so its content is
    // irrelevant — but keep it a valid marker for realism.
    let project = tempfile::TempDir::new().unwrap();
    let marker_dir = project.path().join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(marker_dir.join("config.toml"), "workspace = \"proj\"\n").unwrap();
    guard.chdir(project.path());

    let r = resolve(&no_flag(), &paths).expect("resolve");

    // Result UNCHANGED — config default still wins.
    assert_eq!(r.scope.name().as_str(), "work");
    assert_eq!(r.source, ScopeSource::Config);
    assert!(
        r.project_root.is_none(),
        "project_root must stay None on a Config win (no behavior change)"
    );

    // Detection populated: the shadowed marker dir is recorded (canonicalized).
    let recorded = r
        .overridden_project_marker
        .as_deref()
        .expect("overridden_project_marker must be recorded when a marker is present");
    let expected = project
        .path()
        .canonicalize()
        .unwrap_or_else(|_| project.path().to_path_buf());
    assert_eq!(
        recorded, expected,
        "overridden_project_marker must be the project dir whose .tome/config.toml was shadowed"
    );
}

/// A `[workspace] default` win WITHOUT any project marker in the ancestry leaves
/// `overridden_project_marker` as `None` — nothing was shadowed, so no notice.
#[test]
fn config_default_win_without_marker_records_nothing() {
    let guard = Guard::new();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    seed_workspace(&paths, "work");
    std::fs::write(
        &paths.global_config_file,
        "[workspace]\ndefault = \"work\"\n",
    )
    .unwrap();

    // CWD with NO `.tome/config.toml` marker anywhere in the ancestry.
    let isolated = tempfile::TempDir::new().unwrap();
    guard.chdir(isolated.path());

    let r = resolve(&no_flag(), &paths).expect("resolve");
    assert_eq!(r.scope.name().as_str(), "work");
    assert_eq!(r.source, ScopeSource::Config);
    assert!(
        r.overridden_project_marker.is_none(),
        "no marker present → nothing shadowed → no notice"
    );
}

/// A project-marker-resolved run (no `[workspace] default` set) leaves
/// `overridden_project_marker` as `None` and `project_root` set as before —
/// the marker branch never records an override.
#[test]
fn project_marker_resolution_records_no_override() {
    let guard = Guard::new();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // The marker names workspace "proj"; seed it so membership passes.
    seed_workspace(&paths, "proj");
    // No `[workspace] default` written — resolution falls through to the marker.

    let project = tempfile::TempDir::new().unwrap();
    let marker_dir = project.path().join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(marker_dir.join("config.toml"), "workspace = \"proj\"\n").unwrap();
    guard.chdir(project.path());

    let r = resolve(&no_flag(), &paths).expect("resolve");
    assert_eq!(r.scope.name().as_str(), "proj");
    assert_eq!(r.source, ScopeSource::ProjectMarker);
    assert!(
        r.project_root.is_some(),
        "project_root set on a marker resolution"
    );
    assert!(
        r.overridden_project_marker.is_none(),
        "the marker branch never records an override"
    );
}

// ---- Issue #287: strict vs lenient resolution on a malformed config -------

/// STRICT `resolve` (every foreground command) propagates exit 5 when the
/// global config is malformed — the intended "fail loud" universal gate.
#[test]
fn strict_resolve_on_malformed_config_exits_5() {
    let guard = Guard::new();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // Unknown key inside a known section — a typo.
    std::fs::write(&paths.global_config_file, "[query]\nnope = 1\n").unwrap();

    let isolated = tempfile::TempDir::new().unwrap();
    guard.chdir(isolated.path());

    let err = resolve(&no_flag(), &paths).expect_err("malformed config must fail strict");
    assert_eq!(err.exit_code(), 5, "strict resolve must propagate exit 5");
}

/// LENIENT `resolve_lenient` (diagnostic commands only) tolerates a malformed
/// config: step 3 degrades to defaults and the resolver falls through to the
/// global fallback so doctor/status can still run and report the problem.
#[test]
fn lenient_resolve_on_malformed_config_falls_through_to_global() {
    let guard = Guard::new();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    std::fs::write(&paths.global_config_file, "[query]\nnope = 1\n").unwrap();

    let isolated = tempfile::TempDir::new().unwrap();
    guard.chdir(isolated.path());

    let r = resolve_lenient(&no_flag(), &paths).expect("lenient resolve must not fail");
    assert!(r.scope.is_global(), "degrades to global fallback");
    assert_eq!(r.source, ScopeSource::GlobalFallback);
}

/// A `--workspace` flag is honoured under lenient resolution too — leniency
/// only softens step 3 (`[workspace] default`); the higher-priority flag/env
/// inputs still win, exactly as under strict.
#[test]
fn lenient_resolve_still_honours_flag_over_malformed_config() {
    let guard = Guard::new();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "work");

    std::fs::write(&paths.global_config_file, "[query]\nnope = 1\n").unwrap();

    let isolated = tempfile::TempDir::new().unwrap();
    guard.chdir(isolated.path());

    let args = GlobalScopeArgs {
        workspace: Some("work".to_string()),
    };
    let r = resolve_lenient(&args, &paths).expect("flag resolves under lenient");
    assert_eq!(r.scope.name().as_str(), "work");
    assert_eq!(r.source, ScopeSource::Flag);
}

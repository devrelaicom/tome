//! Library-API tests for `tome harness use` — across all three scopes.
//! Each test installs a single-entry stub override (named `"stub"` so
//! the sync orchestrator's effective list recognises it) and asserts
//! that the correct settings file was edited.

mod common;

use std::sync::Mutex;

use common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::cli::{HarnessScopeArg, HarnessUseArgs};
use tome::commands::harness::use_;
use tome::harness::StubHarness;
use tome::output::Mode;
use tome::paths::Paths;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

fn make_resolved_scope(name: &str, project_root: Option<std::path::PathBuf>) -> ResolvedScope {
    let scope = Scope(WorkspaceName::parse(name).unwrap());
    let source = if project_root.is_some() {
        ScopeSource::ProjectMarker
    } else {
        ScopeSource::GlobalFallback
    };
    ResolvedScope {
        scope,
        source,
        project_root,
    }
}

#[test]
fn use_unknown_harness_errors_with_exit_18() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessUseArgs {
        name: "totally-not-a-harness".to_string(),
        scope: HarnessScopeArg::Global,
        force: false,
    };
    let scope = make_resolved_scope("global", None);
    let err = use_::run(args, &scope, &paths, Mode::Json).expect_err("unknown");
    assert_eq!(err.exit_code(), 18, "want HarnessNotSupported; got {err:?}");
}

#[test]
fn use_project_scope_without_project_errors_with_usage() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness)]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessUseArgs {
        name: "stub".to_string(),
        scope: HarnessScopeArg::Project,
        force: false,
    };
    let scope = make_resolved_scope("global", None);
    let err = use_::run(args, &scope, &paths, Mode::Json).expect_err("usage");
    assert_eq!(err.exit_code(), 2, "want Usage; got {err:?}");
}

#[test]
fn use_global_scope_writes_global_settings_file() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness)]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessUseArgs {
        name: "stub".to_string(),
        scope: HarnessScopeArg::Global,
        force: false,
    };
    let scope = make_resolved_scope("global", None);
    use_::run(args, &scope, &paths, Mode::Json).expect("use ok");

    let body = std::fs::read_to_string(&paths.global_settings_file).expect("global settings");
    assert!(
        body.contains("stub"),
        "global settings must include stub: {body}"
    );
}

#[test]
fn use_workspace_scope_writes_workspace_settings_file() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness)]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "demo");

    let args = HarnessUseArgs {
        name: "stub".to_string(),
        scope: HarnessScopeArg::Workspace,
        force: false,
    };
    let scope = make_resolved_scope("demo", None);
    use_::run(args, &scope, &paths, Mode::Json).expect("use ok");

    let ws_settings = paths.workspaces_dir.join("demo/settings.toml");
    let body = std::fs::read_to_string(&ws_settings).expect("workspace settings");
    assert!(
        body.contains("stub"),
        "workspace settings must include stub: {body}"
    );
}

#[test]
fn use_project_scope_writes_project_marker() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness)]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    // Build a project marker pointing at `global`.
    let project_dir = TempDir::new().unwrap();
    let marker_dir = project_dir.path().join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(marker_dir.join("config.toml"), "workspace = \"global\"\n").unwrap();

    // Pre-populate $HOME for any harness detect() calls during sync.
    let prev_home = std::env::var_os("HOME");
    unsafe {
        std::env::set_var("HOME", env.home_path());
    }

    let args = HarnessUseArgs {
        name: "stub".to_string(),
        scope: HarnessScopeArg::Project,
        force: false,
    };
    let scope = make_resolved_scope("global", Some(project_dir.path().to_path_buf()));
    let result = use_::run(args, &scope, &paths, Mode::Json);

    unsafe {
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
    }
    result.expect("use ok");

    let marker_body =
        std::fs::read_to_string(Paths::project_marker_config(project_dir.path())).unwrap();
    assert!(
        marker_body.contains("stub"),
        "project marker must include stub: {marker_body}"
    );
}

#[test]
fn use_idempotent_when_name_already_present_does_not_invoke_sync() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness)]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    // Pre-write global settings with stub already present.
    std::fs::write(&paths.global_settings_file, "harnesses = [\"stub\"]\n").unwrap();
    let mtime_before = std::fs::metadata(&paths.global_settings_file)
        .unwrap()
        .modified()
        .unwrap();

    // Sleep so mtime granularity can advance if a write happens.
    std::thread::sleep(std::time::Duration::from_millis(1100));

    let args = HarnessUseArgs {
        name: "stub".to_string(),
        scope: HarnessScopeArg::Global,
        force: false,
    };
    let scope = make_resolved_scope("global", None);
    use_::run(args, &scope, &paths, Mode::Json).expect("use ok");

    let mtime_after = std::fs::metadata(&paths.global_settings_file)
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "settings file must not be rewritten"
    );
}

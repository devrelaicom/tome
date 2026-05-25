//! Library-API tests for `tome harness remove` — mirror of `use`.

mod common;

use std::sync::Mutex;

use common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tome::cli::{HarnessRemoveArgs, HarnessScopeArg};
use tome::commands::harness::remove;
use tome::harness::StubHarness;
use tome::output::Mode;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

fn make_resolved_scope(name: &str) -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::parse(name).unwrap()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    }
}

#[test]
fn remove_from_empty_global_settings_is_noop() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness)]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessRemoveArgs {
        name: "stub".to_string(),
        scope: HarnessScopeArg::Global,
    };
    let scope = make_resolved_scope("global");
    remove::run(args, &scope, &paths, Mode::Json).expect("remove ok");

    // No settings file should have been created.
    assert!(
        !paths.global_settings_file.exists(),
        "no-op remove must not create the settings file",
    );
}

#[test]
fn remove_existing_entry_from_global_drops_it() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness)]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.
    std::fs::write(
        &paths.global_settings_file,
        "harnesses = [\"stub\", \"other\"]\n",
    )
    .unwrap();

    let args = HarnessRemoveArgs {
        name: "stub".to_string(),
        scope: HarnessScopeArg::Global,
    };
    let scope = make_resolved_scope("global");
    remove::run(args, &scope, &paths, Mode::Json).expect("remove ok");

    let body = std::fs::read_to_string(&paths.global_settings_file).unwrap();
    assert!(!body.contains("\"stub\""), "stub must be gone: {body}");
    assert!(body.contains("other"), "other must remain: {body}");
}

#[test]
fn remove_last_entry_leaves_empty_array() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness)]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.
    std::fs::write(&paths.global_settings_file, "harnesses = [\"stub\"]\n").unwrap();

    let args = HarnessRemoveArgs {
        name: "stub".to_string(),
        scope: HarnessScopeArg::Global,
    };
    let scope = make_resolved_scope("global");
    remove::run(args, &scope, &paths, Mode::Json).expect("remove ok");

    let body = std::fs::read_to_string(&paths.global_settings_file).unwrap();
    assert!(body.contains("harnesses"), "key must remain: {body}");
    assert!(!body.contains("stub"), "name must be gone: {body}");
}

#[test]
fn remove_from_workspace_scope_writes_workspace_settings_file() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness)]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "demo");
    let ws_dir = paths.workspaces_dir.join("demo");
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(
        ws_dir.join("settings.toml"),
        "name = \"demo\"\nharnesses = [\"stub\"]\n",
    )
    .unwrap();

    let args = HarnessRemoveArgs {
        name: "stub".to_string(),
        scope: HarnessScopeArg::Workspace,
    };
    let scope = make_resolved_scope("demo");
    remove::run(args, &scope, &paths, Mode::Json).expect("remove ok");

    let body = std::fs::read_to_string(ws_dir.join("settings.toml")).unwrap();
    assert!(!body.contains("\"stub\""), "stub must be gone: {body}");
}

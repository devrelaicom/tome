//! Library-API tests for `tome harness remove` — mirror of `use`.

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tome::cli::{HarnessRemoveArgs, HarnessScopeArg};
use tome::commands::harness::remove;
use tome::harness::StubHarness;
use tome::output::Mode;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

fn make_resolved_scope(name: &str) -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::parse(name).unwrap()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    }
}

#[test]
fn remove_from_empty_global_settings_is_noop() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessRemoveArgs {
        name: "stub".to_string(),
        scope: Some(HarnessScopeArg::Global),
    };
    let scope = make_resolved_scope("global");
    remove::run(args, &scope, &paths, Mode::Json).expect("remove ok");

    // No config file should have been created (no-op).
    assert!(
        !paths.global_config_file.exists(),
        "no-op remove must not create the config file",
    );
}

#[test]
fn remove_existing_entry_from_global_drops_it() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // Task 2: global scope writes to config.toml [harness].enabled.
    std::fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"stub\", \"other\"]\n",
    )
    .unwrap();

    let args = HarnessRemoveArgs {
        name: "stub".to_string(),
        scope: Some(HarnessScopeArg::Global),
    };
    let scope = make_resolved_scope("global");
    remove::run(args, &scope, &paths, Mode::Json).expect("remove ok");

    let body = std::fs::read_to_string(&paths.global_config_file).unwrap();
    assert!(!body.contains("\"stub\""), "stub must be gone: {body}");
    assert!(body.contains("other"), "other must remain: {body}");
}

#[test]
fn remove_last_entry_leaves_empty_array() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // Task 2: global scope writes to config.toml [harness].enabled.
    std::fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"stub\"]\n",
    )
    .unwrap();

    let args = HarnessRemoveArgs {
        name: "stub".to_string(),
        scope: Some(HarnessScopeArg::Global),
    };
    let scope = make_resolved_scope("global");
    remove::run(args, &scope, &paths, Mode::Json).expect("remove ok");

    let body = std::fs::read_to_string(&paths.global_config_file).unwrap();
    assert!(body.contains("enabled"), "enabled key must remain: {body}");
    assert!(!body.contains("stub"), "name must be gone: {body}");

    // T-M6 (US3 review): parse the resulting TOML and assert the array
    // is empty. The substring assertion above is necessary but not
    // sufficient — a write that dropped the key entirely or replaced
    // the array with a different shape would still pass.
    // Task 2: parse as Config and access .harness.enabled
    let parsed: tome::config::Config = toml::from_str(&body).expect("config TOML must round-trip");
    assert_eq!(
        parsed.harness.enabled,
        Some(Vec::<String>::new()),
        "harness.enabled key must be present as an empty array",
    );
}

#[test]
fn remove_from_workspace_scope_writes_workspace_settings_file() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

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
        scope: Some(HarnessScopeArg::Workspace),
    };
    let scope = make_resolved_scope("demo");
    remove::run(args, &scope, &paths, Mode::Json).expect("remove ok");

    let body = std::fs::read_to_string(ws_dir.join("settings.toml")).unwrap();
    assert!(!body.contains("\"stub\""), "stub must be gone: {body}");
}

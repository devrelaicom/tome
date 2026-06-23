//! Task 9: `[harness] default_scope` in `~/.tome/config.toml` is used as the
//! default when `--scope` is not passed to `tome harness use` or `tome harness
//! remove`.

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for};
use tome::cli::{HarnessScopeArg, HarnessUseArgs};
use tome::commands::harness::use_;
use tome::harness::StubHarness;
use tome::output::Mode;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

fn global_scope() -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    }
}

/// When `--scope` is absent and `[harness] default_scope = "global"` in
/// config, the stub harness settings file at the GLOBAL path is written.
#[test]
fn config_default_scope_global_used_without_flag() {
    let _mutex = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // Write [harness] default_scope = "global" into global config.toml
    std::fs::write(
        &paths.global_config_file,
        "[harness]\ndefault_scope = \"global\"\n",
    )
    .unwrap();

    // No explicit --scope (None).
    let args = HarnessUseArgs {
        names: vec!["stub".to_string()],
        all: false,
        scope: None,
        force: false,
    };
    // Resolved scope has NO project_root → `project` scope would fail with exit 2.
    // If effective_harness_scope correctly returns Global, this succeeds.
    let scope = global_scope();
    let r = use_::run(args, &scope, &paths, Mode::Json).expect("should succeed");
    let _ = r;
}

/// Explicit `--scope project` overrides `[harness] default_scope = "global"`.
/// Since there is no project root this would fail (exit 2) — confirming the
/// explicit flag takes priority over the config default.
#[test]
fn explicit_scope_overrides_config_default() {
    let _mutex = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // Config says global, but we pass --scope project explicitly.
    std::fs::write(
        &paths.global_config_file,
        "[harness]\ndefault_scope = \"global\"\n",
    )
    .unwrap();

    let args = HarnessUseArgs {
        names: vec!["stub".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Project), // explicit
        force: false,
    };
    // No project root → project scope fails with exit 2 (Usage).
    let scope = global_scope();
    let err = use_::run(args, &scope, &paths, Mode::Json).expect_err("project with no root");
    assert_eq!(
        err.exit_code(),
        2,
        "explicit project scope overrides config global; exit 2 expected"
    );
}

/// When `--scope` is absent and NO `[harness] default_scope` is in config,
/// the hardcoded fallback `project` is used. Since there is no project root
/// this exits 2 (confirming Project is the fallback).
#[test]
fn no_config_default_scope_falls_back_to_project() {
    let _mutex = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // No config written → no [harness] default_scope
    let args = HarnessUseArgs {
        names: vec!["stub".to_string()],
        all: false,
        scope: None, // no explicit flag
        force: false,
    };
    let scope = global_scope();
    let err = use_::run(args, &scope, &paths, Mode::Json).expect_err("no project root");
    assert_eq!(
        err.exit_code(),
        2,
        "fallback project scope with no root should exit 2"
    );
}

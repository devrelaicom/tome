//! Task 9: `[harness] default_scope` in `~/.tome/config.toml` is used as the
//! default when `--scope` is not passed to `tome harness use` or `tome harness
//! remove`.

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for};
use tome::cli::{HarnessRemoveArgs, HarnessScopeArg, HarnessUseArgs};
use tome::commands::harness::{remove, use_};
use tome::harness::StubHarness;
use tome::output::Mode;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

fn global_scope() -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
        overridden_project_marker: None,
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
        include_opt_in: false,
        scope: None,
        force: false,
    };
    // Resolved scope has NO project_root → `project` scope would fail with exit 2.
    // If effective_harness_scope correctly returns Global, this succeeds.
    let scope = global_scope();
    use_::run(args, &scope, &paths, Mode::Json).expect("should succeed");
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
        include_opt_in: false,
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
        include_opt_in: false,
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

// ── harness remove also uses effective_harness_scope ─────────────────────────

/// T9 coverage gap: `harness remove` also routes through `effective_harness_scope`
/// but was previously untested for the config-driven default.
///
/// When `--scope` is absent and `[harness] default_scope = "global"` in config,
/// the remove command edits the GLOBAL config table (`[harness].enabled`), not
/// a project-scope settings file.
#[test]
fn config_default_scope_global_drives_harness_remove() {
    let _mutex = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // Seed the global harness list with "stub", and set default_scope = "global".
    std::fs::write(
        &paths.global_config_file,
        "[harness]\ndefault_scope = \"global\"\nenabled = [\"stub\"]\n",
    )
    .unwrap();

    // Remove with no --scope; should derive Global from config and edit config.toml.
    let args = HarnessRemoveArgs {
        names: vec!["stub".to_string()],
        all: false,
        scope: None, // no explicit flag — must come from config
    };
    let scope = global_scope();
    // No project_root → remove succeeds only if effective scope is Global, not Project.
    remove::run(args, &scope, &paths, Mode::Json).expect("remove should succeed with global scope");

    // Verify "stub" was removed from [harness].enabled in config.toml.
    let body = std::fs::read_to_string(&paths.global_config_file).unwrap();
    let parsed: tome::config::Config =
        toml::from_str(&body).expect("config.toml must remain valid TOML");
    let enabled = parsed.harness.enabled.unwrap_or_default();
    assert!(
        !enabled.iter().any(|h| h == "stub"),
        "stub must be absent from [harness].enabled after remove; got: {enabled:?}"
    );
}

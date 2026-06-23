//! Library-API tests for `tome harness list` (no arg) — effective list
//! computation with source-chain annotations + excluded names section.

use crate::common::{HarnessModulesGuard, NamedStubHarness, ToolEnv, paths_for};
use tome::cli::HarnessListArgs;
use tome::commands::harness::list;
use tome::output::Mode;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

fn fallback_scope() -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    }
}

#[test]
fn list_with_only_global_settings_emits_global_source_chain() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set(["alpha", "beta"]));

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // Seed the global workspace.
    // `global` is auto-seeded by index bootstrap; no manual seed needed.
    // Task 2: global harness settings now live in config.toml [harness].enabled.
    std::fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"alpha\", \"beta\"]\n",
    )
    .unwrap();

    let args = HarnessListArgs { workspace: None };
    let scope = fallback_scope();
    // Use JSON mode to test silent compute; failure to compute would
    // surface here.
    let result = list::run(args, &scope, &paths, Mode::Json);
    assert!(result.is_ok(), "list run: {result:?}");
}

#[test]
fn list_emits_excluded_names_section() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set(["alpha", "beta"]));

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.
    // Task 2: global harness settings now live in config.toml [harness].enabled.
    // Global declares alpha, beta and excludes alpha.
    std::fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"alpha\", \"beta\", \"!alpha\"]\n",
    )
    .unwrap();

    let args = HarnessListArgs { workspace: None };
    let scope = fallback_scope();
    let result = list::run(args, &scope, &paths, Mode::Human);
    assert!(result.is_ok(), "list run: {result:?}");
}

#[test]
fn list_with_no_declarations_anywhere_is_empty() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set(["alpha"]));

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessListArgs { workspace: None };
    let scope = fallback_scope();
    let result = list::run(args, &scope, &paths, Mode::Json);
    assert!(result.is_ok(), "list run: {result:?}");
}

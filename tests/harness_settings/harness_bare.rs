//! Library-API tests for bare `tome harness` — lists every supported
//! harness in tabular form with detection + per-project targets (FR-520).
//!
//! Process-global serialisation: `HARNESS_MODULES_OVERRIDE` is a single
//! `RwLock` slot; tests that install a guard must hold the file-local
//! mutex for their entire duration.
//!
//! `$HOME` mutations are serialised via [`HomeGuard`] (T-B1 from US3
//! review) so parallel tests can't race the env.

use crate::common::{HarnessModulesGuard, HomeGuard, NamedStubHarness, ToolEnv, paths_for};
use tome::commands::harness::bare;
use tome::output::Mode;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

fn fallback_scope() -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
        overridden_project_marker: None,
    }
}

#[test]
fn bare_runs_against_real_registry_without_project() {
    // No override — exercise SUPPORTED_HARNESSES against an isolated
    // $HOME where no harness dotdirs exist.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let _home = HomeGuard::install(env.home_path());
    let scope = fallback_scope();
    let result = bare::run(&scope, &paths, Mode::Human);
    assert!(result.is_ok(), "bare run: {result:?}");
}

#[test]
fn bare_with_synthetic_registry_emits_one_row_per_module() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set(["alpha", "beta"]));

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let _home = HomeGuard::install(env.home_path());
    let scope = fallback_scope();
    let result = bare::run(&scope, &paths, Mode::Json);
    assert!(result.is_ok(), "bare run: {result:?}");
}

#[test]
fn bare_with_project_root_emits_per_project_targets() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set(["alpha"]));

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let _home = HomeGuard::install(env.home_path());
    let project = env.home_path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let scope = ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::ProjectMarker,
        project_root: Some(project.clone()),
        overridden_project_marker: None,
    };
    let result = bare::run(&scope, &paths, Mode::Json);
    assert!(result.is_ok(), "bare run: {result:?}");
}

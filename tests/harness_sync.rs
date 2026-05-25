//! T-M5 (US3 review) — `tome harness sync` thin-wrapper behaviour.
//!
//! `tome harness sync` requires a resolved project root; absence is
//! exit 2 (Usage). Prior coverage tested the orchestrator's sync logic
//! via `harness::sync::sync_project` directly but never exercised the
//! CLI wrapper's project-required precondition.

mod common;

use common::{HomeGuard, ToolEnv, paths_for};
use tome::commands::harness::sync;
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
fn sync_without_project_marker_exits_with_usage_code_2() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let _home = HomeGuard::install(env.home_path());

    let scope = fallback_scope();
    let err = sync::run(&scope, &paths, Mode::Json).expect_err("missing project marker");
    assert_eq!(err.exit_code(), 2, "want Usage (2); got {err:?}");
}

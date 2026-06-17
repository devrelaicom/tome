//! Project-required precondition for the harness reconcile.
//!
//! The former `tome harness sync` thin wrapper was removed pre-launch;
//! its reconcile is now folded into the unified `tome sync` command.
//! This guards the same precondition the old wrapper enforced: with no
//! resolved project root (and without `--all`), `tome sync` exits 2
//! (Usage).

use crate::common::{HomeGuard, ToolEnv, paths_for};
use tome::cli::SyncArgs;
use tome::commands::sync;
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
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let _home = HomeGuard::install(env.home_path());

    let scope = fallback_scope();
    // No `--all`, no project marker → the harness reconcile has no project
    // to act on, so `tome sync` refuses with Usage (2).
    let args = SyncArgs {
        all: false,
        rules_only: false,
        harness_only: false,
        harness: None,
    };
    let err = sync::run(args, &scope, &paths, Mode::Json).expect_err("missing project marker");
    assert_eq!(err.exit_code(), 2, "want Usage (2); got {err:?}");
}

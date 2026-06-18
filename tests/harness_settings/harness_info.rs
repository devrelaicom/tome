//! Library-API tests for `tome harness info <name>`.

use crate::common::{HomeGuard, ToolEnv, paths_for};
use tome::cli::HarnessInfoArgs;
use tome::commands::harness::info;
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
fn info_for_unknown_harness_returns_exit_18() {
    // `info::run` reads the process-global harness-modules override slot;
    // serialise against the override-installing tests that share this binary.
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessInfoArgs {
        name: "not-a-real-harness".to_string(),
    };
    let scope = fallback_scope();
    let _home = HomeGuard::install(env.home_path());
    let err = info::run(args, &scope, &paths, Mode::Json).expect_err("unknown");
    assert_eq!(err.exit_code(), 18);
}

#[test]
fn info_for_real_harness_runs_without_project() {
    // `info::run` reads the process-global harness-modules override slot;
    // serialise against the override-installing tests that share this binary.
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessInfoArgs {
        name: "claude-code".to_string(),
    };
    let scope = fallback_scope();
    let _home = HomeGuard::install(env.home_path());
    let result = info::run(args, &scope, &paths, Mode::Json);
    assert!(result.is_ok(), "info run: {result:?}");
}

#[test]
fn info_reports_direct_scope_when_global_declares() {
    // `info::run` reads the process-global harness-modules override slot;
    // serialise against the override-installing tests that share this binary.
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.
    std::fs::write(
        &paths.global_settings_file,
        "harnesses = [\"claude-code\"]\n",
    )
    .unwrap();

    let args = HarnessInfoArgs {
        name: "claude-code".to_string(),
    };
    let scope = fallback_scope();
    let _home = HomeGuard::install(env.home_path());
    let result = info::run(args, &scope, &paths, Mode::Human);
    assert!(result.is_ok(), "info run: {result:?}");
}

/// T063: `tome harness info jetbrains-ai` (a manual-only MCP harness) renders
/// the paste-able snippet path without error — for jetbrains-ai the snippet is
/// the primary recovery artifact. (Exact-byte snippet pins live in the
/// `mcp_config` unit tests; this exercises the `info::run` wiring end-to-end.)
#[test]
fn info_for_manual_only_harness_renders_snippet_path() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let scope = fallback_scope();
    let _home = HomeGuard::install(env.home_path());
    // Both modes exercise the snippet branch (Human prints it; Json serialises
    // the `mcp_snippet` field).
    assert!(
        info::run(
            HarnessInfoArgs {
                name: "jetbrains-ai".to_string(),
            },
            &scope,
            &paths,
            Mode::Human,
        )
        .is_ok()
    );
    assert!(
        info::run(
            HarnessInfoArgs {
                name: "jetbrains-ai".to_string(),
            },
            &scope,
            &paths,
            Mode::Json,
        )
        .is_ok()
    );
}

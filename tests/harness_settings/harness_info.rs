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

/// Phase 11 / US4 (M1): `tome harness info generic` / `generic-op` must resolve
/// the opt-in target via `lookup` and print its snippet, NOT error
/// `HarnessNotSupported` (exit 18). The opt-in targets live in `OPT_IN_TARGETS`,
/// not `SUPPORTED_HARNESSES` / the override slot, so `info::run`'s `lookup`
/// fallback is exercised against the REAL registry.
#[test]
fn info_for_opt_in_targets_resolves_via_lookup() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    let _home = HomeGuard::install(env.home_path());

    for name in ["generic", "generic-op"] {
        let args = HarnessInfoArgs {
            name: name.to_string(),
        };
        let scope = fallback_scope();
        let result = info::run(args, &scope, &paths, Mode::Json);
        assert!(
            result.is_ok(),
            "info {name} must resolve via lookup (not exit 18); got {result:?}",
        );
    }
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
    // Task 2: global harness settings now live in config.toml [harness].enabled.
    std::fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"claude-code\"]\n",
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

/// MINOR (US5 closeout): capture the Human-mode STDOUT bytes of
/// `tome harness info jetbrains-ai` (via the real CLI) and assert the
/// `MCP config — paste into …:` heading + the exact paste-able snippet (now
/// carrying `"env": {}` per M1). `info` is read-only — no project, no models.
#[test]
fn info_human_stdout_contains_paste_heading_and_snippet() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let out = env
        .cmd()
        .args(["harness", "info", "jetbrains-ai"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains("MCP config — paste into jetbrains-ai:"),
        "paste heading present; got:\n{s}",
    );
    // The snippet body — mcpServers shape with env:{} (the M1 fix).
    assert!(s.contains("\"mcpServers\""), "snippet present; got:\n{s}");
    assert!(
        s.contains("\"env\": {}"),
        "snippet carries env:{{}}; got:\n{s}"
    );
}

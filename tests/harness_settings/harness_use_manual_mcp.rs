//! Phase 11 / US5 (T064 + T066): manual-MCP notice path for `tome harness use`.
//!
//! Two harnesses carry an MCP-only notice:
//!
//! * `jetbrains-ai` — `mcp_manual_only` (no MCP file written). `use` configures
//!   the rules file and emits a paste-the-snippet notice; the command STILL
//!   SUCCEEDS (success-with-notice is scoped to MCP only).
//! * `pi` — writes its MCP file BUT emits a `pi install pi-mcp-adapter` notice
//!   (`unverified` until the adapter is present).
//!
//! The NEGATIVE test proves the MCP-only scoping (M6/FR-011): when a
//! rules-file write FAILS during `use` of a manual-only harness, the command
//! STILL ERRORS with its normal exit code — the notice never converts a real
//! capability failure into a success.

use crate::common::{HomeGuard, ToolEnv, paths_for, seed_workspace};
use tome::cli::{HarnessScopeArg, HarnessUseArgs};
use tome::commands::harness::use_;
use tome::commands::harness::use_::{HarnessUseOutcome, HarnessUseReport, HarnessUseResult};
use tome::output::Mode;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

/// Extract the single successful outcome from a one-harness report.
fn single_ok(report: &HarnessUseReport) -> &HarnessUseOutcome {
    assert_eq!(report.results.len(), 1, "expected one harness result");
    match &report.results[0] {
        HarnessUseResult::Ok(o) => o,
        other => panic!("expected Ok outcome, got {other:?}"),
    }
}

fn make_project_scope(workspace: &str, project_root: std::path::PathBuf) -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::parse(workspace).unwrap()),
        source: ScopeSource::ProjectMarker,
        project_root: Some(project_root),
        overridden_project_marker: None,
    }
}

/// `compute_mcp_notice` (the pure notice builder `use` calls) returns the
/// paste-the-snippet notice for jetbrains-ai, the adapter notice for pi, and
/// `None` for a normal harness — without any filesystem setup.
#[test]
fn compute_mcp_notice_classifies_each_harness() {
    // Serialise against override-installing tests sharing this binary (the
    // notice reads the effective registry).
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    // jetbrains-ai: manual-only → paste-the-snippet notice carrying the EXACT
    // snippet bytes + the `tome harness info` pointer.
    let jb = use_::compute_mcp_notice("jetbrains-ai", "demo").expect("jetbrains notice");
    assert!(
        jb.contains("manually") && jb.contains("tome harness info jetbrains-ai"),
        "jetbrains notice must point at the manual path; got: {jb}",
    );
    // The embedded snippet carries the canonical args (workspace + harness).
    assert!(jb.contains("\"--workspace\""), "snippet present: {jb}");
    assert!(jb.contains("\"demo\""));
    assert!(jb.contains("\"jetbrains-ai\""));

    // pi: writes its file but needs the adapter.
    let pi = use_::compute_mcp_notice("pi", "demo").expect("pi notice");
    assert!(
        pi.contains("pi-mcp-adapter"),
        "pi notice must mention the adapter; got: {pi}",
    );

    // A normal harness has no MCP notice.
    assert!(use_::compute_mcp_notice("codex", "demo").is_none());
    assert!(use_::compute_mcp_notice("claude-code", "demo").is_none());
}

/// `tome harness use jetbrains-ai` (project scope) SUCCEEDS, writes its rules
/// file, and writes NO MCP file — the manual-MCP case is a notice, never an
/// error.
#[test]
fn use_jetbrains_ai_succeeds_writes_rules_no_mcp_file() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "demo");

    let project = env.home_path().join("project");
    let marker_dir = project.join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(marker_dir.join("config.toml"), "workspace = \"demo\"\n").unwrap();
    // A non-empty RULES.md so the rules sink has content to land.
    std::fs::write(marker_dir.join("RULES.md"), "# rules\n").unwrap();

    let _home = HomeGuard::install(env.home_path());

    let args = HarnessUseArgs {
        names: vec!["jetbrains-ai".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Project),
        force: false,
    };
    let scope = make_project_scope("demo", project.clone());
    // MUST succeed despite MCP being manual-only.
    use_::run(args, &scope, &paths, Mode::Json).expect("use jetbrains-ai ok");

    // Rules file (StandaloneFile under .aiassistant/) was written.
    let rules = project.join(".aiassistant/rules/tome.md");
    assert!(
        rules.is_file(),
        "jetbrains-ai rules file must be written: {}",
        rules.display(),
    );
    // NO MCP file was written (manual-only skips the MCP sink).
    let mcp = project.join(".aiassistant/mcp.json");
    assert!(
        !mcp.exists(),
        "manual-only harness must write no MCP file: {}",
        mcp.display(),
    );
}

/// NEGATIVE (M6/FR-011): a rules-file write FAILURE during `use` of the
/// manual-only jetbrains-ai harness STILL errors — the MCP-only
/// success-with-notice must not swallow a real capability failure.
///
/// Force the rules write to fail by making the rules-file's PARENT a symlink
/// (the SSOT write guard refuses a symlinked component), so `sync_project`
/// surfaces an IO error before the notice is ever reached.
#[cfg(unix)]
#[test]
fn use_jetbrains_ai_still_errors_when_rules_write_fails() {
    use std::os::unix::fs::symlink;

    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "demo");

    let project = env.home_path().join("project");
    let marker_dir = project.join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(marker_dir.join("config.toml"), "workspace = \"demo\"\n").unwrap();
    std::fs::write(marker_dir.join("RULES.md"), "# rules\n").unwrap();

    // jetbrains-ai's rules file lands at `.aiassistant/rules/tome.md`. Make the
    // `.aiassistant` component a symlink so the symlink-refusing write guard
    // rejects it → a real (non-MCP) capability failure.
    let real = env.home_path().join("elsewhere");
    std::fs::create_dir_all(&real).unwrap();
    symlink(&real, project.join(".aiassistant")).unwrap();

    let _home = HomeGuard::install(env.home_path());

    let args = HarnessUseArgs {
        names: vec!["jetbrains-ai".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Project),
        force: false,
    };
    let scope = make_project_scope("demo", project.clone());
    let err = use_::run(args, &scope, &paths, Mode::Json)
        .expect_err("rules write failure must error, not succeed-with-notice");
    // The IO/symlink-refusal exit code (7) — the normal failure code, NOT a
    // success. Manual-MCP scoping does not convert this into a notice.
    assert_eq!(
        err.exit_code(),
        7,
        "a rules-write failure during a manual-MCP `use` must still error; got {err:?}",
    );
}

/// `tome harness use pi` (project scope) SUCCEEDS, writes its MCP file, AND the
/// adapter notice is computed (`pi-mcp-adapter`).
#[test]
fn use_pi_writes_mcp_and_emits_adapter_notice() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "demo");

    let project = env.home_path().join("project");
    let marker_dir = project.join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(marker_dir.join("config.toml"), "workspace = \"demo\"\n").unwrap();
    std::fs::write(marker_dir.join("RULES.md"), "# rules\n").unwrap();

    let _home = HomeGuard::install(env.home_path());

    let args = HarnessUseArgs {
        names: vec!["pi".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Project),
        force: false,
    };
    let scope = make_project_scope("demo", project.clone());
    use_::run(args, &scope, &paths, Mode::Json).expect("use pi ok");

    // Pi writes its GLOBAL MCP config under home (`~/.pi/agent/mcp.json`).
    let mcp = env.home_path().join(".pi/agent/mcp.json");
    assert!(
        mcp.is_file(),
        "pi must write its MCP config: {}",
        mcp.display(),
    );
    // And the adapter notice is emitted.
    assert!(
        use_::compute_mcp_notice("pi", "demo")
            .unwrap()
            .contains("pi-mcp-adapter"),
    );
}

/// M2 (US5 closeout): drive the REAL `use_::run` emission path (via the
/// `run_inner` compute seam `run` itself wraps) for jetbrains-ai and assert the
/// EMITTED `HarnessUseOutcome.mcp_notice` is `Some(...)` carrying the snippet
/// pointer. This proves the `run → compute_mcp_notice → outcome` chain — a
/// regression that dropped `mcp_notice` from `run`'s outcome would fail HERE
/// even though `compute_mcp_notice` (asserted above) stayed correct.
#[test]
fn run_emits_mcp_notice_for_jetbrains_ai_and_pi() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "demo");

    let project = env.home_path().join("project");
    let marker_dir = project.join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(marker_dir.join("config.toml"), "workspace = \"demo\"\n").unwrap();
    std::fs::write(marker_dir.join("RULES.md"), "# rules\n").unwrap();

    let _home = HomeGuard::install(env.home_path());
    let scope = make_project_scope("demo", project.clone());

    // jetbrains-ai: the EMITTED outcome carries the paste-the-snippet notice
    // pointing at `tome harness info jetbrains-ai`.
    let jb_args = HarnessUseArgs {
        names: vec!["jetbrains-ai".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Project),
        force: false,
    };
    let (jb_report, jb_err) =
        use_::run_inner(jb_args, &scope, &paths).expect("use jetbrains-ai ok");
    assert!(jb_err.is_none(), "jetbrains-ai use must not fail");
    let jb_outcome = single_ok(&jb_report);
    let jb_notice = jb_outcome
        .mcp_notice
        .as_deref()
        .expect("run must emit an mcp_notice for jetbrains-ai");
    assert!(
        jb_notice.contains("manually") && jb_notice.contains("tome harness info jetbrains-ai"),
        "emitted notice must point at the manual snippet path; got: {jb_notice}",
    );
    // The embedded snippet (the recovery artifact) is present in the EMITTED
    // notice, not just the standalone helper.
    assert!(
        jb_notice.contains("\"mcpServers\""),
        "snippet present in emitted notice"
    );
    assert!(jb_notice.contains("\"--harness\"") && jb_notice.contains("\"jetbrains-ai\""));

    // pi: the EMITTED outcome carries the `pi-mcp-adapter` install notice.
    let pi_args = HarnessUseArgs {
        names: vec!["pi".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Project),
        force: false,
    };
    let (pi_report, pi_err) = use_::run_inner(pi_args, &scope, &paths).expect("use pi ok");
    assert!(pi_err.is_none(), "pi use must not fail");
    let pi_outcome = single_ok(&pi_report);
    let pi_notice = pi_outcome
        .mcp_notice
        .as_deref()
        .expect("run must emit an mcp_notice for pi");
    assert!(
        pi_notice.contains("pi-mcp-adapter"),
        "emitted pi notice must mention the adapter; got: {pi_notice}",
    );
}

/// MINOR (US5 closeout): capture the Human-mode STDOUT bytes of
/// `tome harness use jetbrains-ai` (via the real CLI) and assert the
/// `Note (MCP):` heading + the notice body (the manual-MCP guidance). Proves
/// the human-emit path prints the notice, not just the JSON field.
#[test]
fn use_human_stdout_contains_note_mcp_heading_and_body() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "demo");

    let project = env.home_path().join("project");
    let marker_dir = project.join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(marker_dir.join("config.toml"), "workspace = \"demo\"\n").unwrap();
    std::fs::write(marker_dir.join("RULES.md"), "# rules\n").unwrap();

    let out = env
        .cmd()
        .current_dir(&project)
        .args(["harness", "use", "jetbrains-ai"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains("Note (MCP):"),
        "Note (MCP) heading present; got:\n{s}",
    );
    assert!(
        s.contains("configures its MCP server manually"),
        "notice body present; got:\n{s}",
    );
}

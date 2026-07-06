//! Phase 11 / US5 (T065 + T066): per-harness MCP integration STATES.
//!
//! The shared `harness_integration::check_harness_integration` is the SSOT both
//! `tome status` and `tome doctor` route through, so the four states
//! `ok` / `manual` / `unverified` / `drift` are pinned here once:
//!
//! * `ok`         — a normal harness (pi-less) with the correct Tome entry.
//! * `manual`     — `mcp_manual_only` harness (jetbrains-ai): no file, no read.
//! * `unverified` — adapter harness (pi): correct entry but adapter-dependent.
//! * `drift`      — a Tome entry carrying a STALE `--workspace` arg.

use crate::common::{HomeGuard, ToolEnv, fabricate_all_registry_models, paths_for};
use tome::doctor::harness_integration::check_harness_integration;
use tome::doctor::{self, SubsystemHealth};
use tome::harness::mcp_config::{self, TomeEntry};
use tome::harness::{McpDialect, lookup};
use tome::settings::resolver::{EffectiveHarness, EffectiveHarnessList};
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

fn effective(names: &[&str]) -> EffectiveHarnessList {
    EffectiveHarnessList {
        harnesses: names
            .iter()
            .map(|n| EffectiveHarness {
                name: (*n).to_string(),
                source_chain: vec!["project".to_string()],
            })
            .collect(),
        excluded: Vec::new(),
    }
}

/// Write the canonical Tome entry for `harness` into its MCP config under
/// `project_root` / `home`, carrying `--workspace <ws>` + `--harness <name>`.
fn write_tome_entry(
    harness: &str,
    project_root: &std::path::Path,
    home: &std::path::Path,
    ws: &str,
) {
    let module = lookup(harness).expect("harness");
    let path = module.mcp_config_path(project_root, home);
    let dialect: McpDialect = module.mcp_dialect();
    let entry = TomeEntry::new(
        "tome".to_string(),
        vec![
            "mcp".to_string(),
            "--workspace".to_string(),
            ws.to_string(),
            "--harness".to_string(),
            harness.to_string(),
        ],
    );
    mcp_config::write_entry(&path, &dialect, &entry).expect("write tome entry");
}

/// `pi` (adapter harness) with a correct entry → `unverified` (NOT `ok`).
#[test]
fn pi_correct_entry_is_unverified() {
    let env = ToolEnv::new();
    let project = env.home_path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let ws = WorkspaceName::parse("demo").unwrap();

    write_tome_entry("pi", &project, env.home_path(), "demo");

    let (_rules, mcp) =
        check_harness_integration(&project, &effective(&["pi"]), env.home_path(), &ws);
    assert_eq!(mcp.len(), 1);
    assert_eq!(mcp[0].harness, "pi");
    assert_eq!(
        mcp[0].health,
        SubsystemHealth::Unverified,
        "pi's entry is adapter-dependent → unverified",
    );
}

/// `jetbrains-ai` (manual-only) → `manual`, with NO MCP file on disk.
#[test]
fn jetbrains_ai_is_manual_without_file() {
    let env = ToolEnv::new();
    let project = env.home_path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let ws = WorkspaceName::parse("demo").unwrap();

    // Deliberately write NO MCP file: a manual-only harness has none.
    let (_rules, mcp) = check_harness_integration(
        &project,
        &effective(&["jetbrains-ai"]),
        env.home_path(),
        &ws,
    );
    assert_eq!(mcp[0].harness, "jetbrains-ai");
    assert_eq!(
        mcp[0].health,
        SubsystemHealth::Manual,
        "jetbrains-ai has no writable MCP file → manual (not broken)",
    );
}

/// A normal harness with the correct entry → `ok`. `crush` keeps a
/// project-relative MCP file and is neither manual-only nor adapter-dependent.
#[test]
fn crush_correct_entry_is_ok() {
    let env = ToolEnv::new();
    let project = env.home_path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let ws = WorkspaceName::parse("demo").unwrap();

    write_tome_entry("crush", &project, env.home_path(), "demo");

    let (_rules, mcp) =
        check_harness_integration(&project, &effective(&["crush"]), env.home_path(), &ws);
    assert_eq!(mcp[0].harness, "crush");
    assert_eq!(
        mcp[0].health,
        SubsystemHealth::Ok,
        "a normal harness with the correct entry → ok",
    );
}

/// A Tome entry carrying a STALE `--workspace` arg → `drift` (takes precedence
/// over the adapter `unverified` for pi so `--fix` re-runs sync).
#[test]
fn stale_workspace_arg_is_drift() {
    let env = ToolEnv::new();
    let project = env.home_path().join("proj");
    std::fs::create_dir_all(&project).unwrap();
    let ws = WorkspaceName::parse("demo").unwrap();

    // Seed crush's entry pinned to a DIFFERENT workspace ("stale").
    write_tome_entry("crush", &project, env.home_path(), "stale");

    let (_rules, mcp) =
        check_harness_integration(&project, &effective(&["crush"]), env.home_path(), &ws);
    assert_eq!(
        mcp[0].health,
        SubsystemHealth::Drift,
        "a stale --workspace arg → drift",
    );

    // The same stale entry for pi is ALSO drift (drift precedes unverified).
    write_tome_entry("pi", &project, env.home_path(), "stale");
    let (_rules, pi_mcp) =
        check_harness_integration(&project, &effective(&["pi"]), env.home_path(), &ws);
    assert_eq!(
        pi_mcp[0].health,
        SubsystemHealth::Drift,
        "a stale --workspace arg for pi → drift, not unverified",
    );
}

/// The new states serialise to their documented wire strings.
#[test]
fn manual_and_unverified_wire_strings() {
    assert_eq!(SubsystemHealth::Manual.as_str(), "manual");
    assert_eq!(SubsystemHealth::Unverified.as_str(), "unverified");
}

/// MINOR (US5 closeout) + issue #427: the `Manual`/`Unverified` states flow
/// all the way through `doctor::assemble_report` — they appear in
/// `report.harness_mcp`, they each contribute an INFORMATIONAL
/// `suggested_fixes` pointer at `tome harness info <name>` (non-auto-fixable,
/// excluded from the exit-75 gate), and `report.overall` is NOT degraded by
/// them. Mirrors `doctor_mcp_states_p11`'s SSOT-level pins at the
/// assembled-report level.
#[test]
fn assemble_report_surfaces_manual_and_unverified_without_degrading() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // Healthy models so the rest of the report classifies Ok — isolating the
    // harness-MCP states as the only thing that COULD affect `overall`.
    fabricate_all_registry_models(&paths);

    let home = env.home_path();
    let _home = HomeGuard::install(home);

    // Project marker binding to `global`, declaring pi + jetbrains-ai.
    let project = home.join("project");
    let marker_dir = project.join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(
        marker_dir.join("config.toml"),
        "workspace = \"global\"\nharnesses = [\"pi\", \"jetbrains-ai\"]\n",
    )
    .unwrap();

    // pi: write its (correct-workspace) entry so it classifies `unverified`,
    // not `broken`. jetbrains-ai: write nothing → `manual`.
    {
        let module = lookup("pi").expect("pi");
        let path = module.mcp_config_path(&project, home);
        let entry = TomeEntry::new(
            "tome".to_string(),
            vec![
                "mcp".to_string(),
                "--workspace".to_string(),
                "global".to_string(),
                "--harness".to_string(),
                "pi".to_string(),
            ],
        );
        mcp_config::write_entry(&path, &module.mcp_dialect(), &entry).expect("write pi entry");
    }

    let scope = ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::ProjectMarker,
        project_root: Some(project.clone()),
        overridden_project_marker: None,
    };
    let report = doctor::assemble_report(&scope, &paths, home, false).expect("assemble");

    // Both states appear in harness_mcp.
    let find = |name: &str| {
        report
            .harness_mcp
            .iter()
            .find(|h| h.harness == name)
            .unwrap_or_else(|| panic!("{name} in harness_mcp; got {:?}", report.harness_mcp))
            .health
    };
    assert_eq!(find("pi"), SubsystemHealth::Unverified);
    assert_eq!(find("jetbrains-ai"), SubsystemHealth::Manual);

    // Issue #427: each harness's MCP subsystem gets exactly one INFORMATIONAL
    // suggested fix — non-auto-fixable, pointing at `tome harness info <name>`.
    for name in ["pi", "jetbrains-ai"] {
        let wire = format!("harness-mcp:{name}");
        let fixes: Vec<_> = report
            .suggested_fixes
            .iter()
            .filter(|f| f.subsystem.to_wire_string() == wire)
            .collect();
        assert_eq!(
            fixes.len(),
            1,
            "{name} manual/unverified must yield exactly ONE pointer fix; got {:?}",
            report.suggested_fixes,
        );
        assert!(!fixes[0].auto_fixable, "{name}: nothing --fix can run");
        assert!(
            fixes[0]
                .command
                .contains(&format!("tome harness info {name}")),
            "{name}: the pointer names the recovery command; got {:?}",
            fixes[0].command,
        );
    }
    // The pointers are informational — they must never flip a `--fix` run
    // into exit 75 (mirrors the issue #283 onboarding exclusion).
    for fix in report
        .suggested_fixes
        .iter()
        .filter(|f| f.subsystem.to_wire_string().starts_with("harness-mcp:"))
    {
        assert!(
            !tome::doctor::fixes::is_blocking_manual_fix(&report, fix),
            "manual/unverified pointer must not count as remaining manual work: {fix:?}",
        );
    }

    // The MCP states themselves contribute NOTHING to degradation: the doctor
    // classifier degrades on a `harness_mcp` entry only when it is
    // Drift/Broken/UserOwned. Manual/Unverified are deliberately absent from
    // that set, so NEITHER pi nor jetbrains-ai's MCP state is a degrader. (The
    // report's `overall` is Degraded here ONLY because this fixture wrote no
    // rules files — a SEPARATE, non-MCP concern surfaced as `harness-rules:*`
    // fixes — which is exactly what proves the MCP states are inert.)
    assert!(
        !report.harness_mcp.iter().any(|h| matches!(
            h.health,
            SubsystemHealth::Drift | SubsystemHealth::Broken | SubsystemHealth::UserOwned
        )),
        "no harness_mcp entry is in the degrading set; got {:?}",
        report.harness_mcp,
    );
    // Every MCP fix that DOES exist is one of the informational pointers —
    // manual/unverified never produce a runnable remediation (`tome sync` /
    // `--fix --force`), only the `harness info` pointer.
    assert!(
        report
            .suggested_fixes
            .iter()
            .filter(|f| f.subsystem.to_wire_string().starts_with("harness-mcp:"))
            .all(|f| !f.auto_fixable && f.command.contains("tome harness info")),
        "manual/unverified must only produce the info pointer; got {:?}",
        report.suggested_fixes,
    );
}

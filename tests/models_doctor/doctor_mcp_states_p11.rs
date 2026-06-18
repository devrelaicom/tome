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

use crate::common::ToolEnv;
use tome::doctor::SubsystemHealth;
use tome::doctor::harness_integration::check_harness_integration;
use tome::harness::mcp_config::{self, TomeEntry};
use tome::harness::{McpDialect, lookup};
use tome::settings::resolver::{EffectiveHarness, EffectiveHarnessList};
use tome::workspace::WorkspaceName;

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

//! FR-step coverage for the sync algorithm (FR-540 through FR-547).
//!
//! Each test names the FR it pins and exercises the relevant code path
//! against the `StubHarness` (plus an inline `OtherStubHarness` for the
//! multi-harness scenarios). Together with `harness_sync_stub.rs` (which
//! covers the higher-level binding-flow scenarios), these tests give
//! the sync orchestrator a complete coverage net.

use std::path::{Path, PathBuf};

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::sync::{self, Action, SyncDeps, SyncSubsystem};
use tome::harness::{
    BlockBodyStyle, HarnessModule, McpConfigFormat, RulesFileStrategy, StubHarness,
};
use tome::workspace::WorkspaceName;

/// Second synthetic harness for multi-harness scenarios. Writes a
/// different rules file and a different MCP config so we can prove
/// per-harness cleanup independence.
struct OtherStubHarness;

impl HarnessModule for OtherStubHarness {
    fn name(&self) -> &'static str {
        "other-stub"
    }
    fn description(&self) -> &'static str {
        "second deterministic test harness"
    }
    fn detect(&self, _home: &Path) -> bool {
        true
    }
    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("OTHER_STUB_RULES.md")
    }
    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }
    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::Inline
    }
    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join("other-stub.mcp.json")
    }
    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Json
    }
    fn mcp_parent_key(&self) -> &'static str {
        "mcpServers"
    }
}

/// Synthetic harness that targets the SAME rules-file path as
/// `StubHarness` (the dedup FR-482 / FR-483 scenarios).
struct SharingStubHarness;

impl HarnessModule for SharingStubHarness {
    fn name(&self) -> &'static str {
        "sharing-stub"
    }
    fn description(&self) -> &'static str {
        "synthetic harness sharing STUB_RULES.md with StubHarness"
    }
    fn detect(&self, _home: &Path) -> bool {
        true
    }
    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("STUB_RULES.md") // intentionally same as StubHarness
    }
    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }
    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::Inline
    }
    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        // Own MCP path so MCP-level removal can still be verified.
        project_root.join("sharing-stub.mcp.json")
    }
    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Json
    }
    fn mcp_parent_key(&self) -> &'static str {
        "mcpServers"
    }
}

struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

fn build_fixture(harnesses_toml: Option<&str>) -> Fixture {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).expect("create tome root");
    seed_workspace(&paths, "test-workspace");
    let workspace = WorkspaceName::parse("test-workspace").unwrap();

    let project = env.home_path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let marker_dir = project.join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    let mut body = "workspace = \"test-workspace\"\n".to_string();
    if let Some(h) = harnesses_toml {
        body.push_str(h);
        body.push('\n');
    }
    std::fs::write(marker_dir.join("config.toml"), body).unwrap();

    Fixture {
        _home: env.home,
        paths,
        project,
        workspace,
    }
}

fn deps_for<'a>(fx: &'a Fixture, force: bool) -> SyncDeps<'a> {
    SyncDeps {
        paths: &fx.paths,
        home_root: fx._home.path(),
        workspace_name: &fx.workspace,
        force,
        only_harness: None,
    }
}

// ---------------------------------------------------------------------------
// FR-540: effective list honours the project marker (priority walk).
// ---------------------------------------------------------------------------

#[test]
fn fr540_project_marker_wins_priority_walk() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let fx = build_fixture(Some("harnesses = [\"stub\"]"));

    // Add a *workspace* settings.toml declaring an empty list — the
    // priority walk must stop at the project marker, NOT see the
    // workspace's empty declaration as authoritative.
    let ws_dir = fx.paths.workspace_dir(&fx.workspace);
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(
        ws_dir.join("settings.toml"),
        "name = \"test-workspace\"\nharnesses = []\n",
    )
    .unwrap();

    let outcome = sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync");
    // Project marker declared "stub" → it's in the effective list.
    assert!(outcome.added.iter().any(|c| c.harness == "stub"));
}

// ---------------------------------------------------------------------------
// FR-541: per-harness consultation iterates over the registry view.
// ---------------------------------------------------------------------------

#[test]
fn fr541_decisions_cover_every_registered_harness() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(StubHarness::default()),
        Box::new(OtherStubHarness),
    ]);

    let fx = build_fixture(Some("harnesses = [\"stub\"]"));
    let outcome = sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync");

    let names: Vec<&str> = outcome
        .decisions
        .iter()
        .map(|d| d.harness.as_str())
        .collect();
    assert_eq!(names, vec!["stub", "other-stub"]);

    // Stub is live, other-stub isn't.
    let stub_decision = outcome
        .decisions
        .iter()
        .find(|d| d.harness == "stub")
        .unwrap();
    let other_decision = outcome
        .decisions
        .iter()
        .find(|d| d.harness == "other-stub")
        .unwrap();
    assert!(stub_decision.in_effective_list);
    assert!(!other_decision.in_effective_list);
}

// ---------------------------------------------------------------------------
// FR-542: ensure rules-file target exists with current Tome content.
// ---------------------------------------------------------------------------

#[test]
fn fr542_rules_file_created_on_first_run() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let fx = build_fixture(Some("harnesses = [\"stub\"]"));
    let outcome = sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync");

    let rules_path = fx.project.join("STUB_RULES.md");
    assert!(rules_path.is_file());
    assert!(
        outcome
            .added
            .iter()
            .any(|c| c.subsystem == SyncSubsystem::Rules)
    );
}

// ---------------------------------------------------------------------------
// FR-543: cleanup removes block when harness leaves the effective list.
// Multi-harness variant: install two, mark only one live; the other's
// block must be removed.
// ---------------------------------------------------------------------------

#[test]
fn fr543_cleanup_removes_block_for_dropped_harness() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(StubHarness::default()),
        Box::new(OtherStubHarness),
    ]);

    // First run: both harnesses live.
    let fx = build_fixture(Some("harnesses = [\"stub\", \"other-stub\"]"));
    sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync 1");

    let stub_rules = fx.project.join("STUB_RULES.md");
    let other_rules = fx.project.join("OTHER_STUB_RULES.md");
    assert!(stub_rules.is_file());
    assert!(other_rules.is_file());

    // Second run: drop other-stub.
    std::fs::write(
        fx.project.join(".tome/config.toml"),
        "workspace = \"test-workspace\"\nharnesses = [\"stub\"]\n",
    )
    .unwrap();

    let outcome = sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync 2");

    let other_body = std::fs::read_to_string(&other_rules).unwrap();
    assert!(
        !other_body.contains("<!-- tome:begin -->"),
        "other-stub's block must be removed; got: {other_body}",
    );
    assert!(
        outcome
            .removed
            .iter()
            .any(|c| c.harness == "other-stub" && c.subsystem == SyncSubsystem::Rules)
    );

    // Stub's block must still be present.
    let stub_body = std::fs::read_to_string(&stub_rules).unwrap();
    assert!(stub_body.contains("<!-- tome:begin -->"));
}

// ---------------------------------------------------------------------------
// FR-544: MCP entry ensure-current including stale-workspace-arg update.
// ---------------------------------------------------------------------------

#[test]
fn fr544_stale_workspace_arg_in_tome_owned_entry_is_updated() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let fx = build_fixture(Some("harnesses = [\"stub\"]"));

    // Pre-populate a stale Tome-owned entry.
    let mcp_path = fx.project.join("stub.mcp.json");
    let stale = serde_json::json!({
        "mcpServers": {
            "tome": {
                "command": "tome",
                "args": ["mcp", "--workspace", "stale-workspace"]
            }
        }
    });
    std::fs::write(&mcp_path, serde_json::to_string_pretty(&stale).unwrap()).unwrap();

    let outcome = sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync");

    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    let args = parsed["mcpServers"]["tome"]["args"].as_array().unwrap();
    assert_eq!(args[2], "test-workspace", "stale arg must be rewritten");

    assert!(
        outcome
            .updated
            .iter()
            .any(|c| c.subsystem == SyncSubsystem::Mcp)
    );
}

// ---------------------------------------------------------------------------
// FR-545: MCP entry cleanup for harnesses NOT in the effective list.
// ---------------------------------------------------------------------------

#[test]
fn fr545_mcp_entry_removed_when_harness_not_in_list() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let fx = build_fixture(Some("harnesses = [\"stub\"]"));

    // First run plants the MCP entry.
    sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync 1");
    let mcp_path = fx.project.join("stub.mcp.json");
    assert!(mcp_path.is_file());

    // Drop stub from the effective list and re-sync.
    std::fs::write(
        fx.project.join(".tome/config.toml"),
        "workspace = \"test-workspace\"\nharnesses = []\n",
    )
    .unwrap();
    sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync 2");

    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    assert!(
        parsed["mcpServers"].get("tome").is_none(),
        "tome MCP entry must be removed when harness drops from list",
    );
}

// ---------------------------------------------------------------------------
// FR-546: filesystem-state-as-source-of-truth — manual `rm` of the
// rules file is recovered on the next sync (no sidecar state).
// ---------------------------------------------------------------------------

#[test]
fn fr546_manual_delete_is_recovered_on_next_sync() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let fx = build_fixture(Some("harnesses = [\"stub\"]"));
    sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync 1");
    let rules_path = fx.project.join("STUB_RULES.md");
    assert!(rules_path.is_file());

    // Developer manually deletes the rules file.
    std::fs::remove_file(&rules_path).unwrap();

    let outcome = sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync 2");
    assert!(rules_path.is_file(), "rules file must be recreated");
    assert!(
        outcome
            .added
            .iter()
            .any(|c| c.subsystem == SyncSubsystem::Rules)
    );
}

// ---------------------------------------------------------------------------
// FR-547: SyncOutcome fields populated correctly.
// ---------------------------------------------------------------------------

#[test]
fn fr547_outcome_fields_populated_correctly() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let fx = build_fixture(Some("harnesses = [\"stub\"]"));

    // First sync: both subsystems added.
    let first = sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync 1");
    assert_eq!(first.added.len(), 2);
    assert!(first.updated.is_empty());
    assert!(first.removed.is_empty());
    assert_eq!(first.decisions.len(), 1);
    assert_eq!(first.decisions[0].rules_action, Action::Created);
    assert_eq!(first.decisions[0].mcp_action, Action::Created);

    // Second sync: idempotent → leave_alones increments.
    let second = sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync 2");
    assert!(second.added.is_empty());
    assert!(second.updated.is_empty());
    assert!(second.removed.is_empty());
    assert_eq!(second.leave_alones, 2);
    assert_eq!(second.decisions[0].rules_action, Action::LeftAlone);
    assert_eq!(second.decisions[0].mcp_action, Action::LeftAlone);
}

// ---------------------------------------------------------------------------
// FR-482: shared rules-file path written once across two live harnesses.
// ---------------------------------------------------------------------------

#[test]
fn fr482_shared_rules_path_written_once() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(StubHarness::default()),
        Box::new(SharingStubHarness),
    ]);

    let fx = build_fixture(Some("harnesses = [\"stub\", \"sharing-stub\"]"));
    let outcome = sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync");

    let rules_changes: Vec<_> = outcome
        .added
        .iter()
        .chain(outcome.updated.iter())
        .filter(|c| c.subsystem == SyncSubsystem::Rules)
        .collect();
    assert_eq!(
        rules_changes.len(),
        1,
        "shared rules path must produce exactly one rules change; got {rules_changes:?}",
    );
}

// ---------------------------------------------------------------------------
// FR-483: shared rules-file path retained when ONE of two sharing
// harnesses drops from the list.
// ---------------------------------------------------------------------------

#[test]
fn fr483_shared_rules_path_retained_while_any_sharer_is_live() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(StubHarness::default()),
        Box::new(SharingStubHarness),
    ]);

    let fx = build_fixture(Some("harnesses = [\"stub\", \"sharing-stub\"]"));
    sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync 1");
    let rules_path = fx.project.join("STUB_RULES.md");
    assert!(rules_path.is_file());

    // Drop sharing-stub but keep stub.
    std::fs::write(
        fx.project.join(".tome/config.toml"),
        "workspace = \"test-workspace\"\nharnesses = [\"stub\"]\n",
    )
    .unwrap();
    sync::sync_project(&fx.project, &deps_for(&fx, false)).expect("sync 2");

    let body = std::fs::read_to_string(&rules_path).unwrap();
    assert!(
        body.contains("<!-- tome:begin -->"),
        "rules block must survive while any sharer is live; got: {body}",
    );
}

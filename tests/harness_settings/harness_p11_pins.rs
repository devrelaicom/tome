//! Phase 11 (US1) — byte-stable pins for the 11 new harness modules.
//!
//! Two layers:
//!
//! 1. **Dialect byte pins** (unit level): each new harness's `mcp_dialect()`
//!    is fed through the public `mcp_config::write_entry` and the EXACT
//!    serialized bytes of the entry under its parent key are pinned. This
//!    covers every dialect shape: devin/kiro/junie/cline/antigravity/pi
//!    `mcpServers`, crush `mcp`+`type:stdio`, copilot `servers`+`type:stdio`,
//!    copilot-cli `mcpServers`+`type:local`+`tools`+`env`, zed
//!    `context_servers`. Round-trip + idempotent-rewrite is asserted too.
//!
//! 2. **Frontmatter/standalone byte pins** (unit level): kiro + jetbrains-ai
//!    produce a Tome-owned `---`-fenced header above the directive.
//!
//! 3. **Sync-level pins** (end-to-end via `sync_project` over the REAL
//!    modules, installed through `HarnessModulesGuard`): the shared
//!    `.github/copilot-instructions.md` single region across copilot +
//!    copilot-cli, and the jetbrains-ai "writes NO MCP file" gate.

use std::path::PathBuf;

use tempfile::TempDir;

use tome::harness::mcp_config::{self, TomeEntry};
use tome::harness::sync::{self, SyncDeps};
use tome::harness::{
    HarnessModule, antigravity::ANTIGRAVITY, cline::CLINE, copilot::COPILOT,
    copilot_cli::COPILOT_CLI, crush::CRUSH, devin::DEVIN, jetbrains_ai::JETBRAINS_AI, junie::JUNIE,
    kiro::KIRO, pi::PI, zed::ZED,
};
use tome::workspace::WorkspaceName;

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};

// ===========================================================================
// 1. Per-dialect MCP byte pins (unit level).
// ===========================================================================

/// The canonical Tome entry used in every pin (`<ws>` = "demo").
fn tome_entry() -> TomeEntry {
    TomeEntry::new(
        "tome".to_string(),
        vec![
            "mcp".to_string(),
            "--workspace".to_string(),
            "demo".to_string(),
        ],
    )
}

/// Write `module`'s dialect to a temp JSON file and return the bytes.
fn write_and_read(module: &dyn HarnessModule, file: &str) -> String {
    let tmp = TempDir::new().unwrap();
    let target = tmp.path().join(file);
    let dialect = module.mcp_dialect();
    mcp_config::write_entry(&target, &dialect, &tome_entry()).unwrap();
    // Idempotent second write — bytes must not change.
    mcp_config::write_entry(&target, &dialect, &tome_entry()).unwrap();
    let body = std::fs::read_to_string(&target).unwrap();
    // Round-trip: read back and confirm ownership.
    let read = mcp_config::read_entry(&target, &dialect).unwrap().unwrap();
    assert!(
        mcp_config::is_tome_owned(&read),
        "round-tripped {} entry must be Tome-owned",
        module.name(),
    );
    body
}

#[test]
fn devin_mcp_pins_exact_bytes() {
    // mcpServers + CommandArgs + emit_env (`"env": {}`).
    let body = write_and_read(&DEVIN, "config.json");
    assert_eq!(
        body,
        "{\n  \"mcpServers\": {\n    \"tome\": {\n      \"command\": \"tome\",\n      \"args\": [\n        \"mcp\",\n        \"--workspace\",\n        \"demo\"\n      ],\n      \"env\": {}\n    }\n  }\n}\n",
    );
}

#[test]
fn crush_mcp_pins_exact_bytes() {
    // mcp parent key + CommandArgs + per-entry type:stdio, NO env.
    let body = write_and_read(&CRUSH, "crush.json");
    assert_eq!(
        body,
        "{\n  \"mcp\": {\n    \"tome\": {\n      \"type\": \"stdio\",\n      \"command\": \"tome\",\n      \"args\": [\n        \"mcp\",\n        \"--workspace\",\n        \"demo\"\n      ]\n    }\n  }\n}\n",
    );
}

#[test]
fn copilot_cli_mcp_pins_exact_bytes() {
    // mcpServers + type:local + env:{} + tools:["*"].
    let body = write_and_read(&COPILOT_CLI, "mcp-config.json");
    assert_eq!(
        body,
        "{\n  \"mcpServers\": {\n    \"tome\": {\n      \"type\": \"local\",\n      \"command\": \"tome\",\n      \"args\": [\n        \"mcp\",\n        \"--workspace\",\n        \"demo\"\n      ],\n      \"env\": {},\n      \"tools\": [\n        \"*\"\n      ]\n    }\n  }\n}\n",
    );
}

#[test]
fn copilot_mcp_pins_exact_bytes() {
    // servers parent key + type:stdio, NO env.
    let body = write_and_read(&COPILOT, "mcp.json");
    assert_eq!(
        body,
        "{\n  \"servers\": {\n    \"tome\": {\n      \"type\": \"stdio\",\n      \"command\": \"tome\",\n      \"args\": [\n        \"mcp\",\n        \"--workspace\",\n        \"demo\"\n      ]\n    }\n  }\n}\n",
    );
}

#[test]
fn zed_mcp_pins_exact_bytes() {
    // context_servers parent key + CommandArgs + env:{}.
    let body = write_and_read(&ZED, "settings.json");
    assert_eq!(
        body,
        "{\n  \"context_servers\": {\n    \"tome\": {\n      \"command\": \"tome\",\n      \"args\": [\n        \"mcp\",\n        \"--workspace\",\n        \"demo\"\n      ],\n      \"env\": {}\n    }\n  }\n}\n",
    );
}

#[test]
fn mcpservers_emit_env_harnesses_pin_identical_bytes() {
    // kiro / junie / cline / antigravity / pi all share the exact same
    // dialect (mcpServers + CommandArgs + emit_env). Pin the shape once and
    // assert every one produces it byte-for-byte.
    const EXPECTED: &str = "{\n  \"mcpServers\": {\n    \"tome\": {\n      \"command\": \"tome\",\n      \"args\": [\n        \"mcp\",\n        \"--workspace\",\n        \"demo\"\n      ],\n      \"env\": {}\n    }\n  }\n}\n";
    let modules: &[&dyn HarnessModule] = &[&KIRO, &JUNIE, &CLINE, &ANTIGRAVITY, &PI];
    for m in modules {
        let body = write_and_read(*m, "mcp.json");
        assert_eq!(body, EXPECTED, "dialect bytes for {}", m.name());
    }
}

// ===========================================================================
// 2. Frontmatter / standalone byte pins (unit level).
// ===========================================================================

#[test]
fn kiro_standalone_frontmatter_pins_exact_bytes() {
    let tmp = TempDir::new().unwrap();
    let project = tmp.path();
    let target = KIRO.rules_file_target(project);
    let fm = KIRO.rules_frontmatter().expect("kiro has frontmatter");
    tome::harness::rules_file::write_standalone_with_frontmatter(&target, &fm, "the directive\n")
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "---\ninclusion: always\n---\nthe directive\n",
    );
    // Idempotent rewrite.
    tome::harness::rules_file::write_standalone_with_frontmatter(&target, &fm, "the directive\n")
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "---\ninclusion: always\n---\nthe directive\n",
    );
}

#[test]
fn jetbrains_standalone_frontmatter_pins_exact_bytes() {
    let tmp = TempDir::new().unwrap();
    let project = tmp.path();
    let target = JETBRAINS_AI.rules_file_target(project);
    let fm = JETBRAINS_AI
        .rules_frontmatter()
        .expect("jetbrains-ai has frontmatter");
    tome::harness::rules_file::write_standalone_with_frontmatter(&target, &fm, "the directive\n")
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "---\napply: always\n---\nthe directive\n",
    );
}

// ===========================================================================
// 3. Sync-level pins over the REAL modules.
// ===========================================================================

struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

/// Build a bound-project fixture declaring `harnesses_toml` in the marker and
/// seeding a known `.tome/RULES.md` body so inline rules pins are stable.
fn build_fixture(harnesses_toml: &str) -> Fixture {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).expect("create tome root");
    seed_workspace(&paths, "test-workspace");
    let workspace = WorkspaceName::parse("test-workspace").unwrap();

    let project = env.home_path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let marker_dir = project.join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(
        marker_dir.join("config.toml"),
        format!("workspace = \"test-workspace\"\n{harnesses_toml}\n"),
    )
    .unwrap();
    std::fs::write(marker_dir.join("RULES.md"), "ROUTING DIRECTIVE BODY\n").unwrap();

    Fixture {
        _home: env.home,
        paths,
        project,
        workspace,
    }
}

fn deps_for<'a>(fx: &'a Fixture) -> SyncDeps<'a> {
    SyncDeps {
        paths: &fx.paths,
        home_root: fx._home.path(),
        workspace_name: &fx.workspace,
        force: false,
        only_harness: None,
    }
}

#[test]
fn copilot_and_copilot_cli_share_one_rules_region() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Install ONLY the two copilot modules so the shared-sink dedupe is
    // exercised in isolation.
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(tome::harness::copilot::Copilot),
        Box::new(tome::harness::copilot_cli::CopilotCli),
    ]);

    let fx = build_fixture("harnesses = [\"copilot\", \"copilot-cli\"]");
    sync::sync_project(&fx.project, &deps_for(&fx)).expect("sync");

    let shared = fx.project.join(".github/copilot-instructions.md");
    let body = std::fs::read_to_string(&shared).expect("shared rules file written");
    // Exactly ONE Tome region despite two harnesses targeting the file.
    assert_eq!(
        body.matches("<!-- tome:begin -->").count(),
        1,
        "shared sink must collapse to a single Tome region:\n{body}",
    );
    assert_eq!(body.matches("<!-- tome:end -->").count(), 1);
    // The inline body landed (both copilot harnesses are Inline).
    assert!(
        body.contains("ROUTING DIRECTIVE BODY"),
        "inline directive body present:\n{body}",
    );
}

#[test]
fn jetbrains_ai_writes_no_mcp_file() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard =
        HarnessModulesGuard::install(vec![Box::new(tome::harness::jetbrains_ai::JetbrainsAi)]);

    let fx = build_fixture("harnesses = [\"jetbrains-ai\"]");
    let outcome = sync::sync_project(&fx.project, &deps_for(&fx)).expect("sync");

    // The MCP sink path is never written for a manual-only harness.
    let mcp_path = JETBRAINS_AI.mcp_config_path(&fx.project, fx._home.path());
    assert!(
        !mcp_path.exists(),
        "manual-only harness must not write an MCP config file at {}",
        mcp_path.display(),
    );

    // The MCP decision is LeftAlone (skipped), and no MCP change recorded.
    let decision = outcome
        .decisions
        .iter()
        .find(|d| d.harness == "jetbrains-ai")
        .expect("jetbrains-ai decision present");
    assert_eq!(decision.mcp_action, sync::Action::LeftAlone);
    assert!(
        !outcome
            .added
            .iter()
            .chain(&outcome.updated)
            .chain(&outcome.removed)
            .any(|c| c.harness == "jetbrains-ai" && c.subsystem == sync::SyncSubsystem::Mcp),
        "no MCP SyncChange may be recorded for the manual-only harness",
    );

    // The rules-file sink STILL ran — manual-only governs only the MCP sink.
    let rules = JETBRAINS_AI.rules_file_target(&fx.project);
    assert!(
        rules.exists(),
        "manual-only harness still receives its standalone rules file at {}",
        rules.display(),
    );
}

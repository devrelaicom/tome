//! Library-API unit tests for the `claude-code` harness module
//! (Phase 4 / US1.c — T157).
//!
//! These tests target [`tome::harness::claude_code::CLAUDE_CODE`] in
//! isolation: no sync orchestrator, no `HARNESS_MODULES_OVERRIDE`
//! installation, no DB. Each `HarnessModule` trait method is exercised
//! against `TempDir`-rooted home / project directories so the assertions
//! are hermetic.
//!
//! Mirrors the per-harness §R-8 spec row in
//! `specs/004-phase-4-refactor-harnesses/research.md` line 136.

use std::fs;

use tempfile::TempDir;
use tome::harness::claude_code::CLAUDE_CODE;
use tome::harness::{BlockBodyStyle, HarnessModule, McpConfigFormat, RulesFileStrategy};

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

#[test]
fn name_is_claude_code() {
    assert_eq!(CLAUDE_CODE.name(), "claude-code");
}

// ---------------------------------------------------------------------------
// detect()
// ---------------------------------------------------------------------------

#[test]
fn detect_returns_false_when_claude_dir_missing() {
    let home = TempDir::new().unwrap();
    assert!(
        !CLAUDE_CODE.detect(home.path()),
        "no .claude/ in home → detect must be false"
    );
}

#[test]
fn detect_returns_true_when_claude_dir_exists() {
    let home = TempDir::new().unwrap();
    fs::create_dir(home.path().join(".claude")).unwrap();
    assert!(
        CLAUDE_CODE.detect(home.path()),
        ".claude/ exists in home → detect must be true"
    );
}

#[test]
fn detect_returns_false_when_claude_is_a_file() {
    // The contract is existence-as-directory: `home.join(".claude").is_dir()`.
    // A regular file at that path must NOT trigger detection.
    let home = TempDir::new().unwrap();
    fs::write(home.path().join(".claude"), b"not a dir").unwrap();
    assert!(
        !CLAUDE_CODE.detect(home.path()),
        ".claude as a file → detect must be false (is_dir() guard)"
    );
}

// ---------------------------------------------------------------------------
// rules_file_target() — precedence ladder per research §R-8.
// ---------------------------------------------------------------------------

#[test]
fn rules_file_target_falls_back_to_agents_md_when_no_files_exist() {
    let project = TempDir::new().unwrap();
    let target = CLAUDE_CODE.rules_file_target(project.path());
    assert_eq!(
        target,
        project.path().join("AGENTS.md"),
        "no candidate files → fallback to AGENTS.md"
    );
}

#[test]
fn rules_file_target_prefers_existing_agents_md_over_claude_md() {
    let project = TempDir::new().unwrap();
    fs::write(project.path().join("AGENTS.md"), b"").unwrap();
    fs::write(project.path().join("CLAUDE.md"), b"").unwrap();
    let target = CLAUDE_CODE.rules_file_target(project.path());
    assert_eq!(
        target,
        project.path().join("AGENTS.md"),
        "AGENTS.md wins over CLAUDE.md when both exist"
    );
}

#[test]
fn rules_file_target_uses_claude_md_when_agents_md_missing() {
    let project = TempDir::new().unwrap();
    fs::write(project.path().join("CLAUDE.md"), b"").unwrap();
    let target = CLAUDE_CODE.rules_file_target(project.path());
    assert_eq!(
        target,
        project.path().join("CLAUDE.md"),
        "no AGENTS.md, only CLAUDE.md → use CLAUDE.md"
    );
}

#[test]
fn rules_file_target_uses_nested_claude_md_as_last_resort() {
    let project = TempDir::new().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();
    fs::write(project.path().join(".claude/CLAUDE.md"), b"").unwrap();
    let target = CLAUDE_CODE.rules_file_target(project.path());
    assert_eq!(
        target,
        project.path().join(".claude/CLAUDE.md"),
        "only .claude/CLAUDE.md present → use the nested path"
    );
}

// ---------------------------------------------------------------------------
// Rules-file strategy + body style.
// ---------------------------------------------------------------------------

#[test]
fn rules_file_strategy_is_block_in_existing_file() {
    assert_eq!(
        CLAUDE_CODE.rules_file_strategy(),
        RulesFileStrategy::BlockInExistingFile,
    );
}

#[test]
fn block_body_style_is_at_include() {
    assert_eq!(CLAUDE_CODE.block_body_style(), BlockBodyStyle::AtInclude);
}

// ---------------------------------------------------------------------------
// MCP config.
// ---------------------------------------------------------------------------

#[test]
fn mcp_config_path_is_project_local() {
    let project = TempDir::new().unwrap();
    let home = TempDir::new().unwrap();
    let path = CLAUDE_CODE.mcp_config_path(project.path(), home.path());
    assert_eq!(
        path,
        project.path().join(".claude/settings.json"),
        "claude-code MCP config is per-project — home is ignored",
    );
}

#[test]
fn mcp_config_format_is_json() {
    assert_eq!(CLAUDE_CODE.mcp_config_format(), McpConfigFormat::Json);
}

#[test]
fn mcp_parent_key_is_mcp_servers_camel() {
    assert_eq!(CLAUDE_CODE.mcp_parent_key(), "mcpServers");
}

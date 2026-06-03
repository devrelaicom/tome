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
// rules_file_target() — corrected precedence ladder (Phase 6 / FR-020/021/022).
//
// AGENTS.md is NO LONGER a claude-code candidate: Claude Code does not
// natively read it. The candidate set is CLAUDE.md > .claude/CLAUDE.md.
// ---------------------------------------------------------------------------

#[test]
fn rules_file_target_falls_back_to_claude_md_when_no_files_exist() {
    let project = TempDir::new().unwrap();
    let target = CLAUDE_CODE.rules_file_target(project.path());
    assert_eq!(
        target,
        project.path().join("CLAUDE.md"),
        "no candidate files → fallback to CLAUDE.md (never AGENTS.md)"
    );
}

#[test]
fn rules_file_target_ignores_agents_md_uses_claude_md() {
    // Phase 6 correction: even with an AGENTS.md present, claude-code targets
    // CLAUDE.md — AGENTS.md is not in its candidate set.
    let project = TempDir::new().unwrap();
    fs::write(project.path().join("AGENTS.md"), b"").unwrap();
    fs::write(project.path().join("CLAUDE.md"), b"").unwrap();
    let target = CLAUDE_CODE.rules_file_target(project.path());
    assert_eq!(
        target,
        project.path().join("CLAUDE.md"),
        "CLAUDE.md wins; AGENTS.md is not a candidate"
    );
}

#[test]
fn rules_file_target_ignores_agents_md_falls_back_to_claude_md() {
    // Only AGENTS.md exists → claude-code STILL targets CLAUDE.md (creating
    // it), because AGENTS.md is not a candidate. This is the substance of the
    // correction (FR-021): a project's rules-include block must not land where
    // Claude Code cannot see it.
    let project = TempDir::new().unwrap();
    fs::write(project.path().join("AGENTS.md"), b"").unwrap();
    let target = CLAUDE_CODE.rules_file_target(project.path());
    assert_eq!(
        target,
        project.path().join("CLAUDE.md"),
        "AGENTS.md alone must NOT be selected — fall back to CLAUDE.md"
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

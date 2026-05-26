//! `claude-code` — Anthropic's Claude Code CLI.
//!
//! Per research §R-8:
//!
//! - Per-user dir: `~/.claude/`.
//! - Rules-file target: `AGENTS.md` > `CLAUDE.md` > `.claude/CLAUDE.md`
//!   (first existing wins; falls back to `AGENTS.md` if none exist).
//! - Strategy: `BlockInExistingFile`, body style `AtInclude`.
//! - MCP config: `<project>/.claude/settings.json` (per-project).
//! - Parent key: `"mcpServers"`.

use std::path::{Path, PathBuf};

use crate::harness::{BlockBodyStyle, HarnessModule, McpConfigFormat, RulesFileStrategy};

/// Unit struct implementing [`HarnessModule`] for Claude Code.
pub struct ClaudeCode;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const CLAUDE_CODE: ClaudeCode = ClaudeCode;

impl HarnessModule for ClaudeCode {
    fn name(&self) -> &'static str {
        "claude-code"
    }

    fn description(&self) -> &'static str {
        "Anthropic's Claude Code CLI"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".claude").is_dir()
    }

    fn detect_path(&self, home: &Path) -> PathBuf {
        // `name()` is "claude-code" but the per-user dir is `~/.claude/`
        // (no `-code` suffix). Override the default to keep the path
        // reported by `tome harness info`'s `detected_path` in lockstep
        // with what `detect` actually probes.
        home.join(".claude")
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        // Precedence: AGENTS.md > CLAUDE.md > .claude/CLAUDE.md. First
        // existing candidate wins; fall back to AGENTS.md if none exist
        // (the sync algorithm will create it on first write).
        for candidate in ["AGENTS.md", "CLAUDE.md", ".claude/CLAUDE.md"] {
            let p = project_root.join(candidate);
            if p.exists() {
                return p;
            }
        }
        project_root.join("AGENTS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::AtInclude
    }

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join(".claude/settings.json")
    }

    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Json
    }

    fn mcp_parent_key(&self) -> &'static str {
        "mcpServers"
    }
}

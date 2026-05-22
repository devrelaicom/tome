//! `codex` — OpenAI Codex CLI.
//!
//! Per research §R-8:
//!
//! - Per-user dir: `~/.codex/`.
//! - Rules-file target: `AGENTS.md` (Codex CLI only reads this file).
//! - Strategy: `BlockInExistingFile`, body style `AtInclude`.
//! - MCP config: `~/.codex/config.toml` (global; no per-project
//!   support).
//! - Parent key: `"mcp_servers"` — snake_case is the documented TOML
//!   convention here, distinct from the JSON harnesses' `"mcpServers"`.

use std::path::{Path, PathBuf};

use crate::harness::{BlockBodyStyle, HarnessModule, McpConfigFormat, RulesFileStrategy};

/// Unit struct implementing [`HarnessModule`] for Codex CLI.
pub struct Codex;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const CODEX: Codex = Codex;

impl HarnessModule for Codex {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn description(&self) -> &'static str {
        "OpenAI Codex CLI"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".codex").is_dir()
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("AGENTS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::AtInclude
    }

    fn mcp_config_path(&self, _project_root: &Path, home: &Path) -> PathBuf {
        home.join(".codex/config.toml")
    }

    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Toml
    }

    fn mcp_parent_key(&self) -> &'static str {
        "mcp_servers"
    }
}

//! `opencode` — OpenCode CLI.
//!
//! Per research §R-8 + `contracts/harness-modules.md`:
//!
//! - Per-user dir: `~/.opencode/`.
//! - Rules-file target: `AGENTS.md`.
//! - Strategy: `BlockInExistingFile`. Body style is `Inline` — OpenCode
//!   does not document `@`-include support, so the block holds the
//!   full rules content verbatim and the sync algorithm must rewrite
//!   the block on every summary regeneration.
//! - MCP config: `<project>/opencode.json` (per-project, no dot
//!   prefix).
//! - Parent key: `"mcpServers"`.

use std::path::{Path, PathBuf};

use crate::harness::{BlockBodyStyle, HarnessModule, McpConfigFormat, RulesFileStrategy};

/// Unit struct implementing [`HarnessModule`] for OpenCode CLI.
pub struct OpenCode;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const OPENCODE: OpenCode = OpenCode;

impl HarnessModule for OpenCode {
    fn name(&self) -> &'static str {
        "opencode"
    }

    fn description(&self) -> &'static str {
        "OpenCode CLI"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".opencode").is_dir()
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("AGENTS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::Inline
    }

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join("opencode.json")
    }

    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Json
    }

    fn mcp_parent_key(&self) -> &'static str {
        "mcpServers"
    }
}

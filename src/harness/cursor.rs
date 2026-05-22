//! `cursor` — Cursor IDE.
//!
//! Per research §R-8:
//!
//! - Per-user dir: `~/.cursor/`.
//! - Rules-file target: `<project>/.cursor/rules/TOME_SKILLS.md`
//!   (Tome-owned standalone file; no markers, no surrounding content).
//! - Strategy: `StandaloneFile`. `block_body_style()` is never
//!   consulted; the trait returns `Inline` as a harmless placeholder.
//! - MCP config: `<project>/.cursor/mcp.json` (per-project).
//! - Parent key: `"mcpServers"`.

use std::path::{Path, PathBuf};

use crate::harness::{BlockBodyStyle, HarnessModule, McpConfigFormat, RulesFileStrategy};

/// Unit struct implementing [`HarnessModule`] for Cursor.
pub struct Cursor;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const CURSOR: Cursor = Cursor;

impl HarnessModule for Cursor {
    fn name(&self) -> &'static str {
        "cursor"
    }

    fn description(&self) -> &'static str {
        "Cursor IDE"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".cursor").is_dir()
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join(".cursor/rules/TOME_SKILLS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::StandaloneFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        // Never consulted for `StandaloneFile`. Returning `Inline` is
        // documented as a harmless placeholder in the contract.
        BlockBodyStyle::Inline
    }

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join(".cursor/mcp.json")
    }

    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Json
    }

    fn mcp_parent_key(&self) -> &'static str {
        "mcpServers"
    }
}

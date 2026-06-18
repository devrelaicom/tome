//! `junie` — JetBrains Junie CLI.
//!
//! Phase 11 (US1). Baseline integration: rules-file + MCP dialect only.
//!
//! - Per-user dir: `~/.junie/` (the default `detect_path`).
//! - Rules-file target: `<project>/.junie/AGENTS.md`, `BlockInExistingFile` ·
//!   `Inline` (the trait default). A Junie-namespaced `AGENTS.md` (under
//!   `.junie/`) rather than the project-root one — Tome owns a delimited
//!   block inside it.
//! - MCP config: `<project>/.junie/mcp/mcp.json` (per-project).
//! - MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`,
//!   `emit_env:true` (`"env": {}`), no extra fields.

use std::path::{Path, PathBuf};

use crate::harness::{EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy};

/// Unit struct implementing [`HarnessModule`] for Junie.
pub struct Junie;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const JUNIE: Junie = Junie;

impl HarnessModule for Junie {
    fn name(&self) -> &'static str {
        "junie"
    }

    fn description(&self) -> &'static str {
        "JetBrains Junie"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".junie").is_dir()
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join(".junie/AGENTS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    // `block_body_style()` defaults to `Inline` (G3) — exactly Junie's body.

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join(".junie/mcp/mcp.json")
    }

    /// Junie's MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`,
    /// `emit_env:true` (`"env": {}`), no extra fields.
    fn mcp_dialect(&self) -> McpDialect {
        McpDialect {
            file_format: FileFormat::Json,
            parent_key: "mcpServers",
            entry_shape: EntryShape::CommandArgs,
            entry_type: None,
            emit_env: true,
            extra_fields: &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_paths() {
        assert_eq!(JUNIE.name(), "junie");
        assert_eq!(JUNIE.detect_path(Path::new("/h")), Path::new("/h/.junie"));
        assert_eq!(
            JUNIE.rules_file_target(Path::new("/proj")),
            Path::new("/proj/.junie/AGENTS.md"),
        );
        assert_eq!(
            JUNIE.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/proj/.junie/mcp/mcp.json"),
        );
    }

    #[test]
    fn dialect_is_mcpservers_command_args_emit_env() {
        let d = JUNIE.mcp_dialect();
        assert_eq!(d.parent_key, "mcpServers");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, None);
        assert!(d.emit_env);
        assert!(!JUNIE.mcp_manual_only());
    }
}

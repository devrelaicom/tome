//! `copilot` — GitHub Copilot in VS Code.
//!
//! Phase 11 (US1). Baseline integration: rules-file + MCP dialect only.
//!
//! - Per-user dir: `~/.vscode/` (the default name `copilot` does NOT match
//!   it, so `detect_path` is overridden).
//! - Rules-file target: `<project>/.github/copilot-instructions.md`,
//!   `BlockInExistingFile` · `Inline` (the trait default). SHARES this sink
//!   with the `copilot-cli` harness — the shared-sink single-region collapse
//!   writes exactly one Tome block.
//! - MCP config: `<project>/.vscode/mcp.json` (per-project).
//! - MCP dialect: JSON `servers` parent key + `CommandArgs`, `type:"stdio"`,
//!   omit-empty-`env`, no extra fields.

use std::path::{Path, PathBuf};

use crate::harness::{
    EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy, ServerType,
};

/// Unit struct implementing [`HarnessModule`] for GitHub Copilot (VS Code).
pub struct Copilot;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const COPILOT: Copilot = Copilot;

impl HarnessModule for Copilot {
    fn name(&self) -> &'static str {
        "copilot"
    }

    fn description(&self) -> &'static str {
        "GitHub Copilot (VS Code)"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".vscode").is_dir()
    }

    fn detect_path(&self, home: &Path) -> PathBuf {
        // `name()` is "copilot" but the per-user dir is `~/.vscode/`.
        home.join(".vscode")
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join(".github/copilot-instructions.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    // `block_body_style()` defaults to `Inline` (G3).

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join(".vscode/mcp.json")
    }

    /// Copilot (VS Code) MCP dialect: JSON `servers` parent key +
    /// `CommandArgs`, `type:"stdio"`, omit-empty-`env`, no extra fields.
    fn mcp_dialect(&self) -> McpDialect {
        McpDialect {
            file_format: FileFormat::Json,
            parent_key: "servers",
            entry_shape: EntryShape::CommandArgs,
            entry_type: Some(ServerType::Stdio),
            emit_env: false,
            extra_fields: &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_paths() {
        assert_eq!(COPILOT.name(), "copilot");
        assert_eq!(
            COPILOT.detect_path(Path::new("/h")),
            Path::new("/h/.vscode"),
        );
        assert_eq!(
            COPILOT.rules_file_target(Path::new("/proj")),
            Path::new("/proj/.github/copilot-instructions.md"),
        );
        assert_eq!(
            COPILOT.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/proj/.vscode/mcp.json"),
        );
    }

    #[test]
    fn dialect_is_servers_command_args_type_stdio() {
        let d = COPILOT.mcp_dialect();
        assert_eq!(d.parent_key, "servers");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, Some(ServerType::Stdio));
        assert!(!d.emit_env);
        assert!(d.extra_fields.is_empty());
        assert!(!COPILOT.mcp_manual_only());
    }

    #[test]
    fn shares_rules_sink_with_copilot_cli() {
        // The single-region dedupe relies on these two pointing at the same
        // file. Pin the equality so a future path edit to either is caught.
        assert_eq!(
            COPILOT.rules_file_target(Path::new("/proj")),
            crate::harness::copilot_cli::COPILOT_CLI.rules_file_target(Path::new("/proj")),
        );
    }
}

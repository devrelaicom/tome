//! `devin` — Cognition's Devin agent.
//!
//! Phase 11 (US1). Baseline integration: rules-file + MCP dialect only.
//! Session steering (the G2 `CommandHook` SessionStart entry) lands in US2.
//!
//! - Per-user dir: `~/.devin/` (the default `detect_path`).
//! - Rules-file target: `<project>/AGENTS.md`, `BlockInExistingFile` ·
//!   `Inline` (the trait default). Shares `AGENTS.md` with codex / gemini /
//!   opencode — the shared-sink single-region collapse handles co-ownership.
//! - MCP config: `<project>/.devin/config.json` (per-project).
//! - MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`, `emit_env:true`
//!   (`"env": {}` per the contract), no extra fields.

use std::path::{Path, PathBuf};

use crate::harness::{EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy};

/// Unit struct implementing [`HarnessModule`] for Devin.
pub struct Devin;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const DEVIN: Devin = Devin;

impl HarnessModule for Devin {
    fn name(&self) -> &'static str {
        "devin"
    }

    fn description(&self) -> &'static str {
        "Cognition Devin"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".devin").is_dir()
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("AGENTS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    // `block_body_style()` defaults to `Inline` (G3) — exactly Devin's body.

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join(".devin/config.json")
    }

    /// Devin's MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`
    /// discriminator, `emit_env:true` (the contract shows `"env": {}`), no
    /// extra fields.
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
        assert_eq!(DEVIN.name(), "devin");
        assert_eq!(DEVIN.detect_path(Path::new("/h")), Path::new("/h/.devin"),);
        assert_eq!(
            DEVIN.rules_file_target(Path::new("/proj")),
            Path::new("/proj/AGENTS.md"),
        );
        assert_eq!(
            DEVIN.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/proj/.devin/config.json"),
        );
    }

    #[test]
    fn dialect_is_mcpservers_command_args_emit_env() {
        let d = DEVIN.mcp_dialect();
        assert_eq!(d.file_format, FileFormat::Json);
        assert_eq!(d.parent_key, "mcpServers");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, None);
        assert!(d.emit_env);
        assert!(d.extra_fields.is_empty());
        assert!(!DEVIN.mcp_manual_only());
    }
}

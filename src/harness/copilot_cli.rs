//! `copilot-cli` — GitHub Copilot CLI.
//!
//! Phase 11 (US1). Baseline integration: rules-file + MCP dialect only.
//! Session steering (the G2 `CommandHook` SessionStart entry, flat
//! `additionalContext` envelope) lands in US2.
//!
//! - Per-user dir: `~/.copilot/` (the default name `copilot-cli` does NOT
//!   match it, so `detect_path` is overridden).
//! - Rules-file target: `<project>/.github/copilot-instructions.md`,
//!   `BlockInExistingFile` · `Inline` (the trait default). SHARES this sink
//!   with the `copilot` (VS Code) harness — the shared-sink single-region
//!   collapse writes exactly one Tome block.
//! - MCP config: `~/.copilot/mcp-config.json` (GLOBAL, under `home`).
//! - MCP dialect: JSON `mcpServers` + `CommandArgs`, `type:"local"`,
//!   `emit_env:true` (`"env": {}`), plus a mandated `tools: ["*"]` field.

use std::path::{Path, PathBuf};

use crate::harness::{
    EntryShape, ExtraField, ExtraValue, FileFormat, HarnessModule, McpDialect, RulesFileStrategy,
    ServerType,
};

/// Unit struct implementing [`HarnessModule`] for GitHub Copilot CLI.
pub struct CopilotCli;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const COPILOT_CLI: CopilotCli = CopilotCli;

impl HarnessModule for CopilotCli {
    fn name(&self) -> &'static str {
        "copilot-cli"
    }

    fn description(&self) -> &'static str {
        "GitHub Copilot CLI"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".copilot").is_dir()
    }

    fn detect_path(&self, home: &Path) -> PathBuf {
        // `name()` is "copilot-cli" but the per-user dir is `~/.copilot/`.
        home.join(".copilot")
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join(".github/copilot-instructions.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    // `block_body_style()` defaults to `Inline` (G3).

    fn mcp_config_path(&self, _project_root: &Path, home: &Path) -> PathBuf {
        // GLOBAL config under the per-user dir.
        home.join(".copilot/mcp-config.json")
    }

    /// Copilot CLI's MCP dialect: JSON `mcpServers` + `CommandArgs`,
    /// `type:"local"`, `emit_env:true` (`"env": {}`), and a mandated
    /// `tools: ["*"]` field (re-derived on every rewrite).
    fn mcp_dialect(&self) -> McpDialect {
        McpDialect {
            file_format: FileFormat::Json,
            parent_key: "mcpServers",
            entry_shape: EntryShape::CommandArgs,
            entry_type: Some(ServerType::Local),
            emit_env: true,
            extra_fields: &[ExtraField {
                key: "tools",
                value: ExtraValue::StringArray(&["*"]),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_paths() {
        assert_eq!(COPILOT_CLI.name(), "copilot-cli");
        assert_eq!(
            COPILOT_CLI.detect_path(Path::new("/h")),
            Path::new("/h/.copilot"),
        );
        assert_eq!(
            COPILOT_CLI.rules_file_target(Path::new("/proj")),
            Path::new("/proj/.github/copilot-instructions.md"),
        );
        // GLOBAL MCP path under home.
        assert_eq!(
            COPILOT_CLI.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/h/.copilot/mcp-config.json"),
        );
    }

    #[test]
    fn dialect_has_type_local_and_tools_extra() {
        let d = COPILOT_CLI.mcp_dialect();
        assert_eq!(d.parent_key, "mcpServers");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, Some(ServerType::Local));
        assert!(d.emit_env);
        assert_eq!(d.extra_fields.len(), 1);
        assert_eq!(d.extra_fields[0].key, "tools");
        assert_eq!(d.extra_fields[0].value, ExtraValue::StringArray(&["*"]));
        assert!(!COPILOT_CLI.mcp_manual_only());
    }
}

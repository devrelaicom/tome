//! `cline` — the Cline VS Code extension.
//!
//! Phase 11 (US1). Baseline integration: standalone rules file + MCP dialect.
//! Session steering (the G2 `TsPlugin` shim) lands in US2/US3.
//!
//! - Per-user dir: `~/.cline/` (the default `detect_path`).
//! - Rules-file sink: a `StandaloneFile` at `<project>/.clinerules/tome.md` —
//!   a dedicated namespaced file inside Cline's own `.clinerules/` rules dir,
//!   so Tome never clobbers a developer rule file.
//! - MCP config: `~/.cline/mcp.json` (GLOBAL, under `home`).
//! - MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`,
//!   `emit_env:true` (`"env": {}`), no extra fields.

use std::path::{Path, PathBuf};

use crate::harness::{EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy};

/// Unit struct implementing [`HarnessModule`] for Cline.
pub struct Cline;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const CLINE: Cline = Cline;

impl HarnessModule for Cline {
    fn name(&self) -> &'static str {
        "cline"
    }

    fn description(&self) -> &'static str {
        "Cline"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".cline").is_dir()
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        // Dedicated namespaced file inside Cline's own `.clinerules/` dir.
        project_root.join(".clinerules/tome.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::StandaloneFile
    }

    fn rules_namespaced_file(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".clinerules/tome.md"))
    }

    fn mcp_config_path(&self, _project_root: &Path, home: &Path) -> PathBuf {
        // GLOBAL config under the per-user dir.
        home.join(".cline/mcp.json")
    }

    /// Cline's MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`,
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
        assert_eq!(CLINE.name(), "cline");
        assert_eq!(CLINE.detect_path(Path::new("/h")), Path::new("/h/.cline"));
        assert_eq!(
            CLINE.rules_file_target(Path::new("/proj")),
            Path::new("/proj/.clinerules/tome.md"),
        );
        // GLOBAL MCP path under home.
        assert_eq!(
            CLINE.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/h/.cline/mcp.json"),
        );
    }

    #[test]
    fn standalone_namespaced_rules_file() {
        assert_eq!(
            CLINE.rules_file_strategy(),
            RulesFileStrategy::StandaloneFile
        );
        assert_eq!(
            CLINE.rules_namespaced_file(Path::new("/proj")),
            Some(PathBuf::from("/proj/.clinerules/tome.md")),
        );
    }

    #[test]
    fn dialect_is_mcpservers_command_args_emit_env() {
        let d = CLINE.mcp_dialect();
        assert_eq!(d.parent_key, "mcpServers");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, None);
        assert!(d.emit_env);
        assert!(!CLINE.mcp_manual_only());
    }
}

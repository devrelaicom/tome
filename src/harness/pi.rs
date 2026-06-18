//! `pi` — the Pi agent.
//!
//! Phase 11 (US1). Baseline integration: rules-file + MCP dialect only.
//! Session steering (the G2 `TsPlugin` shim) lands in US2/US3.
//!
//! - Per-user dir: `~/.pi/` (the default `detect_path`).
//! - Rules-file target: `<project>/AGENTS.md`, `BlockInExistingFile` ·
//!   `Inline` (the trait default). Shares `AGENTS.md` with codex / gemini /
//!   opencode / devin — the shared-sink single-region collapse handles it.
//! - MCP config: `~/.pi/agent/mcp.json` (GLOBAL, under `home`). Tome writes
//!   a normal `mcpServers` entry here. The "install `pi-mcp-adapter`" notice
//!   is deferred to US5, so `mcp_manual_only()` stays the default `false` —
//!   Tome DOES write the file in US1.
//! - MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`,
//!   `emit_env:true` (`"env": {}`), no extra fields.

use std::path::{Path, PathBuf};

use crate::harness::{EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy};

/// Unit struct implementing [`HarnessModule`] for Pi.
pub struct Pi;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const PI: Pi = Pi;

impl HarnessModule for Pi {
    fn name(&self) -> &'static str {
        "pi"
    }

    fn description(&self) -> &'static str {
        "Pi agent"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".pi").is_dir()
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("AGENTS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    // `block_body_style()` defaults to `Inline` (G3) — exactly Pi's body.

    fn mcp_config_path(&self, _project_root: &Path, home: &Path) -> PathBuf {
        // GLOBAL config under the per-user dir.
        home.join(".pi/agent/mcp.json")
    }

    /// Pi's MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`,
    /// `emit_env:true` (`"env": {}`), no extra fields.
    ///
    /// NOTE: `mcp_manual_only()` stays the default `false` in US1 — Tome
    /// writes the file. The "install pi-mcp-adapter" success-with-notice is a
    /// US5 fast-follow.
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
        assert_eq!(PI.name(), "pi");
        assert_eq!(PI.detect_path(Path::new("/h")), Path::new("/h/.pi"));
        assert_eq!(
            PI.rules_file_target(Path::new("/proj")),
            Path::new("/proj/AGENTS.md"),
        );
        // GLOBAL MCP path under home.
        assert_eq!(
            PI.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/h/.pi/agent/mcp.json"),
        );
    }

    #[test]
    fn dialect_is_mcpservers_command_args_emit_env_and_not_manual() {
        let d = PI.mcp_dialect();
        assert_eq!(d.parent_key, "mcpServers");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, None);
        assert!(d.emit_env);
        // US1: Pi writes its MCP file (the adapter notice is US5).
        assert!(!PI.mcp_manual_only());
    }
}

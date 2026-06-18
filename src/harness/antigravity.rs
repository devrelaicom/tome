//! `antigravity` — the Antigravity IDE.
//!
//! Phase 11 (US1). Baseline integration: standalone rules file + MCP dialect.
//! Session steering (the G2 `injectSteps` envelope) lands in US2/US3.
//!
//! - Per-user dir: `~/.gemini/` — Antigravity shares the Gemini config tree.
//!   The `antigravity-cli` → `gemini` alias (in [`HARNESS_ALIASES`]) routes
//!   the *CLI* to the Gemini module; this `antigravity` module is the IDE,
//!   which has its OWN project rules sink + a GLOBAL MCP file under the
//!   shared Gemini tree.
//! - Rules-file sink: a `StandaloneFile` at `<project>/.agent/rules/tome.md`.
//! - MCP config: `~/.gemini/config/mcp_config.json` (GLOBAL, under `home`).
//! - MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`,
//!   `emit_env:true` (`"env": {}`), no extra fields.
//!
//! [`HARNESS_ALIASES`]: super::HARNESS_ALIASES

use std::path::{Path, PathBuf};

use crate::harness::{EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy};

/// Unit struct implementing [`HarnessModule`] for Antigravity IDE.
pub struct Antigravity;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const ANTIGRAVITY: Antigravity = Antigravity;

impl HarnessModule for Antigravity {
    fn name(&self) -> &'static str {
        "antigravity"
    }

    fn description(&self) -> &'static str {
        "Antigravity IDE"
    }

    fn detect(&self, home: &Path) -> bool {
        // Antigravity shares the Gemini config tree.
        home.join(".gemini").is_dir()
    }

    fn detect_path(&self, home: &Path) -> PathBuf {
        // `name()` is "antigravity" but the per-user dir is `~/.gemini/`.
        home.join(".gemini")
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join(".agent/rules/tome.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::StandaloneFile
    }

    fn rules_namespaced_file(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".agent/rules/tome.md"))
    }

    // F5 DEFER (US1 closeout): antigravity is a `StandaloneFile` rules harness
    // but inherits the DEFAULT `guardrails_target` = `InFileRegion` on the SAME
    // `.agent/rules/tome.md` path — needs an explicit guardrails-sink decision
    // (StandaloneSibling or suppression) before the guardrails pass is wired for
    // the new harnesses.
    // TODO(P11-guardrails): pick the guardrails sink for StandaloneFile harnesses.

    fn mcp_config_path(&self, _project_root: &Path, home: &Path) -> PathBuf {
        // GLOBAL config under the shared Gemini tree.
        home.join(".gemini/config/mcp_config.json")
    }

    /// Antigravity's MCP dialect: JSON `mcpServers` + `CommandArgs`, no
    /// `type`, `emit_env:true` (`"env": {}`), no extra fields.
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
        assert_eq!(ANTIGRAVITY.name(), "antigravity");
        assert_eq!(
            ANTIGRAVITY.detect_path(Path::new("/h")),
            Path::new("/h/.gemini"),
        );
        assert_eq!(
            ANTIGRAVITY.rules_file_target(Path::new("/proj")),
            Path::new("/proj/.agent/rules/tome.md"),
        );
        // GLOBAL MCP path under the shared Gemini tree.
        assert_eq!(
            ANTIGRAVITY.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/h/.gemini/config/mcp_config.json"),
        );
    }

    #[test]
    fn dialect_is_mcpservers_command_args_emit_env() {
        let d = ANTIGRAVITY.mcp_dialect();
        assert_eq!(d.parent_key, "mcpServers");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, None);
        assert!(d.emit_env);
        assert!(!ANTIGRAVITY.mcp_manual_only());
    }

    /// Live-probe gate (T087): NOT run in CI. A human must confirm against a
    /// real Antigravity install the `.agent/` rules dir, the `.agents/` hooks
    /// dir, and the `injectSteps` session-start envelope before US2/US3 ship.
    #[test]
    #[ignore = "live-probe: confirm .agent/ rules dir + .agents/ hooks dir + injectSteps envelope"]
    fn antigravity_rules_hooks_dirs_and_inject_steps_live_probe() {
        // No automated body — see the doc comment for the manual checklist.
    }
}

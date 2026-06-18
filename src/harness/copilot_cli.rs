//! `copilot-cli` — GitHub Copilot CLI.
//!
//! Phase 11. Baseline integration: rules-file + MCP dialect (US1) plus the
//! G2 `CommandHook` SessionStart entry (US2).
//!
//! ## Session steering (US2, T046)
//!
//! Copilot CLI gets a Tome-owned `SessionStart` command hook written into
//! `<project>/.github/hooks/tome.json` (the `CopilotHooks` spec — wrapped in
//! `{ version, hooks: { SessionStart: [...] } }`, under a Tome-dedicated
//! `tome.json` so it never collides with a developer's own hook file). The
//! hook runs `tome harness session-start --workspace <ws> --harness
//! copilot-cli`, whose stdout is wrapped in the
//! [`Envelope::FlatAdditionalContext`] `{ "additionalContext": … }` shape
//! (contract session-steering.md §Stdout envelopes).
//!
//! [`Envelope::FlatAdditionalContext`]: crate::harness::Envelope::FlatAdditionalContext
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
    EntryShape, Envelope, ExtraField, ExtraValue, FileFormat, HarnessModule, HookEvent,
    HookFileSpec, McpDialect, RulesFileStrategy, ServerType, SessionSteering,
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

    /// Session steering (US2, T046): a `SessionStart` command hook in
    /// `<project>/.github/hooks/tome.json` (the `CopilotHooks` spec) whose
    /// stdout is wrapped in the [`Envelope::FlatAdditionalContext`] shape.
    fn session_steering(&self) -> SessionSteering {
        SessionSteering::CommandHook {
            file_spec: HookFileSpec::CopilotHooks,
            event: HookEvent::SessionStart,
            envelope: Envelope::FlatAdditionalContext,
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

    /// US2 (T046): copilot-cli steers via a `CopilotHooks` `SessionStart`
    /// command hook wrapped in the `FlatAdditionalContext` envelope.
    #[test]
    fn session_steering_is_copilot_hooks_session_start_flat() {
        assert_eq!(
            COPILOT_CLI.session_steering(),
            SessionSteering::CommandHook {
                file_spec: HookFileSpec::CopilotHooks,
                event: HookEvent::SessionStart,
                envelope: Envelope::FlatAdditionalContext,
            },
        );
    }
}

//! `devin` — Cognition's Devin agent.
//!
//! Phase 11. Baseline integration: rules-file + MCP dialect (US1) plus the
//! G2 `CommandHook` SessionStart entry (US2).
//!
//! ## Session steering (US2, T045)
//!
//! Devin gets a Tome-owned `SessionStart` command hook written into
//! `<project>/.devin/hooks.v1.json` (the `DevinHooksV1` spec — no wrapper key)
//! by the `reconcile_command_hooks` pass. The hook runs `tome harness
//! session-start --workspace <ws> --harness devin`, whose stdout is wrapped in
//! the [`Envelope::ClaudeNested`] `{ "hookSpecificOutput": { … } }` shape Devin
//! consumes (contract session-steering.md §Stdout envelopes).
//!
//! [`Envelope::ClaudeNested`]: crate::harness::Envelope::ClaudeNested
//!
//! - Per-user dir: `~/.devin/` (the default `detect_path`).
//! - Rules-file target: `<project>/AGENTS.md`, `BlockInExistingFile` ·
//!   `AtInclude` (devin resolves `@`-includes — contract §"Body delivery
//!   rule"). Shares `AGENTS.md` with codex / gemini (`AtInclude`) and pi /
//!   opencode (`Inline`) — the shared-sink single-region collapse handles
//!   co-ownership.
//!
//! ## Shared-sink body-style resolution (pre-existing, NOT redesigned here)
//!
//! The orchestrator writes ONE Tome region per shared `AGENTS.md` path, and its
//! body style is the GROUP's lowest-common-denominator (`group_body_style` in
//! `sync.rs`): `Inline` wins the moment ANY live co-owner requires it, because
//! an include directive is unreadable as prose to a non-`@`-resolving harness,
//! whereas an inline body is readable by everyone. So when pi or opencode
//! (`Inline`) is also live, the shared region is written `Inline` regardless of
//! devin's own declared style; when ONLY `AtInclude` harnesses (devin + codex +
//! gemini) co-own the path, the region is the `@`-include form devin resolves.
//! Declaring `AtInclude` here makes devin's stated capability match the
//! contract; the shared-sink LCD (a pre-existing concern) is unchanged.
//! - MCP config: `<project>/.devin/config.json` (per-project).
//! - MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`, `emit_env:true`
//!   (`"env": {}` per the contract), no extra fields.

use std::path::{Path, PathBuf};

use crate::harness::{
    BlockBodyStyle, EntryShape, Envelope, FileFormat, HarnessModule, HookEvent, HookFileSpec,
    McpDialect, RulesFileStrategy, SessionSteering,
};

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

    /// Devin resolves `@`-style includes (contract §"Body delivery rule",
    /// line 19), so its OWN-only body is an `@`-include. When it co-owns a
    /// shared `AGENTS.md` with an `Inline`-only harness the group LCD wins (see
    /// the module doc comment) — this declares devin's capability, not the
    /// shared-sink outcome.
    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::AtInclude
    }

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

    /// Session steering (US2, T045): a `SessionStart` command hook in
    /// `<project>/.devin/hooks.v1.json` (the `DevinHooksV1` spec) whose stdout
    /// is wrapped in the [`Envelope::ClaudeNested`] shape Devin consumes.
    fn session_steering(&self) -> SessionSteering {
        SessionSteering::CommandHook {
            file_spec: HookFileSpec::DevinHooksV1,
            event: HookEvent::SessionStart,
            envelope: Envelope::ClaudeNested,
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
    fn block_body_style_is_at_include() {
        // Devin resolves `@`-includes (contract §"Body delivery rule", line 19).
        // The shared-sink LCD may still write the region inline when an
        // Inline-only co-owner is live; this pins devin's declared capability.
        assert_eq!(DEVIN.block_body_style(), BlockBodyStyle::AtInclude);
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

    /// US2 (T045): devin steers via a `DevinHooksV1` `SessionStart` command
    /// hook wrapped in the `ClaudeNested` envelope.
    #[test]
    fn session_steering_is_devin_hooks_v1_session_start_claude_nested() {
        assert_eq!(
            DEVIN.session_steering(),
            SessionSteering::CommandHook {
                file_spec: HookFileSpec::DevinHooksV1,
                event: HookEvent::SessionStart,
                envelope: Envelope::ClaudeNested,
            },
        );
    }
}

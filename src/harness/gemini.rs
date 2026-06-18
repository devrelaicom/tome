//! `gemini` — Google Gemini CLI.
//!
//! Per research §R-8:
//!
//! - Per-user dir: `~/.gemini/`.
//! - Rules-file target: `AGENTS.md` > `GEMINI.md` > `.gemini/GEMINI.md`
//!   (first existing wins; falls back to `AGENTS.md` if none exist).
//! - Strategy: `BlockInExistingFile`, body style `AtInclude`.
//! - MCP config: `~/.gemini/settings.json` (GLOBAL, under `home`).
//! - Parent key: `"mcpServers"`.
//! - Guardrails target (Phase 6): `AGENTS.md` preferred else `GEMINI.md`,
//!   in-file region, no suppression — the trait default (`InFileRegion` on
//!   the rules-file target) yields exactly this, so no `guardrails_target`
//!   override is needed (FR-012).
//!
//! ## Session steering (Phase 11, US2, T048) — MCP vs hook are DIFFERENT files
//!
//! Gemini gets a Tome-owned `SessionStart` command hook (the `GeminiSettings`
//! spec) wrapped in [`Envelope::ClaudeNested`], delivered via the
//! `reconcile_command_hooks` pass.
//!
//! CRITICAL distinction (no clobber):
//!
//! - The **MCP** server is written to the **GLOBAL** `~/.gemini/settings.json`
//!   (`mcp_config_path` joins under `home`) — the `mcpServers` key.
//! - The **hook** is written to the **PROJECT** `<project>/.gemini/settings.json`
//!   (`HookFileSpec::GeminiSettings` resolves under `project_root`, see
//!   `reconcile::hooks::hook_file_path`) — the `hooks` key.
//!
//! These are two distinct files (`home` vs `project_root`), so the MCP write and
//! the hook write never touch the same file and cannot clobber each other. Even
//! if a user's `home` and `project_root` coincided, the two writers touch
//! disjoint top-level keys (`mcpServers` vs `hooks`) and both go through the
//! lenient, preserve-order JSON read/modify/write path, so each preserves the
//! other's keys.
//!
//! [`Envelope::ClaudeNested`]: crate::harness::Envelope::ClaudeNested

use std::path::{Path, PathBuf};

use crate::harness::{
    BlockBodyStyle, Envelope, HarnessModule, HookEvent, HookFileSpec, RulesFileStrategy,
    SessionSteering,
};

/// Unit struct implementing [`HarnessModule`] for Gemini CLI.
pub struct Gemini;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const GEMINI: Gemini = Gemini;

impl HarnessModule for Gemini {
    fn name(&self) -> &'static str {
        "gemini"
    }

    fn description(&self) -> &'static str {
        "Google Gemini CLI"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".gemini").is_dir()
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        for candidate in ["AGENTS.md", "GEMINI.md", ".gemini/GEMINI.md"] {
            let p = project_root.join(candidate);
            if p.exists() {
                return p;
            }
        }
        project_root.join("AGENTS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::AtInclude
    }

    fn mcp_config_path(&self, _project_root: &Path, home: &Path) -> PathBuf {
        // GLOBAL `~/.gemini/settings.json` — distinct from the PROJECT
        // `<project>/.gemini/settings.json` the `GeminiSettings` hook writes
        // (see `session_steering`). Different files: no clobber.
        home.join(".gemini/settings.json")
    }

    // MCP dialect: the trait default ([`McpDialect::LEGACY`]) is exactly
    // Gemini's shape (JSON `mcpServers` + `CommandArgs`), so no override.

    /// Session steering (US2, T048): a `SessionStart` command hook in the
    /// PROJECT `<project>/.gemini/settings.json` (the `GeminiSettings` spec,
    /// `hooks` key) wrapped in the [`Envelope::ClaudeNested`] shape. This is a
    /// DIFFERENT file from the GLOBAL `~/.gemini/settings.json` the MCP server
    /// is written to (see the module doc comment) — no clobber.
    fn session_steering(&self) -> SessionSteering {
        SessionSteering::CommandHook {
            file_spec: HookFileSpec::GeminiSettings,
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
        assert_eq!(GEMINI.name(), "gemini");
        assert_eq!(GEMINI.detect_path(Path::new("/h")), Path::new("/h/.gemini"));
        // MCP server → GLOBAL settings.json under home.
        assert_eq!(
            GEMINI.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/h/.gemini/settings.json"),
        );
    }

    /// US2 (T048): gemini steers via a `GeminiSettings` `SessionStart` command
    /// hook wrapped in the `ClaudeNested` envelope.
    #[test]
    fn session_steering_is_gemini_settings_session_start_claude_nested() {
        assert_eq!(
            GEMINI.session_steering(),
            SessionSteering::CommandHook {
                file_spec: HookFileSpec::GeminiSettings,
                event: HookEvent::SessionStart,
                envelope: Envelope::ClaudeNested,
            },
        );
    }

    /// CRITICAL (T048): the MCP file (GLOBAL `~/.gemini/settings.json`) and the
    /// hook file (PROJECT `<project>/.gemini/settings.json`) are DIFFERENT
    /// files — a project root distinct from home keeps them disjoint, so the
    /// MCP write and the hook write never clobber each other.
    #[test]
    fn mcp_file_and_hook_file_are_different_paths() {
        let home = Path::new("/h");
        let project = Path::new("/proj");
        let mcp_path = GEMINI.mcp_config_path(project, home);
        // The hook path is what `reconcile::hooks::hook_file_path` computes for
        // the `GeminiSettings` spec: `<project_root>/.gemini/settings.json`.
        let hook_path = project.join(".gemini/settings.json");
        assert_eq!(mcp_path, Path::new("/h/.gemini/settings.json"));
        assert_eq!(hook_path, Path::new("/proj/.gemini/settings.json"));
        assert_ne!(
            mcp_path, hook_path,
            "gemini MCP (global) and hook (project) must be different files",
        );
    }
}

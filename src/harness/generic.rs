//! `generic` — the portable AGENTS.md + `./mcp.json` write target.
//!
//! Phase 11 (US4). An OPT-IN target (never auto-detected, never in `--all`):
//! the user opts in by name via `tome harness use generic`. It flows through
//! the STANDARD sink loop — no special dispatch — and writes the lowest-common-
//! denominator surface a portable agent host understands:
//!
//! - Rules: a `<!-- tome:begin -->…<!-- tome:end -->` block (Inline body) inside
//!   `<project>/AGENTS.md` (shared with the other `AGENTS.md` sharers; the
//!   single-region collapse keeps exactly one Tome block, FR-013a).
//! - MCP config: `<project>/mcp.json` (project-root, `mcpServers` + `CommandArgs`
//!   + `"env": {}`).
//!
//! Because it inherits every Phase-6/Phase-11 trait default (GuardrailsOnly, no
//! native agents, `SessionSteering::None`), it is safe-by-default: only the two
//! sinks above ever run for it.
//!
//! Registered in [`super::OPT_IN_TARGETS`], NOT [`super::SUPPORTED_HARNESSES`]:
//! [`detect`](HarnessModule::detect) returns `false` unconditionally — there is
//! no per-user `generic` dir to probe — so the harness never surfaces via
//! detection and `--all` never writes it.

use std::path::{Path, PathBuf};

use crate::harness::{
    BlockBodyStyle, EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy,
};

/// Unit struct implementing [`HarnessModule`] for the `generic` target.
pub struct Generic;

/// Static instance used by the [`OPT_IN_TARGETS`] registry.
///
/// [`OPT_IN_TARGETS`]: super::OPT_IN_TARGETS
pub const GENERIC: Generic = Generic;

impl HarnessModule for Generic {
    fn name(&self) -> &'static str {
        "generic"
    }

    fn description(&self) -> &'static str {
        "Generic AGENTS.md + ./mcp.json target"
    }

    fn detect(&self, _home: &Path) -> bool {
        // Inert: `generic` is a write target the user opts into by name, not a
        // detectable harness. Never auto-detected, never in `--all`.
        false
    }

    fn is_opt_in_target(&self) -> bool {
        true
    }

    fn detect_path(&self, _home: &Path) -> PathBuf {
        // No per-user dir exists; report a stable, never-existing sentinel so
        // `tome harness info`'s `detected_path` is honest (it never claims this
        // is present). The default `~/.generic` would imply a probe that never
        // happens, so override to a `<home>`-relative marker that documents the
        // opt-in nature.
        _home.join(".tome/generic-target")
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("AGENTS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        // No documented `@`-include support for a generic host — inline the
        // verbatim rules so every reader receives the real directive.
        BlockBodyStyle::Inline
    }

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join("mcp.json")
    }

    /// JSON `mcpServers` + `CommandArgs`, no `type`, `emit_env:true`
    /// (`"env": {}`), no extra fields — the portable shape per contract
    /// mcp-dialects.md.
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
        assert_eq!(GENERIC.name(), "generic");
        assert_eq!(
            GENERIC.rules_file_target(Path::new("/proj")),
            Path::new("/proj/AGENTS.md"),
        );
        assert_eq!(
            GENERIC.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/proj/mcp.json"),
        );
    }

    #[test]
    fn never_detected() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Even with a `.generic` dir present, detection stays false — it is an
        // opt-in-by-name target.
        std::fs::create_dir_all(tmp.path().join(".generic")).unwrap();
        assert!(!GENERIC.detect(tmp.path()));
    }

    #[test]
    fn is_opt_in_target() {
        assert!(GENERIC.is_opt_in_target());
        assert!(GENERIC.open_plugins_root(Path::new("/proj")).is_none());
    }

    #[test]
    fn dialect_is_mcp_servers_command_args_emit_env() {
        let d = GENERIC.mcp_dialect();
        assert_eq!(d.parent_key, "mcpServers");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, None);
        assert!(d.emit_env);
        assert!(d.extra_fields.is_empty());
        assert!(!GENERIC.mcp_manual_only());
    }
}

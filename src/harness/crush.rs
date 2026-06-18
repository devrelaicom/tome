//! `crush` — Charm's Crush CLI.
//!
//! Phase 11 (US1). Baseline integration: rules-file + MCP dialect only.
//!
//! - Per-user dir: `~/.config/crush/` (preferred) or `~/.crush/`.
//! - Rules-file target: `<project>/CRUSH.md`, `BlockInExistingFile` ·
//!   `Inline` (the trait default).
//! - MCP config: `<project>/crush.json` (per-project, no dot prefix).
//! - MCP dialect: JSON `mcp` parent key + `CommandArgs`, per-entry
//!   `type:"stdio"`, omit-empty-`env`, no extra fields.

use std::path::{Path, PathBuf};

use crate::harness::{
    EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy, ServerType,
};

/// Unit struct implementing [`HarnessModule`] for Crush.
pub struct Crush;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const CRUSH: Crush = Crush;

impl HarnessModule for Crush {
    fn name(&self) -> &'static str {
        "crush"
    }

    fn description(&self) -> &'static str {
        "Charm Crush"
    }

    fn detect(&self, home: &Path) -> bool {
        // Crush stores per-user config under XDG `~/.config/crush/`; tolerate
        // the legacy `~/.crush/` dir too.
        home.join(".config/crush").is_dir() || home.join(".crush").is_dir()
    }

    fn detect_path(&self, home: &Path) -> PathBuf {
        // `detect` accepts EITHER the XDG `~/.config/crush/` or the legacy
        // `~/.crush/` dir. Report whichever actually EXISTS so `tome harness
        // info` doesn't claim a non-existent `detected_path`; fall back to the
        // XDG dir (the preferred primary) when neither is present.
        let xdg = home.join(".config/crush");
        if xdg.is_dir() {
            return xdg;
        }
        let legacy = home.join(".crush");
        if legacy.is_dir() {
            return legacy;
        }
        xdg
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("CRUSH.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    // `block_body_style()` defaults to `Inline` (G3) — exactly Crush's body.

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join("crush.json")
    }

    /// Crush's MCP dialect: JSON `mcp` parent key + `CommandArgs`, per-entry
    /// `type:"stdio"`, omit-empty-`env`, no extra fields.
    fn mcp_dialect(&self) -> McpDialect {
        McpDialect {
            file_format: FileFormat::Json,
            parent_key: "mcp",
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
        assert_eq!(CRUSH.name(), "crush");
        assert_eq!(
            CRUSH.detect_path(Path::new("/h")),
            Path::new("/h/.config/crush"),
        );
        assert_eq!(
            CRUSH.rules_file_target(Path::new("/proj")),
            Path::new("/proj/CRUSH.md"),
        );
        assert_eq!(
            CRUSH.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/proj/crush.json"),
        );
    }

    #[test]
    fn detect_path_reports_legacy_dir_when_only_legacy_exists() {
        // When ONLY `~/.crush/` exists (no XDG dir), `detect_path` must report
        // the legacy dir — not a non-existent `~/.config/crush/`.
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join(".crush")).unwrap();
        assert_eq!(CRUSH.detect_path(home), home.join(".crush"));
        assert!(
            CRUSH.detect(home),
            "detect must agree the harness is present"
        );
    }

    #[test]
    fn detect_path_prefers_xdg_when_both_exist() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join(".config/crush")).unwrap();
        std::fs::create_dir_all(home.join(".crush")).unwrap();
        assert_eq!(CRUSH.detect_path(home), home.join(".config/crush"));
    }

    #[test]
    fn detect_path_falls_back_to_xdg_when_neither_exists() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        assert_eq!(CRUSH.detect_path(home), home.join(".config/crush"));
    }

    #[test]
    fn dialect_is_mcp_command_args_type_stdio() {
        let d = CRUSH.mcp_dialect();
        assert_eq!(d.parent_key, "mcp");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, Some(ServerType::Stdio));
        assert!(!d.emit_env);
        assert!(d.extra_fields.is_empty());
        assert!(!CRUSH.mcp_manual_only());
    }
}

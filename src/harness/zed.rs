//! `zed` — Zed editor.
//!
//! Phase 11 (US1). Baseline integration: standalone rules file + MCP dialect.
//!
//! - Per-user dir: `~/.config/zed/` (preferred) or `~/.zed/`.
//! - Rules-file sink: a `StandaloneFile` at `<project>/.rules` — Zed's
//!   highest-precedence project rules file (first-match-wins). The
//!   `#[ignore]`d live-probe below documents the manual confirmation that
//!   Zed actually reads project `.rules` as a first-match-wins sink.
//! - MCP config: `<project>/.zed/settings.json` (per-project).
//! - MCP dialect: JSON `context_servers` parent key + `CommandArgs`, no
//!   `type`, `emit_env:true` (`"env": {}`), no extra fields.

use std::path::{Path, PathBuf};

use crate::harness::{EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy};

/// Unit struct implementing [`HarnessModule`] for Zed.
pub struct Zed;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const ZED: Zed = Zed;

impl HarnessModule for Zed {
    fn name(&self) -> &'static str {
        "zed"
    }

    fn description(&self) -> &'static str {
        "Zed editor"
    }

    fn detect(&self, home: &Path) -> bool {
        // Zed stores per-user config under XDG `~/.config/zed/`; tolerate the
        // legacy `~/.zed/` dir too.
        home.join(".config/zed").is_dir() || home.join(".zed").is_dir()
    }

    fn detect_path(&self, home: &Path) -> PathBuf {
        // `detect` accepts EITHER the XDG `~/.config/zed/` or the legacy
        // `~/.zed/` dir. Report whichever actually EXISTS so `tome harness info`
        // doesn't claim a non-existent `detected_path`; fall back to the XDG dir
        // (the preferred primary) when neither is present.
        let xdg = home.join(".config/zed");
        if xdg.is_dir() {
            return xdg;
        }
        let legacy = home.join(".zed");
        if legacy.is_dir() {
            return legacy;
        }
        xdg
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        // `.rules` is the dedicated Tome-owned standalone sink (the highest-
        // precedence project rules file Zed reads). Declared as the namespaced
        // file so the never-clobber intent is explicit.
        project_root.join(".rules")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::StandaloneFile
    }

    fn rules_namespaced_file(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".rules"))
    }

    // F5 DEFER (US1 closeout): zed is a `StandaloneFile` rules harness but
    // inherits the DEFAULT `guardrails_target` = `InFileRegion` on the SAME
    // `.rules` path — needs an explicit guardrails-sink decision
    // (StandaloneSibling or suppression) before the guardrails pass is wired for
    // the new harnesses.
    // TODO(P11-guardrails): pick the guardrails sink for StandaloneFile harnesses.

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join(".zed/settings.json")
    }

    /// Zed's MCP dialect: JSON `context_servers` parent key + `CommandArgs`,
    /// no `type`, `emit_env:true` (`"env": {}`), no extra fields.
    fn mcp_dialect(&self) -> McpDialect {
        McpDialect {
            file_format: FileFormat::Json,
            parent_key: "context_servers",
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
        assert_eq!(ZED.name(), "zed");
        assert_eq!(
            ZED.detect_path(Path::new("/h")),
            Path::new("/h/.config/zed")
        );
        assert_eq!(
            ZED.rules_file_target(Path::new("/proj")),
            Path::new("/proj/.rules"),
        );
        assert_eq!(
            ZED.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/proj/.zed/settings.json"),
        );
    }

    #[test]
    fn detect_path_reports_legacy_dir_when_only_legacy_exists() {
        // When ONLY `~/.zed/` exists (no XDG dir), `detect_path` must report the
        // legacy dir — not a non-existent `~/.config/zed/`.
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join(".zed")).unwrap();
        assert_eq!(ZED.detect_path(home), home.join(".zed"));
        assert!(ZED.detect(home), "detect must agree the harness is present");
    }

    #[test]
    fn detect_path_prefers_xdg_when_both_exist() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        std::fs::create_dir_all(home.join(".config/zed")).unwrap();
        std::fs::create_dir_all(home.join(".zed")).unwrap();
        assert_eq!(ZED.detect_path(home), home.join(".config/zed"));
    }

    #[test]
    fn detect_path_falls_back_to_xdg_when_neither_exists() {
        let tmp = tempfile::TempDir::new().unwrap();
        let home = tmp.path();
        assert_eq!(ZED.detect_path(home), home.join(".config/zed"));
    }

    #[test]
    fn dialect_is_context_servers_command_args_emit_env() {
        let d = ZED.mcp_dialect();
        assert_eq!(d.parent_key, "context_servers");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, None);
        assert!(d.emit_env);
        assert!(!ZED.mcp_manual_only());
    }

    /// Live-probe gate (T087): NOT run in CI. A human must confirm against a
    /// real Zed install that a project-root `.rules` file is read as the
    /// first-match-wins highest-precedence rules sink before this ships.
    #[test]
    #[ignore = "live-probe: confirm Zed reads project .rules as a first-match-wins sink"]
    fn zed_reads_project_rules_as_first_match_wins_live_probe() {
        // No automated body — see the doc comment for the manual checklist a
        // human runs against a real Zed install. Present so the gate is
        // discoverable via `cargo test -- --ignored`.
    }
}

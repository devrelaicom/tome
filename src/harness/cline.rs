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

use crate::harness::{
    EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy, SessionSteering, ShimKind,
};

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

    // F5 DEFER (US1 closeout): cline is a `StandaloneFile` rules harness but
    // inherits the DEFAULT `guardrails_target` = `InFileRegion` on the SAME
    // standalone path — needs an explicit guardrails-sink decision
    // (StandaloneSibling or suppression) before the guardrails pass is wired for
    // the new harnesses.
    // TODO(P11-guardrails): pick the guardrails sink for StandaloneFile harnesses.

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

    /// Cline cannot run a native session-start hook, so Tome ships an embedded
    /// TypeScript plugin shim (Phase 11 / G2, US3). The dir is PROJECT-RELATIVE:
    /// `reconcile_plugins` anchors it under `project_root` via `project_root.join(dir)`
    /// (a relative `dir` is anchored; the `session_steering()` signature takes no
    /// `project_root`, so relative is the only option). The shim lands at
    /// `<project>/.cline/plugins/tome.ts` — a dedicated file inside Cline's own
    /// plugin dir, so a developer's sibling plugin is never clobbered or removed.
    fn session_steering(&self) -> SessionSteering {
        SessionSteering::TsPlugin {
            dir: PathBuf::from(".cline/plugins"),
            kind: ShimKind::Cline,
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

    /// Phase 11 / US3 (T057): Cline's session steering is the embedded
    /// `TsPlugin` shim, project-relative dir `.cline/plugins`, `ShimKind::Cline`.
    #[test]
    fn session_steering_is_cline_ts_plugin() {
        assert_eq!(
            CLINE.session_steering(),
            SessionSteering::TsPlugin {
                dir: PathBuf::from(".cline/plugins"),
                kind: ShimKind::Cline,
            },
        );
    }

    /// T058 — shim byte pin: Cline's embedded `tome.ts` is non-empty.
    #[test]
    fn embedded_shim_is_non_empty() {
        let plugin =
            crate::harness::plugin_assets::find("cline").expect("cline shim must be embedded");
        let entry = plugin
            .files
            .iter()
            .find(|f| f.rel_path == "tome.ts")
            .expect("cline shim must contain tome.ts");
        assert!(!entry.bytes.is_empty(), "cline tome.ts must be non-empty");
    }

    /// T058 — invocation + fail-closed pin: the embedded shim invokes
    /// `tome … session-start … --harness cline` (B3: it defers to the Rust
    /// directive source) and no-ops fail-closed on a missing binary (a `catch`
    /// that returns the empty string → no injection).
    #[test]
    fn embedded_shim_invokes_session_start_and_fails_closed() {
        let plugin = crate::harness::plugin_assets::find("cline").unwrap();
        let src = std::str::from_utf8(plugin.files[0].bytes).expect("shim is UTF-8");
        // Invokes the `tome` launcher's `session-start` for THIS harness.
        assert!(src.contains("\"tome\""), "shim launches the `tome` binary");
        assert!(
            src.contains("session-start"),
            "shim runs the session-start subcommand",
        );
        assert!(
            src.contains("\"--harness\"") && src.contains("\"cline\""),
            "shim passes --harness cline (defers to the Rust directive source)",
        );
        // Fail-closed: a catch that yields empty → no injection.
        assert!(
            src.contains("catch") && src.contains("return \"\""),
            "shim must fail closed (catch → empty string → no injection) on a missing binary",
        );
    }
}

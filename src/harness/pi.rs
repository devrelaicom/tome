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

use crate::harness::{
    EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy, SessionSteering, ShimKind,
};

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

    /// Pi cannot run a native session-start hook, so Tome ships an embedded
    /// TypeScript extension shim (Phase 11 / G2, US3). The dir is PROJECT-RELATIVE
    /// — `reconcile_plugins` anchors it under `project_root` via
    /// `project_root.join(dir)`. The shim lands at `<project>/.pi/extensions/tome.ts`,
    /// a dedicated file inside Pi's own extensions dir; a developer's sibling
    /// extension is never touched.
    fn session_steering(&self) -> SessionSteering {
        SessionSteering::TsPlugin {
            dir: PathBuf::from(".pi/extensions"),
            kind: ShimKind::Pi,
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

    /// Phase 11 / US3 (T057): Pi's session steering is the embedded `TsPlugin`
    /// shim, project-relative dir `.pi/extensions`, `ShimKind::Pi`.
    #[test]
    fn session_steering_is_pi_ts_plugin() {
        assert_eq!(
            PI.session_steering(),
            SessionSteering::TsPlugin {
                dir: PathBuf::from(".pi/extensions"),
                kind: ShimKind::Pi,
            },
        );
    }

    /// T058 — shim byte pin: Pi's embedded `tome.ts` is non-empty.
    #[test]
    fn embedded_shim_is_non_empty() {
        let plugin = crate::harness::plugin_assets::find("pi").expect("pi shim must be embedded");
        let entry = plugin
            .files
            .iter()
            .find(|f| f.rel_path == "tome.ts")
            .expect("pi shim must contain tome.ts");
        assert!(!entry.bytes.is_empty(), "pi tome.ts must be non-empty");
    }

    /// T058 — invocation + fail-closed pin: the embedded shim invokes
    /// `tome … session-start … --harness pi` (B3) and no-ops fail-closed on a
    /// missing binary.
    #[test]
    fn embedded_shim_invokes_session_start_and_fails_closed() {
        let plugin = crate::harness::plugin_assets::find("pi").unwrap();
        let shim = plugin
            .files
            .iter()
            .find(|f| f.rel_path == "tome.ts")
            .expect("pi shim must contain tome.ts");
        let src = std::str::from_utf8(shim.bytes).expect("shim is UTF-8");
        assert!(src.contains("\"tome\""), "shim launches the `tome` binary");
        assert!(
            src.contains("session-start"),
            "shim runs the session-start subcommand",
        );
        assert!(
            src.contains("\"--harness\"") && src.contains("\"pi\""),
            "shim passes --harness pi (defers to the Rust directive source)",
        );
        assert!(
            src.contains("catch") && src.contains("return \"\""),
            "shim must fail closed (catch → empty string → no injection) on a missing binary",
        );
    }

    /// Live-probe merge gate (T087). NOT run in CI — a human must run this
    /// against a real Pi install before the shim ships.
    ///
    /// What to verify by hand:
    ///
    /// 1. `tome sync --harness pi` (or `tome harness use pi`) in a
    ///    workspace-bound project, then confirm `.pi/extensions/tome.ts` is
    ///    written.
    /// 2. Start Pi in that project and confirm the shim's
    ///    `pi.on("before_agent_start", …)` handler is actually invoked and the
    ///    returned `{ message: { customType:"tome", content:<directive>,
    ///    display:true } }` is injected at session start. The byte-pin +
    ///    integration tests prove Tome WRITES the shim; only a real Pi can
    ///    confirm it READS this extension API shape.
    #[test]
    #[ignore = "live-probe: confirm Pi before_agent_start extension API shape"]
    fn pi_reads_before_agent_start_shape_live_probe() {
        // No automated body — see the doc comment for the manual checklist a
        // human runs against a real Pi install. Present so the gate is
        // discoverable via `cargo test -- --ignored`.
    }
}

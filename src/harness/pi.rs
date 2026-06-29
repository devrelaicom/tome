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

use crate::error::TomeError;
use crate::harness::agents::{
    self, CanonicalAgent, TranslatedAgent, agent_extension, agent_filename,
};
use crate::harness::{
    AgentFormat, EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy,
    SessionSteering, ShimKind,
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

    /// Pi's MCP entry requires the `pi-mcp-adapter` to take effect. Tome writes
    /// the file (so `mcp_manual_only()` stays `false`) but `use` emits this
    /// install instruction and doctor/status report `unverified` (Phase 11 /
    /// US5, contract mcp-dialects.md § "Manual-only" — pi case).
    fn mcp_adapter_notice(&self) -> Option<&'static str> {
        Some("Run `pi install pi-mcp-adapter` to enable the Tome MCP server in Pi.")
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

    // -- Native agents (Phase 2) -----------------------------------------------

    fn supports_native_agents(&self) -> bool {
        true
    }

    fn agent_dir(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".pi/agents"))
    }

    fn agent_format(&self) -> Option<AgentFormat> {
        Some(AgentFormat::MarkdownYaml)
    }

    /// Pi subagent: MD+YAML; `name` AND `description` required (Pi silently
    /// skips a file missing either); `tools` is a comma-separated string;
    /// `model` is free-form (registry-resolved for tier aliases, else
    /// pass-through). Emit is inert until the user opts into `agentScope`.
    fn translate_agent(
        &self,
        canonical: &CanonicalAgent,
        clashes: bool,
        models: &crate::model_registry::ModelRegistry,
    ) -> Result<TranslatedAgent, TomeError> {
        let mut frontmatter: Vec<(String, serde_yaml::Value)> = Vec::new();
        let mut dropped: Vec<String> = Vec::new();

        let name = agents::displayed_name(&canonical.plugin, &canonical.name, clashes);
        frontmatter.push(("name".to_owned(), serde_yaml::Value::String(name.clone())));
        frontmatter.push((
            "description".to_owned(),
            serde_yaml::Value::String(agents::synthesize_description(canonical)),
        ));

        if let Some(src) = canonical.model.as_deref() {
            match agents::map_model(models, "pi", src) {
                Some(m) => frontmatter.push(("model".to_owned(), serde_yaml::Value::String(m))),
                None => dropped.push("model".to_owned()),
            }
        }
        if let Some(tools) = &canonical.tools {
            match agents::pi_tools(tools) {
                Some(s) => frontmatter.push(("tools".to_owned(), serde_yaml::Value::String(s))),
                None => dropped.push("tools".to_owned()),
            }
        }
        if canonical.disallowed_tools.is_some() {
            dropped.push("disallowedTools".to_owned());
        }
        if canonical.hooks.is_some() {
            dropped.push("hooks".to_owned());
        }
        if canonical.mcp_servers.is_some() {
            dropped.push("mcpServers".to_owned());
        }
        if canonical.permission_mode.is_some() {
            dropped.push("permissionMode".to_owned());
        }

        let rendered = agents::render_markdown_yaml(&frontmatter, &canonical.body);
        let filename = agent_filename(
            &canonical.plugin,
            &canonical.name,
            agent_extension(AgentFormat::MarkdownYaml),
        );
        Ok(TranslatedAgent {
            dir: PathBuf::from(".pi/agents"),
            filename,
            displayed_name: name,
            format: AgentFormat::MarkdownYaml,
            rendered,
            dropped_fields: dropped,
        })
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

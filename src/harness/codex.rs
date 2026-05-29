//! `codex` — OpenAI Codex CLI.
//!
//! Per research §R-8:
//!
//! - Per-user dir: `~/.codex/`.
//! - Rules-file target: `AGENTS.md` (Codex CLI only reads this file).
//! - Strategy: `BlockInExistingFile`, body style `AtInclude`.
//! - MCP config: `~/.codex/config.toml` (global; no per-project
//!   support).
//! - Parent key: `"mcp_servers"` — snake_case is the documented TOML
//!   convention here, distinct from the JSON harnesses' `"mcpServers"`.
//! - Guardrails target (Phase 6): `AGENTS.md` in-file region, no
//!   hooks-driven suppression — the trait default (`InFileRegion` on the
//!   rules-file target, `suppress_if_hooks_present = false`) is exactly
//!   correct, so no `guardrails_target` override is needed (FR-012).

use std::path::{Path, PathBuf};

use crate::error::TomeError;
use crate::harness::agents::{
    self, CanonicalAgent, TranslatedAgent, agent_extension, agent_filename,
};
use crate::harness::{
    AgentFormat, BlockBodyStyle, HarnessModule, McpConfigFormat, RulesFileStrategy,
};

/// Unit struct implementing [`HarnessModule`] for Codex CLI.
pub struct Codex;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const CODEX: Codex = Codex;

impl HarnessModule for Codex {
    fn name(&self) -> &'static str {
        "codex"
    }

    fn description(&self) -> &'static str {
        "OpenAI Codex CLI"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".codex").is_dir()
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("AGENTS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::AtInclude
    }

    fn mcp_config_path(&self, _project_root: &Path, home: &Path) -> PathBuf {
        home.join(".codex/config.toml")
    }

    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Toml
    }

    fn mcp_parent_key(&self) -> &'static str {
        "mcp_servers"
    }

    // -- Native agents (FR-030–FR-033, FR-036) ------------------------------

    fn supports_native_agents(&self) -> bool {
        true
    }

    fn agent_dir(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".codex/agents"))
    }

    fn agent_format(&self) -> Option<AgentFormat> {
        Some(AgentFormat::Toml)
    }

    /// Codex is OpenAI-vendored TOML. It keeps `name` + `description`; the
    /// body lands in a triple-quoted `developer_instructions` string
    /// (FR-033, R-14). `model` always DROPs (no Anthropic alias maps to an
    /// OpenAI id, FR-034). Read-only intent maps to
    /// `sandbox_mode = "read-only"` (FR-036). `tools` / `disallowedTools`
    /// and the privileged fields have no Codex dialect equivalent and DROP.
    fn translate_agent(
        &self,
        canonical: &CanonicalAgent,
        clashes: bool,
    ) -> Result<TranslatedAgent, TomeError> {
        let mut scalars: Vec<(String, String)> = Vec::new();
        let mut dropped: Vec<String> = Vec::new();

        let name = agents::displayed_name(&canonical.plugin, &canonical.name, clashes);
        scalars.push(("name".to_owned(), name.clone()));

        if let Some(desc) = &canonical.description {
            scalars.push(("description".to_owned(), desc.clone()));
        }

        // `model` is dropped wholesale for Codex (same-vendor-only policy).
        if canonical.model.is_some() {
            dropped.push("model".to_owned());
        }

        // Read-only intent → `sandbox_mode = "read-only"`. Indeterminate /
        // not-read-only → no `sandbox_mode` key (inherit the harness
        // default). C-2: when the intent is NOT reconstructed we record the
        // canonical SOURCE field(s) (`tools` / `disallowedTools`) below, not
        // the harness target name `sandbox_mode` — those source fields drop
        // wholesale for Codex regardless of read-only inference, so the drop
        // is already captured.
        //
        // C-1: only read-only *intent* is reconstructed here; a non-read-only
        // restrictive `tools` allowlist is dropped (full allowlist→sandbox
        // scoping is deferred).
        if let Some(true) = agents::infer_read_only(
            canonical.tools.as_deref(),
            canonical.disallowed_tools.as_deref(),
        ) {
            scalars.push(("sandbox_mode".to_owned(), "read-only".to_owned()));
        }

        // Tool posture + privileged fields have no Codex dialect carrier.
        if canonical.tools.is_some() {
            dropped.push("tools".to_owned());
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

        let rendered = agents::render_codex_toml(&scalars, &canonical.body);
        let filename = agent_filename(
            &canonical.plugin,
            &canonical.name,
            agent_extension(AgentFormat::Toml),
        );

        Ok(TranslatedAgent {
            dir: PathBuf::from(".codex/agents"),
            filename,
            displayed_name: name,
            format: AgentFormat::Toml,
            rendered,
            dropped_fields: dropped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_only_agent() -> CanonicalAgent {
        CanonicalAgent {
            catalog: "cat".into(),
            plugin: "myplugin".into(),
            name: "reviewer".into(),
            description: Some("Reviews code".into()),
            body: "You review.\nBe careful.\n".into(),
            model: Some("opus".into()),
            tools: Some(vec!["Read".into(), "Grep".into()]),
            disallowed_tools: None,
            hooks: Some(serde_json::json!({"x": 1})),
            mcp_servers: None,
            permission_mode: None,
        }
    }

    #[test]
    fn body_in_developer_instructions_model_drops_sandbox_read_only() {
        let t = CODEX.translate_agent(&read_only_agent(), false).unwrap();
        assert_eq!(t.filename, "myplugin__reviewer.toml");
        // Body lands in a triple-quoted developer_instructions string.
        assert!(
            t.rendered.contains("developer_instructions = \"\"\""),
            "body must be triple-quoted:\n{}",
            t.rendered
        );
        // Read-only tool posture → sandbox_mode = "read-only".
        assert!(t.rendered.contains("sandbox_mode = \"read-only\""));
        // model + tools + hooks have no Codex carrier → dropped + recorded.
        assert!(!t.rendered.contains("model"));
        assert!(t.dropped_fields.contains(&"model".to_owned()));
        assert!(t.dropped_fields.contains(&"tools".to_owned()));
        assert!(t.dropped_fields.contains(&"hooks".to_owned()));
    }
}

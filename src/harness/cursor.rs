//! `cursor` — Cursor IDE.
//!
//! Per research §R-8:
//!
//! - Per-user dir: `~/.cursor/`.
//! - Rules-file target: `<project>/.cursor/rules/TOME_SKILLS.md`
//!   (Tome-owned standalone file; no markers, no surrounding content).
//! - Strategy: `StandaloneFile`. `block_body_style()` is never
//!   consulted; the trait returns `Inline` as a harmless placeholder.
//! - MCP config: `<project>/.cursor/mcp.json` (per-project).
//! - Parent key: `"mcpServers"`.

use std::path::{Path, PathBuf};

use crate::error::TomeError;
use crate::harness::agents::{
    self, CanonicalAgent, TranslatedAgent, agent_extension, agent_filename,
};
use crate::harness::{
    AgentFormat, BlockBodyStyle, GuardrailsPlacement, GuardrailsTarget, HarnessModule,
    McpConfigFormat, RulesFileStrategy,
};

/// Unit struct implementing [`HarnessModule`] for Cursor.
pub struct Cursor;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const CURSOR: Cursor = Cursor;

impl HarnessModule for Cursor {
    fn name(&self) -> &'static str {
        "cursor"
    }

    fn description(&self) -> &'static str {
        "Cursor IDE"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".cursor").is_dir()
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join(".cursor/rules/TOME_SKILLS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::StandaloneFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        // Never consulted for `StandaloneFile`. Returning `Inline` is
        // documented as a harmless placeholder in the contract.
        BlockBodyStyle::Inline
    }

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join(".cursor/mcp.json")
    }

    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Json
    }

    fn mcp_parent_key(&self) -> &'static str {
        "mcpServers"
    }

    // -- Guardrails fallback (FR-011, FR-012, FR-015) -----------------------

    /// Cursor owns a fully Tome-managed standalone sibling for guardrails,
    /// `TOME_GUARDRAILS.md` — DISTINCT from the Phase 4 skills sibling
    /// (`TOME_SKILLS.md`). Each plugin is still individually marker-wrapped
    /// inside it so per-plugin removal works; the file is deleted entirely
    /// when no plugin contributes (FR-015). No hooks-driven suppression
    /// (Cursor has no native JSON hooks).
    fn guardrails_target(&self, project_root: &Path) -> GuardrailsTarget {
        GuardrailsTarget {
            placement: GuardrailsPlacement::StandaloneSibling {
                file: project_root.join(".cursor/rules/TOME_GUARDRAILS.md"),
            },
            suppress_if_hooks_present: false,
        }
    }

    // -- Native agents (FR-030–FR-032, FR-036) ------------------------------

    fn supports_native_agents(&self) -> bool {
        true
    }

    fn agent_dir(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".cursor/agents"))
    }

    fn agent_format(&self) -> Option<AgentFormat> {
        Some(AgentFormat::MarkdownYaml)
    }

    /// Cursor uses Markdown + YAML frontmatter with the body in the file
    /// body. It keeps `name` + `description` + `tools`. `model` DROPs (no
    /// same-vendor Cursor Anthropic id is enumerated yet, FR-034). Read-only
    /// intent maps to `readonly: true` (FR-036). The privileged fields and
    /// `disallowedTools` have no Cursor carrier and DROP.
    fn translate_agent(
        &self,
        canonical: &CanonicalAgent,
        clashes: bool,
    ) -> Result<TranslatedAgent, TomeError> {
        let mut frontmatter: Vec<(String, serde_yaml::Value)> = Vec::new();
        let mut dropped: Vec<String> = Vec::new();

        let name = agents::displayed_name(&canonical.plugin, &canonical.name, clashes);
        frontmatter.push(("name".to_owned(), serde_yaml::Value::String(name.clone())));

        if let Some(desc) = &canonical.description {
            frontmatter.push((
                "description".to_owned(),
                serde_yaml::Value::String(desc.clone()),
            ));
        }

        // `model` drops for Cursor (no enumerated same-vendor id yet).
        if canonical.model.is_some() {
            dropped.push("model".to_owned());
        }

        if let Some(tools) = &canonical.tools {
            frontmatter.push((
                "tools".to_owned(),
                serde_yaml::Value::Sequence(
                    tools
                        .iter()
                        .map(|t| serde_yaml::Value::String(t.clone()))
                        .collect(),
                ),
            ));
        }

        // Read-only intent → `readonly: true`. Indeterminate / not-read-only
        // → no `readonly` key (inherit Cursor's default). C-2: Cursor KEEPS
        // the `tools` allowlist verbatim and records `disallowedTools` as a
        // drop below, so when the read-only intent is not reconstructed the
        // responsible canonical SOURCE fields are already accounted for — we
        // do NOT record the harness target name `readonly`.
        //
        // C-1: only read-only *intent* is reconstructed; a non-read-only
        // restrictive `tools` allowlist is carried through as-is, not
        // translated into a finer-grained permission model.
        if let Some(true) = agents::infer_read_only(
            canonical.tools.as_deref(),
            canonical.disallowed_tools.as_deref(),
        ) {
            frontmatter.push(("readonly".to_owned(), serde_yaml::Value::Bool(true)));
        }

        // No Cursor carrier for these.
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
            dir: PathBuf::from(".cursor/agents"),
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
    fn keeps_tools_drops_model_infers_readonly() {
        let agent = CanonicalAgent {
            catalog: "cat".into(),
            plugin: "myplugin".into(),
            name: "reviewer".into(),
            description: Some("Reviews code".into()),
            body: "You review.\n".into(),
            model: Some("opus".into()),
            tools: Some(vec!["Read".into(), "Grep".into()]),
            disallowed_tools: None,
            hooks: None,
            mcp_servers: None,
            permission_mode: None,
        };
        let t = CURSOR.translate_agent(&agent, false).unwrap();
        assert_eq!(t.filename, "myplugin__reviewer.md");
        assert!(t.rendered.contains("tools:"), "tools kept:\n{}", t.rendered);
        assert!(t.rendered.contains("readonly: true"), "read-only inferred");
        // model drops for Cursor (no enumerated same-vendor id yet).
        assert!(!t.rendered.contains("model:"));
        assert!(t.dropped_fields.contains(&"model".to_owned()));
    }
}

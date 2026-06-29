//! `copilot` — GitHub Copilot in VS Code.
//!
//! Phase 11 (US1). Baseline integration: rules-file + MCP dialect only.
//! Phase 2 (native agents). Adds `supports_native_agents` + `translate_agent`
//! via the shared [`translate_copilot_agent`] free function, which `copilot-cli`
//! also calls — guaranteeing byte-identical output for the co-owned
//! `.github/agents/` directory.
//!
//! ## Native agents (co-ownership)
//!
//! Both `copilot` and `copilot-cli` emit to `.github/agents/` (GitHub's unified
//! `.agent.md` format, read by both the VS Code extension and the CLI). To
//! guarantee byte-identical output the translation logic lives in ONE free
//! function here; `copilot_cli` delegates to it via
//! `crate::harness::copilot::translate_copilot_agent`.
//!
//! Copilot specifics (verified live 2026-06-29):
//! - `description` required → [`agents::synthesize_description`].
//! - `name` emitted verbatim via [`agents::displayed_name`] (filename charset
//!   allows `_`/`-`, no slug needed).
//! - `model` OMITTED — display-name values are unstable; omission = inherit.
//!   NOT recorded as a dropped field.
//! - `tools` OMITTED — CC→Copilot tool-set mapping undefined; omission = inherit
//!   all. NEVER emit `tools: ['*']`. `tools` IS recorded in `dropped_fields`
//!   when the source agent carried any.
//! - `disallowedTools`, `hooks`, `mcpServers`, `permissionMode` dropped +
//!   recorded.
//! - Filename uses the `.agent.md` DOUBLE extension via
//!   `agents::agent_filename(&plugin, &name, "agent.md")`.
//!
//! - Per-user dir: `~/.vscode/` (the default name `copilot` does NOT match
//!   it, so `detect_path` is overridden).
//! - Rules-file target: `<project>/.github/copilot-instructions.md`,
//!   `BlockInExistingFile` · `Inline` (the trait default). SHARES this sink
//!   with the `copilot-cli` harness — the shared-sink single-region collapse
//!   writes exactly one Tome block.
//! - MCP config: `<project>/.vscode/mcp.json` (per-project).
//! - MCP dialect: JSON `servers` parent key + `CommandArgs`, `type:"stdio"`,
//!   omit-empty-`env`, no extra fields.

use std::path::{Path, PathBuf};

use crate::error::TomeError;
use crate::harness::agents::{self, CanonicalAgent, TranslatedAgent};
use crate::harness::{
    AgentFormat, EntryShape, FileFormat, HarnessModule, McpDialect, RulesFileStrategy, ServerType,
};

// =========================================================================
// Shared co-owner translation (used by both copilot + copilot-cli).
// =========================================================================

/// Translate a canonical agent into GitHub Copilot's native `.agent.md` form.
///
/// This is the SINGLE shared implementation for both the `copilot` (VS Code)
/// and `copilot-cli` harnesses, which co-own the `.github/agents/` directory.
/// Calling it from both `translate_agent` impls guarantees byte-identical
/// output and prevents the co-ownership reconciler rule from having two
/// divergent renderers to keep in sync.
///
/// ## Copilot specifics (verified 2026-06-29)
///
/// * `name` — verbatim via [`agents::displayed_name`]; Copilot's filename
///   charset allows `_` and `-` so no slug transformation is needed.
/// * `description` — REQUIRED; synthesised from frontmatter → first body line
///   → placeholder via [`agents::synthesize_description`].
/// * `model` — OMITTED (display-name values are unstable; omission = inherit).
///   NOT recorded as a dropped field — it is intentionally inherited, not
///   unknown/unsupported.
/// * `tools` — OMITTED (CC→Copilot tool-set mapping undefined; omission =
///   inherit all tools). NEVER emit `tools: ['*']`. When the source carried a
///   `tools` posture it IS recorded in `dropped_fields`.
/// * `disallowedTools`, `hooks`, `mcpServers`, `permissionMode` — dropped and
///   recorded.
/// * Filename — uses the `.agent.md` DOUBLE extension:
///   `<plugin>__<name>.agent.md` (NOT `agent_extension(MarkdownYaml)` which
///   would give `.md`).
/// * `dir` — `.github/agents`.
pub(crate) fn translate_copilot_agent(
    canonical: &CanonicalAgent,
    clashes: bool,
) -> Result<TranslatedAgent, TomeError> {
    let mut frontmatter: Vec<(String, serde_yaml::Value)> = Vec::new();
    let mut dropped: Vec<String> = Vec::new();

    let name = agents::displayed_name(&canonical.plugin, &canonical.name, clashes);
    frontmatter.push(("name".to_owned(), serde_yaml::Value::String(name.clone())));
    frontmatter.push((
        "description".to_owned(),
        serde_yaml::Value::String(agents::synthesize_description(canonical)),
    ));

    // model OMITTED — display-name values drift; omission = inherit. Not a drop.
    // tools OMITTED — CC→Copilot tool-set mapping is undefined; omission = all
    // tools. NEVER emit `*` (invalid for `tools`). Record the source posture.
    if canonical.tools.is_some() {
        dropped.push("tools".to_owned());
    }
    // model is intentionally inherited, not recorded as dropped.
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
    // `.agent.md` double extension (NOT agent_extension(MarkdownYaml) = "md").
    let filename = agents::agent_filename(&canonical.plugin, &canonical.name, "agent.md");
    Ok(TranslatedAgent {
        dir: PathBuf::from(".github/agents"),
        filename,
        displayed_name: name,
        format: AgentFormat::MarkdownYaml,
        rendered,
        dropped_fields: dropped,
    })
}

/// Unit struct implementing [`HarnessModule`] for GitHub Copilot (VS Code).
pub struct Copilot;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const COPILOT: Copilot = Copilot;

impl HarnessModule for Copilot {
    fn name(&self) -> &'static str {
        "copilot"
    }

    fn description(&self) -> &'static str {
        "GitHub Copilot (VS Code)"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".vscode").is_dir()
    }

    fn detect_path(&self, home: &Path) -> PathBuf {
        // `name()` is "copilot" but the per-user dir is `~/.vscode/`.
        home.join(".vscode")
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join(".github/copilot-instructions.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    // `block_body_style()` defaults to `Inline` (G3).

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join(".vscode/mcp.json")
    }

    /// Copilot (VS Code) MCP dialect: JSON `servers` parent key +
    /// `CommandArgs`, `type:"stdio"`, omit-empty-`env`, no extra fields.
    fn mcp_dialect(&self) -> McpDialect {
        McpDialect {
            file_format: FileFormat::Json,
            parent_key: "servers",
            entry_shape: EntryShape::CommandArgs,
            entry_type: Some(ServerType::Stdio),
            emit_env: false,
            extra_fields: &[],
        }
    }

    fn supports_native_agents(&self) -> bool {
        true
    }

    fn agent_dir(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".github/agents"))
    }

    fn agent_format(&self) -> Option<AgentFormat> {
        Some(AgentFormat::MarkdownYaml)
    }

    fn translate_agent(
        &self,
        canonical: &CanonicalAgent,
        clashes: bool,
        _models: &crate::model_registry::ModelRegistry,
    ) -> Result<TranslatedAgent, TomeError> {
        translate_copilot_agent(canonical, clashes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_and_paths() {
        assert_eq!(COPILOT.name(), "copilot");
        assert_eq!(
            COPILOT.detect_path(Path::new("/h")),
            Path::new("/h/.vscode"),
        );
        assert_eq!(
            COPILOT.rules_file_target(Path::new("/proj")),
            Path::new("/proj/.github/copilot-instructions.md"),
        );
        assert_eq!(
            COPILOT.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/proj/.vscode/mcp.json"),
        );
    }

    #[test]
    fn dialect_is_servers_command_args_type_stdio() {
        let d = COPILOT.mcp_dialect();
        assert_eq!(d.parent_key, "servers");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, Some(ServerType::Stdio));
        assert!(!d.emit_env);
        assert!(d.extra_fields.is_empty());
        assert!(!COPILOT.mcp_manual_only());
    }

    #[test]
    fn shares_rules_sink_with_copilot_cli() {
        // The single-region dedupe relies on these two pointing at the same
        // file. Pin the equality so a future path edit to either is caught.
        assert_eq!(
            COPILOT.rules_file_target(Path::new("/proj")),
            crate::harness::copilot_cli::COPILOT_CLI.rules_file_target(Path::new("/proj")),
        );
    }
}

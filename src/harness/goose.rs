//! `goose` — Block's Goose agent (an Open Plugins host).
//!
//! Phase 11 (US4). Unlike `generic` / `generic-op`, Goose IS auto-detectable
//! (`~/.config/goose` present), so it lives in [`super::SUPPORTED_HARNESSES`]
//! (NOT [`super::OPT_IN_TARGETS`]) and participates in detection + `--all`.
//!
//! Like `generic-op`, it integrates by emitting the self-contained Open Plugins
//! `tome-op` bundle via [`crate::harness::open_plugins::emit_tome_op`]:
//! [`open_plugins_root`] returns `Some`, so the orchestrator / `tome harness use`
//! dispatch to the atomic emitter rather than the per-sink rules/MCP loop.
//!
//! ## Chosen project plugin path
//!
//! `<project>/.config/goose/plugins/tome-op`. Goose reads project-local
//! configuration under `<project>/.config/goose/`; the Open Plugins convention
//! places installable plugins under a `plugins/` subdir. The exact on-disk
//! location Goose scans for project plugins is not pinned in a stable public
//! spec at authoring time, so the `#[ignore]`d live-probe below records the
//! manual confirmation a human must run against a real Goose install before this
//! ships.
//!
//! [`open_plugins_root`]: HarnessModule::open_plugins_root

use std::path::{Path, PathBuf};

use crate::error::TomeError;
use crate::harness::agents::{self, CanonicalAgent, TranslatedAgent};
use crate::harness::open_plugins::TOME_OP_NAME;
use crate::harness::{
    AgentFormat, BlockBodyStyle, EntryShape, FileFormat, HarnessModule, McpDialect,
    RulesFileStrategy,
};

/// Unit struct implementing [`HarnessModule`] for Goose.
pub struct Goose;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const GOOSE: Goose = Goose;

/// The bundle root for Goose: `<project>/.config/goose/plugins/tome-op`.
fn bundle_root(project_root: &Path) -> PathBuf {
    project_root
        .join(".config/goose/plugins")
        .join(TOME_OP_NAME)
}

impl HarnessModule for Goose {
    fn name(&self) -> &'static str {
        "goose"
    }

    fn description(&self) -> &'static str {
        "Goose agent"
    }

    fn detect(&self, home: &Path) -> bool {
        // Goose stores per-user config under XDG `~/.config/goose/`.
        home.join(".config/goose").is_dir()
    }

    fn detect_path(&self, home: &Path) -> PathBuf {
        home.join(".config/goose")
    }

    /// `Some` → dispatch to the `open_plugins` emitter instead of the per-sink
    /// loop. Goose is detectable, but its integration is the `tome-op` bundle.
    fn open_plugins_root(&self, project_root: &Path) -> Option<PathBuf> {
        Some(bundle_root(project_root))
    }

    // Bundle-INTERNAL sinks (informational for `tome harness info goose`; not
    // consulted on the open-plugins dispatch path — the emitter owns the whole
    // bundle atomically).

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        bundle_root(project_root).join("AGENTS.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::Inline
    }

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        bundle_root(project_root).join(".mcp.json")
    }

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

    fn supports_native_agents(&self) -> bool {
        true
    }

    fn agent_dir(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".agents/agents"))
    }

    fn agent_format(&self) -> Option<AgentFormat> {
        Some(AgentFormat::MarkdownYaml)
    }

    /// Goose Custom Agent: MD+YAML; `name` required (underscores OK → emit the
    /// displayed name verbatim); `description` optional; `model` is a bare
    /// provider id (registry-resolved); NO per-agent tools (session-inherited).
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
        if let Some(desc) = &canonical.description {
            frontmatter.push((
                "description".to_owned(),
                serde_yaml::Value::String(desc.clone()),
            ));
        }
        if let Some(src) = canonical.model.as_deref() {
            match agents::map_model(models, "goose", src) {
                Some(m) => {
                    frontmatter.push(("model".to_owned(), serde_yaml::Value::String(m)));
                }
                None => dropped.push("model".to_owned()),
            }
        }
        // Goose Custom Agents inherit session extensions/tools — no carrier.
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

        let rendered = agents::render_markdown_yaml(&frontmatter, &canonical.body);
        let filename = agents::agent_filename(
            &canonical.plugin,
            &canonical.name,
            agents::agent_extension(AgentFormat::MarkdownYaml),
        );
        Ok(TranslatedAgent {
            dir: PathBuf::from(".agents/agents"),
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
    fn identity_and_detect_path() {
        assert_eq!(GOOSE.name(), "goose");
        assert_eq!(
            GOOSE.detect_path(Path::new("/h")),
            Path::new("/h/.config/goose"),
        );
        // Goose is NOT an opt-in target — it is detectable.
        assert!(!GOOSE.is_opt_in_target());
    }

    #[test]
    fn detect_true_when_config_goose_exists() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(!GOOSE.detect(tmp.path()));
        std::fs::create_dir_all(tmp.path().join(".config/goose")).unwrap();
        assert!(GOOSE.detect(tmp.path()));
    }

    #[test]
    fn open_plugins_root_is_under_config_goose_plugins() {
        assert_eq!(
            GOOSE.open_plugins_root(Path::new("/proj")),
            Some(PathBuf::from("/proj/.config/goose/plugins/tome-op")),
        );
    }

    /// Live-probe gate (T073): NOT run in CI. A human must confirm against a real
    /// Goose install that it discovers + installs an Open Plugins `tome-op`
    /// bundle from `<project>/.config/goose/plugins/tome-op` before this ships.
    #[test]
    #[ignore = "live-probe: confirm Goose reads project .config/goose/plugins/tome-op"]
    fn goose_reads_project_plugin_dir_live_probe() {
        // No automated body — see the doc comment for the manual checklist.
    }
}

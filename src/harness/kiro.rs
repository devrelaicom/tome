//! `kiro` — AWS Kiro IDE.
//!
//! Phase 11 (US1). Baseline integration: standalone rules file (with
//! Tome-owned front-matter) + MCP dialect.
//!
//! - Per-user dir: `~/.kiro/` (the default `detect_path`).
//! - Rules-file sink: a dedicated `StandaloneFile` steering file at
//!   `<project>/.kiro/steering/tome.md`, fronted by a Tome-owned YAML
//!   header `inclusion: always` (so Kiro applies it on every session). A
//!   namespaced file under Kiro's own dir — Tome never clobbers a developer
//!   steering file.
//! - MCP config: `<project>/.kiro/settings/mcp.json` (per-project).
//! - MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`,
//!   `emit_env:true` (`"env": {}`), no extra fields. (Kiro additionally
//!   tolerates `disabled`/`autoApprove` keys if pre-existing — the lenient
//!   parse preserves them; Tome never adds them.)

use std::path::{Path, PathBuf};

use crate::error::TomeError;
use crate::harness::agents::{
    self, CanonicalAgent, TranslatedAgent, agent_extension, agent_filename,
};
use crate::harness::{
    AgentFormat, EntryShape, FileFormat, GuardrailsPlacement, GuardrailsTarget, HarnessModule,
    McpDialect, RulesFileStrategy, RulesFrontmatter,
};

/// Unit struct implementing [`HarnessModule`] for Kiro.
pub struct Kiro;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const KIRO: Kiro = Kiro;

impl HarnessModule for Kiro {
    fn name(&self) -> &'static str {
        "kiro"
    }

    fn description(&self) -> &'static str {
        "AWS Kiro IDE"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".kiro").is_dir()
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        // The dedicated steering file Tome fully owns. `rules_namespaced_file`
        // returns the same path; for Kiro (a `StandaloneFile` harness) the two
        // are identical, but declaring the namespaced accessor signals the
        // never-clobber intent to the G3 sink.
        project_root.join(".kiro/steering/tome.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::StandaloneFile
    }

    fn rules_namespaced_file(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".kiro/steering/tome.md"))
    }

    /// Kiro applies a steering file on every session iff its front-matter
    /// declares `inclusion: always`.
    fn rules_frontmatter(&self) -> Option<RulesFrontmatter> {
        Some(RulesFrontmatter {
            fields: &[("inclusion", "always")],
        })
    }

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join(".kiro/settings/mcp.json")
    }

    /// Guardrails land in a Tome-owned standalone sibling (PW3), distinct from
    /// the standalone steering file `.kiro/steering/tome.md` — both inside
    /// Kiro's own `.kiro/steering/` dir. Without it the standalone rules writer
    /// and the in-file guardrails region would share one path and clobber each
    /// other every sync. Mirrors `cursor`.
    fn guardrails_target(&self, project_root: &Path) -> GuardrailsTarget {
        GuardrailsTarget {
            placement: GuardrailsPlacement::StandaloneSibling {
                file: project_root.join(".kiro/steering/TOME_GUARDRAILS.md"),
            },
            suppress_if_hooks_present: false,
        }
    }

    /// Kiro's MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`,
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

    // -- Native agents (Phase 2, Task 8) -------------------------------------

    fn supports_native_agents(&self) -> bool {
        true
    }

    fn agent_dir(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".kiro/agents"))
    }

    fn agent_format(&self) -> Option<AgentFormat> {
        Some(AgentFormat::MarkdownYaml)
    }

    /// Kiro IDE: MD+YAML; `name` required, lowercase+hyphens only; `description`
    /// optional; `tools` are category tags (`read`/`write`/`shell`/`web`);
    /// `model` is DROPPED (Kiro's ids are dotted and the field is ignored in
    /// programmatic subagent dispatch, issue #6637 — emitting one errors).
    fn translate_agent(
        &self,
        canonical: &CanonicalAgent,
        clashes: bool,
        models: &crate::model_registry::ModelRegistry,
    ) -> Result<TranslatedAgent, TomeError> {
        let mut frontmatter: Vec<(String, serde_yaml::Value)> = Vec::new();
        let mut dropped: Vec<String> = Vec::new();

        let displayed = agents::displayed_name(&canonical.plugin, &canonical.name, clashes);
        let name = agents::slugify_agent_name(&displayed, false); // hyphens only
        frontmatter.push(("name".to_owned(), serde_yaml::Value::String(name)));
        if let Some(desc) = &canonical.description {
            frontmatter.push((
                "description".to_owned(),
                serde_yaml::Value::String(desc.clone()),
            ));
        }
        // `map_model(.., "kiro", ..)` is always None → record the drop.
        if let Some(src) = canonical.model.as_deref() {
            match agents::map_model(models, "kiro", src) {
                Some(m) => frontmatter.push(("model".to_owned(), serde_yaml::Value::String(m))),
                None => dropped.push("model".to_owned()),
            }
        }
        if let Some(tools) = &canonical.tools {
            let tags = agents::kiro_tools(tools);
            if tags.is_empty() {
                dropped.push("tools".to_owned());
            } else {
                let seq = tags.into_iter().map(serde_yaml::Value::String).collect();
                frontmatter.push(("tools".to_owned(), serde_yaml::Value::Sequence(seq)));
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
            dir: PathBuf::from(".kiro/agents"),
            filename,
            displayed_name: displayed,
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
        assert_eq!(KIRO.name(), "kiro");
        assert_eq!(KIRO.detect_path(Path::new("/h")), Path::new("/h/.kiro"));
        assert_eq!(
            KIRO.rules_file_target(Path::new("/proj")),
            Path::new("/proj/.kiro/steering/tome.md"),
        );
        assert_eq!(
            KIRO.rules_namespaced_file(Path::new("/proj")),
            Some(PathBuf::from("/proj/.kiro/steering/tome.md")),
        );
        assert_eq!(
            KIRO.mcp_config_path(Path::new("/proj"), Path::new("/h")),
            Path::new("/proj/.kiro/settings/mcp.json"),
        );
    }

    #[test]
    fn standalone_with_inclusion_always_frontmatter() {
        assert_eq!(
            KIRO.rules_file_strategy(),
            RulesFileStrategy::StandaloneFile
        );
        let fm = KIRO.rules_frontmatter().expect("kiro has frontmatter");
        assert_eq!(fm.fields, &[("inclusion", "always")]);
    }

    #[test]
    fn dialect_is_mcpservers_command_args_emit_env() {
        let d = KIRO.mcp_dialect();
        assert_eq!(d.parent_key, "mcpServers");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, None);
        assert!(d.emit_env);
        assert!(!KIRO.mcp_manual_only());
    }

    /// PW3 (phase-wide): guardrails land in a Tome-owned StandaloneSibling that
    /// is NOT the standalone steering-file path.
    #[test]
    fn guardrails_sibling_differs_from_rules_file() {
        let proj = Path::new("/proj");
        let rules = KIRO.rules_file_target(proj);
        match KIRO.guardrails_target(proj).placement {
            GuardrailsPlacement::StandaloneSibling { file } => {
                assert_eq!(
                    file,
                    PathBuf::from("/proj/.kiro/steering/TOME_GUARDRAILS.md")
                );
                assert_ne!(file, rules);
            }
            other => panic!("expected StandaloneSibling, got {other:?}"),
        }
    }
}

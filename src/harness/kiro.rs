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

use crate::harness::{
    EntryShape, FileFormat, GuardrailsPlacement, GuardrailsTarget, HarnessModule, McpDialect,
    RulesFileStrategy, RulesFrontmatter,
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

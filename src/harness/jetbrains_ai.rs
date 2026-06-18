//! `jetbrains-ai` — JetBrains AI Assistant.
//!
//! Phase 11 (US1). UI-only MCP harness: rules-file (standalone + Tome-owned
//! front-matter) only — there is NO writable MCP config file.
//!
//! - Per-user dir: `~/.aiassistant/` (the default name `jetbrains-ai` does
//!   NOT match it, so `detect_path` is overridden).
//! - Rules-file sink: a `StandaloneFile` at
//!   `<project>/.aiassistant/rules/tome.md`, fronted by a Tome-owned YAML
//!   header marking the rule as Always-applied. A namespaced file under
//!   AI Assistant's own dir — Tome never clobbers a developer rule file.
//! - MCP: **manual-only** ([`HarnessModule::mcp_manual_only`] → `true`).
//!   AI Assistant configures MCP servers through its Settings UI; there is
//!   no file Tome can own, so the sync skips the MCP sink entirely.
//!   `mcp_config_path` still returns a path (the trait requires it) but it
//!   is never read or written for a manual-only harness. The "paste this
//!   snippet into Settings" recovery snippet IS surfaced (US5): both
//!   `tome harness info jetbrains-ai` and the `tome harness use` notice
//!   render it via [`mcp_dialect`](JetbrainsAi::mcp_dialect) — which is why
//!   that dialect is an explicit override (it must carry `emit_env:true`),
//!   NOT the LEGACY default.

use std::path::{Path, PathBuf};

use crate::harness::{
    EntryShape, FileFormat, GuardrailsPlacement, GuardrailsTarget, HarnessModule, McpDialect,
    RulesFileStrategy, RulesFrontmatter,
};

/// Unit struct implementing [`HarnessModule`] for JetBrains AI Assistant.
pub struct JetbrainsAi;

/// Static instance used by the [`SUPPORTED_HARNESSES`] registry.
///
/// [`SUPPORTED_HARNESSES`]: super::SUPPORTED_HARNESSES
pub const JETBRAINS_AI: JetbrainsAi = JetbrainsAi;

impl HarnessModule for JetbrainsAi {
    fn name(&self) -> &'static str {
        "jetbrains-ai"
    }

    fn description(&self) -> &'static str {
        "JetBrains AI Assistant"
    }

    fn detect(&self, home: &Path) -> bool {
        home.join(".aiassistant").is_dir()
    }

    fn detect_path(&self, home: &Path) -> PathBuf {
        // `name()` is "jetbrains-ai" but the per-user dir is `~/.aiassistant/`.
        // Override the default so `tome harness info`'s reported path matches
        // what `detect` actually probes.
        home.join(".aiassistant")
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join(".aiassistant/rules/tome.md")
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::StandaloneFile
    }

    fn rules_namespaced_file(&self, project_root: &Path) -> Option<PathBuf> {
        Some(project_root.join(".aiassistant/rules/tome.md"))
    }

    /// AI Assistant's Always apply-mode marker.
    ///
    /// NOTE: the exact front-matter key for AI Assistant's "Always" apply mode
    /// is pending the T087 live-probe — `apply: always` is the planned value;
    /// the live probe confirms (or corrects) it against a real AI Assistant
    /// install before this ships.
    fn rules_frontmatter(&self) -> Option<RulesFrontmatter> {
        Some(RulesFrontmatter {
            fields: &[("apply", "always")],
        })
    }

    /// Guardrails land in a Tome-owned standalone sibling (PW3), distinct from
    /// the standalone rules file `.aiassistant/rules/tome.md` — both inside AI
    /// Assistant's own `.aiassistant/rules/` dir. Without it the standalone rules
    /// writer and the in-file guardrails region would share one path and clobber
    /// each other every sync. Mirrors `cursor`.
    fn guardrails_target(&self, project_root: &Path) -> GuardrailsTarget {
        GuardrailsTarget {
            placement: GuardrailsPlacement::StandaloneSibling {
                file: project_root.join(".aiassistant/rules/TOME_GUARDRAILS.md"),
            },
            suppress_if_hooks_present: false,
        }
    }

    /// Manual-only: AI Assistant configures MCP through its Settings UI, so
    /// Tome writes no MCP file. The sync skips the MCP sink entirely; the
    /// "paste this snippet" notice is a US5 fast-follow.
    fn mcp_manual_only(&self) -> bool {
        true
    }

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        // Never read/written for a manual-only harness, but the trait requires
        // a value. Point it at AI Assistant's own dir so any future diagnostic
        // surface names a plausible location rather than a bogus one.
        project_root.join(".aiassistant/mcp.json")
    }

    /// AI Assistant's MCP dialect: JSON `mcpServers` + `CommandArgs`, no `type`,
    /// `emit_env:true` (`"env": {}` per the contract — matching its US1 default-
    /// shape peers devin/junie/kiro/cline/antigravity/pi), no extra fields.
    ///
    /// Because jetbrains-ai is `mcp_manual_only`, the sync orchestrator NEVER
    /// reads or writes its MCP config file — this dialect is consulted ONLY by
    /// the paste-able recovery snippet (`mcp_config::render_entry_snippet`, via
    /// `tome harness info` and the `use` manual-MCP notice). It must therefore
    /// carry `emit_env:true` so the snippet a user pastes into AI Assistant's
    /// Settings UI matches the contract's default `mcpServers` shape (the
    /// LEGACY default's `emit_env:false` would omit `"env": {}` and diverge
    /// from its peers).
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
        assert_eq!(JETBRAINS_AI.name(), "jetbrains-ai");
        // detect_path must point at the real per-user dir, not `~/.jetbrains-ai`.
        assert_eq!(
            JETBRAINS_AI.detect_path(Path::new("/h")),
            Path::new("/h/.aiassistant"),
        );
        assert_eq!(
            JETBRAINS_AI.rules_file_target(Path::new("/proj")),
            Path::new("/proj/.aiassistant/rules/tome.md"),
        );
    }

    #[test]
    fn standalone_with_always_apply_frontmatter() {
        assert_eq!(
            JETBRAINS_AI.rules_file_strategy(),
            RulesFileStrategy::StandaloneFile,
        );
        let fm = JETBRAINS_AI.rules_frontmatter().expect("has frontmatter");
        assert_eq!(fm.fields, &[("apply", "always")]);
    }

    /// PW3 (phase-wide): guardrails land in a Tome-owned StandaloneSibling that
    /// is NOT the standalone rules-file path.
    #[test]
    fn guardrails_sibling_differs_from_rules_file() {
        let proj = Path::new("/proj");
        let rules = JETBRAINS_AI.rules_file_target(proj);
        match JETBRAINS_AI.guardrails_target(proj).placement {
            GuardrailsPlacement::StandaloneSibling { file } => {
                assert_eq!(
                    file,
                    PathBuf::from("/proj/.aiassistant/rules/TOME_GUARDRAILS.md")
                );
                assert_ne!(file, rules);
            }
            other => panic!("expected StandaloneSibling, got {other:?}"),
        }
    }

    #[test]
    fn mcp_is_manual_only() {
        assert!(JETBRAINS_AI.mcp_manual_only());
    }

    /// M1 (US5 closeout): jetbrains-ai's dialect is the contract default
    /// `mcpServers` CommandArgs shape with `emit_env:true` — matching its US1
    /// peers (devin/junie/kiro/cline/antigravity/pi), NOT the LEGACY default
    /// (`emit_env:false`). It is consulted ONLY by the recovery snippet.
    #[test]
    fn dialect_is_mcpservers_command_args_emit_env() {
        let d = JETBRAINS_AI.mcp_dialect();
        assert_eq!(d.file_format, FileFormat::Json);
        assert_eq!(d.parent_key, "mcpServers");
        assert_eq!(d.entry_shape, EntryShape::CommandArgs);
        assert_eq!(d.entry_type, None);
        assert!(d.emit_env, "snippet must carry env:{{}}");
        assert!(d.extra_fields.is_empty());
    }

    /// M1: the EXACT recovery-snippet bytes jetbrains-ai renders — `env:{}`
    /// present (the whole point of the explicit `emit_env:true` dialect).
    #[test]
    fn recovery_snippet_carries_empty_env_exact_bytes() {
        use crate::harness::mcp_config::{TomeEntry, render_entry_snippet};

        let entry = TomeEntry::new(
            "tome".to_string(),
            vec![
                "mcp".to_string(),
                "--workspace".to_string(),
                "demo".to_string(),
                "--harness".to_string(),
                "jetbrains-ai".to_string(),
            ],
        );
        let snippet = render_entry_snippet(&JETBRAINS_AI.mcp_dialect(), &entry);
        assert_eq!(
            snippet,
            "{\n  \"mcpServers\": {\n    \"tome\": {\n      \"command\": \"tome\",\n      \"args\": [\n        \"mcp\",\n        \"--workspace\",\n        \"demo\",\n        \"--harness\",\n        \"jetbrains-ai\"\n      ],\n      \"env\": {}\n    }\n  }\n}\n",
        );
    }

    /// Live-probe gate (T087): NOT run in CI. A human must confirm against a
    /// real JetBrains AI Assistant install the exact front-matter key/value for
    /// the "Always" apply mode — `apply: always` is the PLANNED value but is
    /// UNVERIFIED. Mirrors the zed/antigravity live-probe gates.
    #[test]
    #[ignore = "live-probe: confirm AI Assistant Always apply-mode front-matter key/value"]
    fn jetbrains_apply_always_frontmatter_live_probe() {
        // No automated body — see the doc comment for the manual checklist a
        // human runs against a real AI Assistant install. Present so the gate is
        // discoverable via `cargo test -- --ignored`.
    }
}

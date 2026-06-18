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
//!   no file Tome can own, so the sync skips the MCP sink entirely. The
//!   "paste this snippet into Settings" notice is a separate US5 concern.
//!   `mcp_config_path` still returns a path (the trait requires it) but it
//!   is never read or written for a manual-only harness.

use std::path::{Path, PathBuf};

use crate::harness::{HarnessModule, RulesFileStrategy, RulesFrontmatter};

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

    // MCP dialect: the default ([`McpDialect::LEGACY`]) is fine — it is never
    // consulted because `mcp_manual_only()` is `true`.
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

    #[test]
    fn mcp_is_manual_only() {
        assert!(JETBRAINS_AI.mcp_manual_only());
    }
}

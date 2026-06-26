//! Deterministic test-only `HarnessModule`.
//!
//! This module ships in the library so integration tests under
//! `tests/` can import it (they consume the library as an external
//! crate and therefore cannot reach `#[cfg(test)]`-gated hooks). It is
//! NOT registered in [`super::SUPPORTED_HARNESSES`]; production code
//! never references it. The first consumer is US1.b-3's harness sync
//! tests, which install it via [`super::HARNESS_MODULES_OVERRIDE`] to
//! exercise the dispatch path without binding to any of the five real
//! harnesses.
//!
//! ## Configurability (Phase 6 / F3, research R-16)
//!
//! Phase 6 adds hooks / guardrails / native-agent dispatch to the trait.
//! `StubHarness` grew from a unit struct into a struct with `Default`-able
//! config fields so a test can drive any combination of the new
//! capabilities (suppression transitions, removal globs, forward-progress)
//! against a synthetic registry. The field defaults reproduce the original
//! unit-struct behaviour *plus the trait's safe defaults*, so
//! `StubHarness::default()` is the drop-in replacement for the old bare
//! `StubHarness` literal. Builder setters (`with_*`) flip individual
//! capabilities without spelling out the whole struct.
//!
//! Behaviour (defaults):
//!
//! - `name()`            → `"stub"`
//! - `description()`     → `"deterministic test-only harness"`
//! - `detect()`          → always `true`
//! - `rules_file_target` → `<project>/STUB_RULES.md`
//! - `rules_file_strategy` → `BlockInExistingFile`
//! - `block_body_style`  → `Inline`
//! - `mcp_config_path`   → `<project>/stub.mcp.json`
//! - `mcp_dialect`       → the trait default (`McpDialect::LEGACY` —
//!   JSON `mcpServers` + `CommandArgs`)
//! - Phase 6 capabilities → the trait's safe defaults (GuardrailsOnly,
//!   no hook settings path, in-file guardrails region without suppression,
//!   no native agents) unless overridden via `with_*`.

use std::path::{Path, PathBuf};

use crate::harness::agents::{CanonicalAgent, TranslatedAgent};
use crate::harness::{
    AgentFormat, BlockBodyStyle, GuardrailsPlacement, GuardrailsTarget, HarnessModule,
    HooksStrategy, RulesFileStrategy, SessionSteering,
};

/// Test-configurable [`HarnessModule`]. All fields default to the original
/// unit-struct + safe-default behaviour; flip individual capabilities via
/// the `with_*` setters.
#[doc(hidden)]
#[derive(Debug, Clone)]
pub struct StubHarness {
    hooks_strategy: HooksStrategy,
    /// When `true`, `hook_settings_path` returns
    /// `<project>/.stub/settings.local.json`; else `None`.
    hook_settings: bool,
    /// `None` ⇒ use the trait default (`InFileRegion` on the rules-file
    /// target, no suppression). `Some(_)` ⇒ return the canned target.
    guardrails_target: Option<GuardrailsTarget>,
    supports_native_agents: bool,
    /// `None` ⇒ derive `<project>/.stub/agents/` when native agents are
    /// supported, else honour the supported flag. `Some(_)` ⇒ canned dir.
    agent_dir: Option<PathBuf>,
    agent_format: Option<AgentFormat>,
    /// Canned `translate_agent` result, cloned per call.
    translation: Option<TranslatedAgent>,
    /// Phase 11 (G2): canned `session_steering()`. Defaults to
    /// [`SessionSteering::None`] (the trait floor).
    session_steering: SessionSteering,
    /// Phase 11 (US6): override the harness `name()`. Defaults to `"stub"`.
    /// Multi-harness selection tests install several differently-named stubs.
    name: &'static str,
    /// Phase 11 (US6): canned `detect()` result. Defaults to `true` (the
    /// original behaviour). Selection's detected-default path filters on this.
    detect: bool,
    /// Phase 11 (US6): when `true`, `mcp_config_path` points under a parent the
    /// fixture makes a SYMLINK (`<project>/<NAME>_MCP_BROKEN/mcp.json`, with
    /// `<project>/<NAME>_MCP_BROKEN` a symlink to a real dir). The
    /// symlink-refusing MCP write guard rejects the LIVE write (driving the
    /// `use` forward-progress per-harness failure path); the non-live `clean`
    /// read of the absent file is a no-op, so a full-registry reconcile that
    /// visits this stub on its remove path does NOT fail. Isolating the failure
    /// to the MCP sink (which never shares the rules/guardrails path) lets a
    /// co-selected healthy stub still succeed.
    fail_mcp: bool,
}

impl Default for StubHarness {
    fn default() -> Self {
        Self {
            hooks_strategy: HooksStrategy::GuardrailsOnly,
            hook_settings: false,
            guardrails_target: None,
            supports_native_agents: false,
            agent_dir: None,
            agent_format: None,
            translation: None,
            session_steering: SessionSteering::None,
            name: "stub",
            detect: true,
            fail_mcp: false,
        }
    }
}

impl StubHarness {
    /// Construct a stub with all defaults (equivalent to the old bare
    /// `StubHarness`).
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_hooks_strategy(mut self, strategy: HooksStrategy) -> Self {
        self.hooks_strategy = strategy;
        self
    }

    /// Make `hook_settings_path` return a real path (under `.stub/`).
    pub fn with_hook_settings(mut self) -> Self {
        self.hook_settings = true;
        self
    }

    pub fn with_guardrails_target(mut self, target: GuardrailsTarget) -> Self {
        self.guardrails_target = Some(target);
        self
    }

    pub fn with_native_agents(mut self, format: AgentFormat) -> Self {
        self.supports_native_agents = true;
        self.agent_format = Some(format);
        self
    }

    pub fn with_agent_dir(mut self, dir: PathBuf) -> Self {
        self.agent_dir = Some(dir);
        self
    }

    pub fn with_translation(mut self, translated: TranslatedAgent) -> Self {
        self.translation = Some(translated);
        self
    }

    /// Drive the Phase 11 `session_steering()` (the `CommandHook` /
    /// `TsPlugin` reconcilers).
    pub fn with_session_steering(mut self, steering: SessionSteering) -> Self {
        self.session_steering = steering;
        self
    }

    /// Phase 11 (US6): override the harness `name()` (a `&'static str`, so
    /// multi-harness selection tests can install several distinct stubs).
    pub fn with_name(mut self, name: &'static str) -> Self {
        self.name = name;
        self
    }

    /// Phase 11 (US6): set the canned `detect()` result (defaults to `true`).
    pub fn with_detect(mut self, detect: bool) -> Self {
        self.detect = detect;
        self
    }

    /// Phase 11 (US6): make the LIVE MCP write fail, to drive the `use`
    /// forward-progress per-harness failure path. The fixture must plant
    /// `<project>/<NAME>_MCP_BROKEN` as a symlink (to a real dir) so the
    /// symlink-refusing MCP write guard rejects the write while the non-live
    /// clean read stays a no-op (and the rules/guardrails sinks, which target a
    /// DIFFERENT clean path, succeed for a co-selected healthy stub).
    pub fn with_failing_mcp(mut self) -> Self {
        self.fail_mcp = true;
        self
    }
}

impl HarnessModule for StubHarness {
    fn name(&self) -> &'static str {
        self.name
    }

    fn description(&self) -> &'static str {
        "deterministic test-only harness"
    }

    fn detect(&self, _home: &Path) -> bool {
        self.detect
    }

    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        // The default-named stub keeps the original `STUB_RULES.md` path (other
        // test harnesses intentionally share it). A custom-named stub gets a
        // name-scoped path so distinct stubs in one override don't collide.
        if self.name == "stub" {
            project_root.join("STUB_RULES.md")
        } else {
            project_root.join(format!("{}_RULES.md", self.name))
        }
    }

    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }

    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::Inline
    }

    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        if self.fail_mcp {
            // Parent is a fixture-planted symlink → the LIVE MCP write is
            // refused by the symlink guard (a non-live clean read is a no-op).
            return project_root
                .join(format!("{}_MCP_BROKEN", self.name))
                .join("mcp.json");
        }
        // Default keeps `stub.mcp.json`; custom-named stubs get a name-scoped
        // file so several stubs in one override don't write the same path.
        if self.name == "stub" {
            project_root.join("stub.mcp.json")
        } else {
            project_root.join(format!("{}.mcp.json", self.name))
        }
    }

    // MCP dialect: the trait default ([`McpDialect::LEGACY`] — JSON
    // `mcpServers` + `CommandArgs`) is exactly the stub's shape.

    fn hooks_strategy(&self) -> HooksStrategy {
        self.hooks_strategy
    }

    fn hook_settings_path(&self, project_root: &Path) -> Option<PathBuf> {
        self.hook_settings
            .then(|| project_root.join(".stub/settings.local.json"))
    }

    fn guardrails_target(&self, project_root: &Path) -> GuardrailsTarget {
        self.guardrails_target
            .clone()
            .unwrap_or_else(|| GuardrailsTarget {
                placement: GuardrailsPlacement::InFileRegion {
                    file: self.rules_file_target(project_root),
                },
                suppress_if_hooks_present: false,
            })
    }

    fn supports_native_agents(&self) -> bool {
        self.supports_native_agents
    }

    fn agent_dir(&self, project_root: &Path) -> Option<PathBuf> {
        if let Some(dir) = &self.agent_dir {
            return Some(dir.clone());
        }
        self.supports_native_agents
            .then(|| project_root.join(".stub/agents"))
    }

    fn agent_format(&self) -> Option<AgentFormat> {
        self.agent_format
    }

    fn session_steering(&self) -> SessionSteering {
        self.session_steering.clone()
    }

    fn translate_agent(
        &self,
        canonical: &CanonicalAgent,
        clashes: bool,
        _models: &crate::model_registry::ModelRegistry,
    ) -> Result<TranslatedAgent, crate::error::TomeError> {
        Ok(self.translation.clone().unwrap_or_else(|| {
            // Minimal deterministic translation when no canned result is
            // supplied: echo the canonical name + body into a Markdown body.
            // Honours `clashes` so dispatch tests can assert the displayed
            // name is plugin-prefixed without a canned result.
            let format = self.agent_format.unwrap_or(AgentFormat::MarkdownYaml);
            let displayed_name = if clashes {
                format!("{}-{}", canonical.plugin, canonical.name)
            } else {
                canonical.name.clone()
            };
            TranslatedAgent {
                dir: PathBuf::from(".stub/agents"),
                filename: format!("{}__{}.md", canonical.plugin, canonical.name),
                displayed_name,
                format,
                rendered: canonical.body.clone(),
                dropped_fields: Vec::new(),
            }
        }))
    }
}

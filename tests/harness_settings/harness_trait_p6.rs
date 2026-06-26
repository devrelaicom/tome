//! Phase 6 / F3 — `HarnessModule` trait extensions (harness-modules-p6.md).
//!
//! Two surfaces:
//! 1. A minimal harness that implements only the pre-Phase-6 required
//!    methods inherits the safe Phase 6 defaults (GuardrailsOnly, no hook
//!    settings path, in-file guardrails region without suppression, no
//!    native agents).
//! 2. `StubHarness` exposes the new capabilities per-test and dispatches
//!    correctly through `with_effective_modules` + `HARNESS_MODULES_OVERRIDE`.

use std::path::{Path, PathBuf};

use tome::harness::agents::CanonicalAgent;
use tome::harness::{
    AgentFormat, BlockBodyStyle, GuardrailsPlacement, GuardrailsTarget, HarnessModule,
    HooksStrategy, McpConfigFormat, RulesFileStrategy, StubHarness, with_effective_modules,
};

/// RAII guard mirroring the documented pattern in `src/harness/mod.rs`.
struct HarnessModulesGuard;

impl HarnessModulesGuard {
    fn install(modules: Vec<Box<dyn HarnessModule>>) -> Self {
        *tome::harness::HARNESS_MODULES_OVERRIDE.write().unwrap() = Some(modules);
        Self
    }
}

impl Drop for HarnessModulesGuard {
    fn drop(&mut self) {
        *tome::harness::HARNESS_MODULES_OVERRIDE.write().unwrap() = None;
    }
}

/// Implements only the pre-Phase-6 required methods, so it inherits every
/// Phase 6 default.
struct MinimalHarness;

impl HarnessModule for MinimalHarness {
    fn name(&self) -> &'static str {
        "minimal"
    }
    fn description(&self) -> &'static str {
        "minimal pre-Phase-6 harness"
    }
    fn detect(&self, _home: &Path) -> bool {
        false
    }
    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("MINIMAL_RULES.md")
    }
    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }
    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::Inline
    }
    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join("minimal.mcp.json")
    }
    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Json
    }
    fn mcp_parent_key(&self) -> &'static str {
        "mcpServers"
    }
}

#[test]
fn defaults_are_safe_for_a_new_harness() {
    let h = MinimalHarness;
    let project = Path::new("/proj");

    assert_eq!(h.hooks_strategy(), HooksStrategy::GuardrailsOnly);
    assert_eq!(h.hook_settings_path(project), None);
    assert!(!h.supports_native_agents());
    assert_eq!(h.agent_dir(project), None);
    assert_eq!(h.agent_format(), None);

    // Default guardrails target: an in-file region on the rules-file
    // target, no hooks-driven suppression.
    let target = h.guardrails_target(project);
    assert!(!target.suppress_if_hooks_present);
    assert_eq!(
        target.placement,
        GuardrailsPlacement::InFileRegion {
            file: project.join("MINIMAL_RULES.md")
        }
    );
}

#[test]
fn stub_default_matches_safe_defaults() {
    let h = StubHarness::default();
    let project = Path::new("/proj");
    assert_eq!(h.hooks_strategy(), HooksStrategy::GuardrailsOnly);
    assert_eq!(h.hook_settings_path(project), None);
    assert!(!h.supports_native_agents());
    assert_eq!(h.agent_dir(project), None);
    assert_eq!(h.agent_format(), None);
}

#[test]
fn stub_overrides_expose_new_capabilities() {
    let project = Path::new("/proj");
    let h = StubHarness::default()
        .with_hooks_strategy(HooksStrategy::RealJson)
        .with_hook_settings()
        .with_native_agents(AgentFormat::Toml)
        .with_guardrails_target(GuardrailsTarget {
            placement: GuardrailsPlacement::StandaloneSibling {
                file: project.join(".stub/GUARDRAILS.md"),
            },
            suppress_if_hooks_present: true,
        });

    assert_eq!(h.hooks_strategy(), HooksStrategy::RealJson);
    assert_eq!(
        h.hook_settings_path(project),
        Some(project.join(".stub/settings.local.json"))
    );
    assert!(h.supports_native_agents());
    assert_eq!(h.agent_format(), Some(AgentFormat::Toml));
    assert_eq!(h.agent_dir(project), Some(project.join(".stub/agents")));

    let target = h.guardrails_target(project);
    assert!(target.suppress_if_hooks_present);
    assert_eq!(
        target.placement,
        GuardrailsPlacement::StandaloneSibling {
            file: project.join(".stub/GUARDRAILS.md")
        }
    );
}

#[test]
fn stub_translate_agent_round_trips_body() {
    let h = StubHarness::default().with_native_agents(AgentFormat::MarkdownYaml);
    let canonical = CanonicalAgent {
        catalog: "cat".into(),
        plugin: "myplugin".into(),
        name: "reviewer".into(),
        description: Some("a reviewer".into()),
        body: "You are a careful reviewer.".into(),
        model: None,
        tools: None,
        disallowed_tools: None,
        hooks: None,
        mcp_servers: None,
        permission_mode: None,
    };
    let reg = tome::model_registry::test_registry();
    // `clashes = false` → the displayed name stays the clean `<name>`.
    let translated = h
        .translate_agent(&canonical, false, &reg)
        .expect("stub translation succeeds");
    assert_eq!(translated.displayed_name, "reviewer");
    assert_eq!(translated.filename, "myplugin__reviewer.md");
    assert_eq!(translated.rendered, "You are a careful reviewer.");
    assert_eq!(translated.format, AgentFormat::MarkdownYaml);

    // `clashes = true` → the displayed name is plugin-prefixed; the filename
    // stays `<plugin>__<name>` regardless (FR-041).
    let clashed = h
        .translate_agent(&canonical, true, &reg)
        .expect("stub translation succeeds");
    assert_eq!(clashed.displayed_name, "myplugin-reviewer");
    assert_eq!(clashed.filename, "myplugin__reviewer.md");
}

#[test]
fn with_effective_modules_dispatches_to_stub() {
    // The override slot is process-global and shared across this consolidated
    // binary; serialise like every other harness-override test.
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(
        StubHarness::default().with_hooks_strategy(HooksStrategy::RealJson),
    )]);

    let strategy = with_effective_modules(|modules| {
        let stub = modules
            .iter()
            .find(|m| m.name() == "stub")
            .expect("stub must be in the effective registry");
        stub.hooks_strategy()
    });
    assert_eq!(strategy, HooksStrategy::RealJson);
}

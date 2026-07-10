//! F7 skeleton tests for the harness registry.
//!
//! Verifies the shape of `SUPPORTED_HARNESSES`, the `lookup` function,
//! and each harness's pinned per-harness specifics (path layout, parent
//! key, format, strategy). Real read/write behaviour for `rules_file` +
//! `mcp_config` lands in US3.c / US4.

use std::path::{Path, PathBuf};

use tome::harness::{
    self, BlockBodyStyle, HARNESS_MODULES_OVERRIDE, HarnessModule, MCP_CONFIG_KEY, McpConfigFormat,
    RulesFileStrategy, SUPPORTED_HARNESSES, lookup, with_effective_modules,
};

/// T-B3 (US3 review): the override slot is process-global. Tests that
/// install or read it must serialise via this mutex so cargo's parallel
/// runner can't observe a half-installed state.

#[test]
fn registry_lists_seventeen_harnesses_in_lex_order() {
    let names: Vec<&str> = SUPPORTED_HARNESSES.iter().map(|h| h.name()).collect();
    assert_eq!(
        names,
        vec![
            "antigravity",
            "claude-code",
            "cline",
            "codex",
            "copilot",
            "copilot-cli",
            "crush",
            "cursor",
            "devin",
            "gemini",
            "goose",
            "jetbrains-ai",
            "junie",
            "kiro",
            "opencode",
            "pi",
            "zed",
        ],
    );
}

#[test]
fn each_name_is_unique() {
    let mut names: Vec<&str> = SUPPORTED_HARNESSES.iter().map(|h| h.name()).collect();
    names.sort_unstable();
    let len_before = names.len();
    names.dedup();
    assert_eq!(names.len(), len_before, "harness names must be unique");
}

#[test]
fn lookup_resolves_each_registered_name() {
    for harness in SUPPORTED_HARNESSES {
        let found = lookup(harness.name()).unwrap_or_else(|| {
            panic!(
                "registered harness {:?} must resolve via lookup()",
                harness.name()
            )
        });
        assert_eq!(found.name(), harness.name());
    }
}

#[test]
fn lookup_returns_none_for_unknown_name() {
    assert!(lookup("definitely-not-a-harness").is_none());
}

#[test]
fn mcp_config_key_constant() {
    assert_eq!(MCP_CONFIG_KEY, "tome");
}

#[test]
fn claude_code_specifics() {
    let h = lookup("claude-code").expect("claude-code registered");
    assert_eq!(h.description(), "Anthropic's Claude Code CLI");
    assert_eq!(h.mcp_parent_key(), "mcpServers");
    assert_eq!(h.mcp_config_format(), McpConfigFormat::Json);
    assert_eq!(
        h.rules_file_strategy(),
        RulesFileStrategy::BlockInExistingFile
    );
    assert_eq!(h.block_body_style(), BlockBodyStyle::AtInclude);

    let project = PathBuf::from("/proj");
    let home = PathBuf::from("/home/u");
    assert_eq!(
        h.mcp_config_path(&project, &home),
        PathBuf::from("/proj/.mcp.json"),
    );
}

#[test]
fn codex_specifics() {
    let h = lookup("codex").expect("codex registered");
    // Codex is the one TOML harness — the parent key is snake_case.
    assert_eq!(h.mcp_parent_key(), "mcp_servers");
    assert_eq!(h.mcp_config_format(), McpConfigFormat::Toml);
    assert_eq!(
        h.rules_file_strategy(),
        RulesFileStrategy::BlockInExistingFile
    );
    assert_eq!(h.block_body_style(), BlockBodyStyle::AtInclude);

    let project = PathBuf::from("/proj");
    let home = PathBuf::from("/home/u");
    // Codex's MCP config is global (lives under home, not project).
    assert_eq!(
        h.mcp_config_path(&project, &home),
        PathBuf::from("/home/u/.codex/config.toml"),
    );
    assert_eq!(
        h.rules_file_target(&project),
        PathBuf::from("/proj/AGENTS.md")
    );
}

#[test]
fn cursor_specifics() {
    let h = lookup("cursor").expect("cursor registered");
    assert_eq!(h.mcp_parent_key(), "mcpServers");
    assert_eq!(h.mcp_config_format(), McpConfigFormat::Json);
    // Cursor is the one standalone-file harness.
    assert_eq!(h.rules_file_strategy(), RulesFileStrategy::StandaloneFile);

    let project = PathBuf::from("/proj");
    let home = PathBuf::from("/home/u");
    assert_eq!(
        h.mcp_config_path(&project, &home),
        PathBuf::from("/proj/.cursor/mcp.json"),
    );
    assert_eq!(
        h.rules_file_target(&project),
        PathBuf::from("/proj/.cursor/rules/TOME_SKILLS.md"),
    );
}

#[test]
fn gemini_specifics() {
    let h = lookup("gemini").expect("gemini registered");
    assert_eq!(h.mcp_parent_key(), "mcpServers");
    assert_eq!(h.mcp_config_format(), McpConfigFormat::Json);
    assert_eq!(
        h.rules_file_strategy(),
        RulesFileStrategy::BlockInExistingFile
    );
    assert_eq!(h.block_body_style(), BlockBodyStyle::AtInclude);

    let project = PathBuf::from("/proj");
    let home = PathBuf::from("/home/u");
    // Gemini's MCP config is global.
    assert_eq!(
        h.mcp_config_path(&project, &home),
        PathBuf::from("/home/u/.gemini/settings.json"),
    );
}

#[test]
fn opencode_specifics() {
    let h = lookup("opencode").expect("opencode registered");
    // Phase 11 G1 (canary fix): OpenCode's parent key is `mcp`, not the
    // legacy `mcpServers`. `mcp_config_format()` stays Json (Jsonc routes
    // through the serde_json path).
    assert_eq!(h.mcp_parent_key(), "mcp");
    assert_eq!(h.mcp_config_format(), McpConfigFormat::Json);
    assert_eq!(
        h.rules_file_strategy(),
        RulesFileStrategy::BlockInExistingFile
    );
    // OpenCode does not document @-include support → Inline body.
    assert_eq!(h.block_body_style(), BlockBodyStyle::Inline);

    let project = PathBuf::from("/proj");
    let home = PathBuf::from("/home/u");
    // OpenCode's MCP config is per-project, no dot prefix.
    assert_eq!(
        h.mcp_config_path(&project, &home),
        PathBuf::from("/proj/opencode.json"),
    );
    assert_eq!(
        h.rules_file_target(&project),
        PathBuf::from("/proj/AGENTS.md")
    );
}

#[test]
fn detect_is_filesystem_existence_only() {
    // Build a `home` containing only `.claude/` and verify only
    // claude-code detects. Confirms the FR-167 invariant: no reads of
    // the harness's own config files.
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(tmp.path().join(".claude")).expect("create .claude");

    let claude = lookup("claude-code").unwrap();
    let codex = lookup("codex").unwrap();
    let cursor = lookup("cursor").unwrap();

    assert!(claude.detect(tmp.path()));
    assert!(!codex.detect(tmp.path()));
    assert!(!cursor.detect(tmp.path()));
}

#[test]
fn rules_file_target_picks_existing_candidate() {
    // Claude Code's corrected precedence is CLAUDE.md > .claude/CLAUDE.md
    // (Phase 6 / FR-020 — AGENTS.md removed). When CLAUDE.md exists, it wins.
    let tmp = tempfile::tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("CLAUDE.md"), "developer-authored").expect("write CLAUDE.md");

    let h = lookup("claude-code").unwrap();
    assert_eq!(
        h.rules_file_target(tmp.path()),
        tmp.path().join("CLAUDE.md")
    );
}

#[test]
fn rules_file_target_falls_back_to_first_preference() {
    // No candidate exists yet → fall back to CLAUDE.md (never AGENTS.md, the
    // Phase 6 correction) so the sync algorithm has a default landing spot.
    let tmp = tempfile::tempdir().expect("tempdir");
    let h = lookup("claude-code").unwrap();
    assert_eq!(
        h.rules_file_target(tmp.path()),
        tmp.path().join("CLAUDE.md")
    );
}

#[test]
fn mcp_config_key_module_export() {
    // The standardised entry key is `"tome"` — pinned in the trait
    // module so callers don't hardcode the literal.
    assert_eq!(harness::MCP_CONFIG_KEY, "tome");
}

/// RAII guard demonstrating the `HarnessModulesGuard` pattern from
/// `mod.rs`. F7 doesn't have a production consumer yet; this lives
/// inside the test file so the pattern is exercised at least once.
struct HarnessModulesGuard;

impl HarnessModulesGuard {
    fn install(modules: Vec<Box<dyn HarnessModule>>) -> Self {
        *HARNESS_MODULES_OVERRIDE.write().unwrap() = Some(modules);
        Self
    }
}

impl Drop for HarnessModulesGuard {
    fn drop(&mut self) {
        *HARNESS_MODULES_OVERRIDE.write().unwrap() = None;
    }
}

struct FakeHarness;

impl HarnessModule for FakeHarness {
    fn name(&self) -> &'static str {
        "fake"
    }
    fn description(&self) -> &'static str {
        "synthetic test harness"
    }
    fn detect(&self, _home: &Path) -> bool {
        true
    }
    fn rules_file_target(&self, project_root: &Path) -> PathBuf {
        project_root.join("FAKE.md")
    }
    fn rules_file_strategy(&self) -> RulesFileStrategy {
        RulesFileStrategy::BlockInExistingFile
    }
    fn block_body_style(&self) -> BlockBodyStyle {
        BlockBodyStyle::Inline
    }
    fn mcp_config_path(&self, project_root: &Path, _home: &Path) -> PathBuf {
        project_root.join("fake.json")
    }
    fn mcp_config_format(&self) -> McpConfigFormat {
        McpConfigFormat::Json
    }
    fn mcp_parent_key(&self) -> &'static str {
        "mcpServers"
    }
}

#[test]
fn override_replaces_effective_modules() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(FakeHarness)]);

    let names = with_effective_modules(|mods| {
        mods.iter()
            .map(|m| m.name().to_string())
            .collect::<Vec<_>>()
    });
    assert_eq!(names, vec!["fake".to_string()]);
    // `lookup` resolves against the static registry by design (the
    // function signature returns `'static`); verify the production
    // registry is still reachable so other tests in this binary don't
    // observe leakage.
    assert!(lookup("claude-code").is_some());
}

#[test]
fn effective_modules_falls_back_to_static_when_no_override() {
    // T-B3: acquire the mutex BEFORE inspecting the slot so we don't
    // observe a half-installed state from a parallel test.
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // With the mutex held, no override is in-flight; the slot must be
    // empty for the static-fallback assertion to be meaningful.
    assert!(
        HARNESS_MODULES_OVERRIDE.read().unwrap().is_none(),
        "crate::common::HARNESS_OVERRIDE_MUTEX held but slot non-empty — install/drop discipline broken",
    );
    let count = with_effective_modules(|mods| mods.len());
    assert_eq!(count, SUPPORTED_HARNESSES.len());
}

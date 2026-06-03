//! Cross-harness library-API tests for every registered `HarnessModule`
//! (Phase 4 / US3.c-1 — T293).
//!
//! Mirrors the per-harness §R-8 spec rows in
//! `specs/004-phase-4-refactor-harnesses/research.md` and the contract in
//! `specs/004-phase-4-refactor-harnesses/contracts/harness-modules.md`.
//!
//! Each per-harness sub-module exercises every `HarnessModule` trait method
//! against `TempDir`-rooted home / project directories so the assertions are
//! hermetic — no orchestrator, no DB, no `HARNESS_MODULES_OVERRIDE`. The
//! cross-harness section at the bottom pins the registry shape (length,
//! ordering, lookup round-trip) and the shared `MCP_CONFIG_KEY` constant.
//!
//! `claude-code` is covered both here (for the cross-harness matrix) and in
//! the focused `tests/harness_module_claude_code.rs` file (the US1.c
//! deliverable). The duplication is intentional: this file's job is the
//! matrix; the per-harness file's job is the precedence-ladder spot-check.

use std::fs;

use tempfile::TempDir;
use tome::harness::{
    BlockBodyStyle, HarnessModule, MCP_CONFIG_KEY, McpConfigFormat, RulesFileStrategy,
    SUPPORTED_HARNESSES, claude_code::CLAUDE_CODE, codex::CODEX, cursor::CURSOR, gemini::GEMINI,
    lookup, opencode::OPENCODE,
};

// ===========================================================================
// claude-code
// ===========================================================================

mod claude_code_tests {
    use super::*;

    #[test]
    fn name_is_claude_code() {
        assert_eq!(CLAUDE_CODE.name(), "claude-code");
    }

    #[test]
    fn description_is_non_empty() {
        assert!(!CLAUDE_CODE.description().is_empty());
    }

    #[test]
    fn detect_false_when_dot_claude_missing() {
        let home = TempDir::new().unwrap();
        assert!(!CLAUDE_CODE.detect(home.path()));
    }

    #[test]
    fn detect_true_when_dot_claude_is_a_directory() {
        let home = TempDir::new().unwrap();
        fs::create_dir(home.path().join(".claude")).unwrap();
        assert!(CLAUDE_CODE.detect(home.path()));
    }

    #[test]
    fn detect_false_when_dot_claude_is_a_file() {
        let home = TempDir::new().unwrap();
        fs::write(home.path().join(".claude"), b"not a dir").unwrap();
        assert!(!CLAUDE_CODE.detect(home.path()));
    }

    // Corrected precedence (Phase 6 / FR-020/021/022):
    // CLAUDE.md > .claude/CLAUDE.md, fallback CLAUDE.md. AGENTS.md is NOT a
    // candidate — Claude Code does not natively read it.

    #[test]
    fn rules_file_target_fallback_is_claude_md() {
        let project = TempDir::new().unwrap();
        assert_eq!(
            CLAUDE_CODE.rules_file_target(project.path()),
            project.path().join("CLAUDE.md"),
        );
    }

    #[test]
    fn rules_file_target_ignores_agents_md_top_is_claude_md() {
        let project = TempDir::new().unwrap();
        fs::write(project.path().join("AGENTS.md"), b"").unwrap();
        fs::write(project.path().join("CLAUDE.md"), b"").unwrap();
        fs::create_dir(project.path().join(".claude")).unwrap();
        fs::write(project.path().join(".claude/CLAUDE.md"), b"").unwrap();
        assert_eq!(
            CLAUDE_CODE.rules_file_target(project.path()),
            project.path().join("CLAUDE.md"),
            "CLAUDE.md wins; AGENTS.md is not a candidate",
        );
    }

    #[test]
    fn rules_file_target_falls_back_when_only_agents_md_exists() {
        // AGENTS.md alone must NOT be selected (substance of the correction).
        let project = TempDir::new().unwrap();
        fs::write(project.path().join("AGENTS.md"), b"").unwrap();
        assert_eq!(
            CLAUDE_CODE.rules_file_target(project.path()),
            project.path().join("CLAUDE.md"),
        );
    }

    #[test]
    fn rules_file_target_nested_wins_when_top_missing() {
        let project = TempDir::new().unwrap();
        fs::create_dir(project.path().join(".claude")).unwrap();
        fs::write(project.path().join(".claude/CLAUDE.md"), b"").unwrap();
        assert_eq!(
            CLAUDE_CODE.rules_file_target(project.path()),
            project.path().join(".claude/CLAUDE.md"),
        );
    }

    #[test]
    fn rules_file_strategy_is_block_in_existing_file() {
        assert_eq!(
            CLAUDE_CODE.rules_file_strategy(),
            RulesFileStrategy::BlockInExistingFile,
        );
    }

    #[test]
    fn block_body_style_is_at_include() {
        assert_eq!(CLAUDE_CODE.block_body_style(), BlockBodyStyle::AtInclude);
    }

    #[test]
    fn mcp_config_path_is_project_dot_claude_settings_json() {
        let project = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let path = CLAUDE_CODE.mcp_config_path(project.path(), home.path());
        assert_eq!(path, project.path().join(".claude/settings.json"));
        assert!(
            !path.starts_with(home.path()),
            "claude-code MCP config is per-project — must NOT live under home"
        );
    }

    #[test]
    fn mcp_config_format_is_json() {
        assert_eq!(CLAUDE_CODE.mcp_config_format(), McpConfigFormat::Json);
    }

    #[test]
    fn mcp_parent_key_is_camel_mcp_servers() {
        assert_eq!(CLAUDE_CODE.mcp_parent_key(), "mcpServers");
    }
}

// ===========================================================================
// codex
// ===========================================================================

mod codex_tests {
    use super::*;

    #[test]
    fn name_is_codex() {
        assert_eq!(CODEX.name(), "codex");
    }

    #[test]
    fn description_is_non_empty() {
        assert!(!CODEX.description().is_empty());
    }

    #[test]
    fn detect_false_when_dot_codex_missing() {
        let home = TempDir::new().unwrap();
        assert!(!CODEX.detect(home.path()));
    }

    #[test]
    fn detect_true_when_dot_codex_is_a_directory() {
        let home = TempDir::new().unwrap();
        fs::create_dir(home.path().join(".codex")).unwrap();
        assert!(CODEX.detect(home.path()));
    }

    #[test]
    fn detect_false_when_dot_codex_is_a_file() {
        let home = TempDir::new().unwrap();
        fs::write(home.path().join(".codex"), b"not a dir").unwrap();
        assert!(!CODEX.detect(home.path()));
    }

    #[test]
    fn rules_file_target_is_project_agents_md() {
        let project = TempDir::new().unwrap();
        // Pre-existing files MUST NOT change the target — Codex CLI only
        // reads `AGENTS.md`, so no precedence ladder applies.
        fs::write(project.path().join("CLAUDE.md"), b"").unwrap();
        assert_eq!(
            CODEX.rules_file_target(project.path()),
            project.path().join("AGENTS.md"),
        );
    }

    #[test]
    fn rules_file_strategy_is_block_in_existing_file() {
        assert_eq!(
            CODEX.rules_file_strategy(),
            RulesFileStrategy::BlockInExistingFile,
        );
    }

    #[test]
    fn block_body_style_is_at_include() {
        assert_eq!(CODEX.block_body_style(), BlockBodyStyle::AtInclude);
    }

    #[test]
    fn mcp_config_path_is_global_under_home() {
        let project = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let path = CODEX.mcp_config_path(project.path(), home.path());
        assert_eq!(path, home.path().join(".codex/config.toml"));
        assert!(
            !path.starts_with(project.path()),
            "codex MCP config is global — must NOT live under project"
        );
    }

    #[test]
    fn mcp_config_format_is_toml() {
        assert_eq!(CODEX.mcp_config_format(), McpConfigFormat::Toml);
    }

    #[test]
    fn mcp_parent_key_is_snake_mcp_servers() {
        // Documented TOML convention is snake_case here, distinct from
        // the JSON harnesses' `mcpServers`.
        assert_eq!(CODEX.mcp_parent_key(), "mcp_servers");
    }
}

// ===========================================================================
// cursor
// ===========================================================================

mod cursor_tests {
    use super::*;

    #[test]
    fn name_is_cursor() {
        assert_eq!(CURSOR.name(), "cursor");
    }

    #[test]
    fn description_is_non_empty() {
        assert!(!CURSOR.description().is_empty());
    }

    #[test]
    fn detect_false_when_dot_cursor_missing() {
        let home = TempDir::new().unwrap();
        assert!(!CURSOR.detect(home.path()));
    }

    #[test]
    fn detect_true_when_dot_cursor_is_a_directory() {
        let home = TempDir::new().unwrap();
        fs::create_dir(home.path().join(".cursor")).unwrap();
        assert!(CURSOR.detect(home.path()));
    }

    #[test]
    fn detect_false_when_dot_cursor_is_a_file() {
        let home = TempDir::new().unwrap();
        fs::write(home.path().join(".cursor"), b"not a dir").unwrap();
        assert!(!CURSOR.detect(home.path()));
    }

    #[test]
    fn rules_file_target_is_tome_owned_standalone_file() {
        let project = TempDir::new().unwrap();
        assert_eq!(
            CURSOR.rules_file_target(project.path()),
            project.path().join(".cursor/rules/TOME_SKILLS.md"),
        );
    }

    #[test]
    fn rules_file_strategy_is_standalone_file() {
        assert_eq!(
            CURSOR.rules_file_strategy(),
            RulesFileStrategy::StandaloneFile,
        );
    }

    #[test]
    fn block_body_style_is_placeholder_inline() {
        // Never consulted for StandaloneFile — the contract documents
        // `Inline` as the harmless placeholder.
        assert_eq!(CURSOR.block_body_style(), BlockBodyStyle::Inline);
    }

    #[test]
    fn mcp_config_path_is_project_dot_cursor_mcp_json() {
        let project = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let path = CURSOR.mcp_config_path(project.path(), home.path());
        assert_eq!(path, project.path().join(".cursor/mcp.json"));
        assert!(
            !path.starts_with(home.path()),
            "cursor MCP config is per-project — must NOT live under home"
        );
    }

    #[test]
    fn mcp_config_format_is_json() {
        assert_eq!(CURSOR.mcp_config_format(), McpConfigFormat::Json);
    }

    #[test]
    fn mcp_parent_key_is_camel_mcp_servers() {
        assert_eq!(CURSOR.mcp_parent_key(), "mcpServers");
    }
}

// ===========================================================================
// gemini
// ===========================================================================

mod gemini_tests {
    use super::*;

    #[test]
    fn name_is_gemini() {
        assert_eq!(GEMINI.name(), "gemini");
    }

    #[test]
    fn description_is_non_empty() {
        assert!(!GEMINI.description().is_empty());
    }

    #[test]
    fn detect_false_when_dot_gemini_missing() {
        let home = TempDir::new().unwrap();
        assert!(!GEMINI.detect(home.path()));
    }

    #[test]
    fn detect_true_when_dot_gemini_is_a_directory() {
        let home = TempDir::new().unwrap();
        fs::create_dir(home.path().join(".gemini")).unwrap();
        assert!(GEMINI.detect(home.path()));
    }

    #[test]
    fn detect_false_when_dot_gemini_is_a_file() {
        let home = TempDir::new().unwrap();
        fs::write(home.path().join(".gemini"), b"not a dir").unwrap();
        assert!(!GEMINI.detect(home.path()));
    }

    // Precedence: AGENTS.md > GEMINI.md > .gemini/GEMINI.md, fallback AGENTS.md.

    #[test]
    fn rules_file_target_fallback_is_agents_md() {
        let project = TempDir::new().unwrap();
        assert_eq!(
            GEMINI.rules_file_target(project.path()),
            project.path().join("AGENTS.md"),
        );
    }

    #[test]
    fn rules_file_target_top_wins_when_all_three_exist() {
        let project = TempDir::new().unwrap();
        fs::write(project.path().join("AGENTS.md"), b"").unwrap();
        fs::write(project.path().join("GEMINI.md"), b"").unwrap();
        fs::create_dir(project.path().join(".gemini")).unwrap();
        fs::write(project.path().join(".gemini/GEMINI.md"), b"").unwrap();
        assert_eq!(
            GEMINI.rules_file_target(project.path()),
            project.path().join("AGENTS.md"),
        );
    }

    #[test]
    fn rules_file_target_second_wins_when_top_missing() {
        let project = TempDir::new().unwrap();
        fs::write(project.path().join("GEMINI.md"), b"").unwrap();
        fs::create_dir(project.path().join(".gemini")).unwrap();
        fs::write(project.path().join(".gemini/GEMINI.md"), b"").unwrap();
        assert_eq!(
            GEMINI.rules_file_target(project.path()),
            project.path().join("GEMINI.md"),
        );
    }

    #[test]
    fn rules_file_target_third_wins_when_top_and_second_missing() {
        let project = TempDir::new().unwrap();
        fs::create_dir(project.path().join(".gemini")).unwrap();
        fs::write(project.path().join(".gemini/GEMINI.md"), b"").unwrap();
        assert_eq!(
            GEMINI.rules_file_target(project.path()),
            project.path().join(".gemini/GEMINI.md"),
        );
    }

    #[test]
    fn rules_file_strategy_is_block_in_existing_file() {
        assert_eq!(
            GEMINI.rules_file_strategy(),
            RulesFileStrategy::BlockInExistingFile,
        );
    }

    #[test]
    fn block_body_style_is_at_include() {
        assert_eq!(GEMINI.block_body_style(), BlockBodyStyle::AtInclude);
    }

    #[test]
    fn mcp_config_path_is_global_under_home() {
        let project = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let path = GEMINI.mcp_config_path(project.path(), home.path());
        assert_eq!(path, home.path().join(".gemini/settings.json"));
        assert!(
            !path.starts_with(project.path()),
            "gemini MCP config is global — must NOT live under project"
        );
    }

    #[test]
    fn mcp_config_format_is_json() {
        assert_eq!(GEMINI.mcp_config_format(), McpConfigFormat::Json);
    }

    #[test]
    fn mcp_parent_key_is_camel_mcp_servers() {
        assert_eq!(GEMINI.mcp_parent_key(), "mcpServers");
    }
}

// ===========================================================================
// opencode
// ===========================================================================

mod opencode_tests {
    use super::*;

    #[test]
    fn name_is_opencode() {
        assert_eq!(OPENCODE.name(), "opencode");
    }

    #[test]
    fn description_is_non_empty() {
        assert!(!OPENCODE.description().is_empty());
    }

    #[test]
    fn detect_false_when_dot_opencode_missing() {
        let home = TempDir::new().unwrap();
        assert!(!OPENCODE.detect(home.path()));
    }

    #[test]
    fn detect_true_when_dot_opencode_is_a_directory() {
        let home = TempDir::new().unwrap();
        fs::create_dir(home.path().join(".opencode")).unwrap();
        assert!(OPENCODE.detect(home.path()));
    }

    #[test]
    fn detect_false_when_dot_opencode_is_a_file() {
        let home = TempDir::new().unwrap();
        fs::write(home.path().join(".opencode"), b"not a dir").unwrap();
        assert!(!OPENCODE.detect(home.path()));
    }

    #[test]
    fn rules_file_target_is_project_agents_md() {
        let project = TempDir::new().unwrap();
        // No precedence ladder — `AGENTS.md` regardless of what else
        // exists in the project root.
        fs::write(project.path().join("CLAUDE.md"), b"").unwrap();
        assert_eq!(
            OPENCODE.rules_file_target(project.path()),
            project.path().join("AGENTS.md"),
        );
    }

    #[test]
    fn rules_file_strategy_is_block_in_existing_file() {
        assert_eq!(
            OPENCODE.rules_file_strategy(),
            RulesFileStrategy::BlockInExistingFile,
        );
    }

    #[test]
    fn block_body_style_is_inline() {
        // OpenCode does not document @-include support; the block holds
        // the full rules verbatim and is rewritten on every summary regen.
        assert_eq!(OPENCODE.block_body_style(), BlockBodyStyle::Inline);
    }

    #[test]
    fn mcp_config_path_is_project_opencode_json_no_dot_prefix() {
        let project = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();
        let path = OPENCODE.mcp_config_path(project.path(), home.path());
        assert_eq!(path, project.path().join("opencode.json"));
        assert!(
            !path.starts_with(home.path()),
            "opencode MCP config is per-project — must NOT live under home"
        );
    }

    #[test]
    fn mcp_config_format_is_json() {
        assert_eq!(OPENCODE.mcp_config_format(), McpConfigFormat::Json);
    }

    #[test]
    fn mcp_parent_key_is_camel_mcp_servers() {
        assert_eq!(OPENCODE.mcp_parent_key(), "mcpServers");
    }
}

// ===========================================================================
// Cross-harness invariants
// ===========================================================================

#[test]
fn supported_harnesses_has_exactly_five_entries() {
    assert_eq!(
        SUPPORTED_HARNESSES.len(),
        5,
        "Phase 4 ships exactly 5 harness modules",
    );
}

#[test]
fn supported_harnesses_in_lexicographic_order() {
    let names: Vec<&str> = SUPPORTED_HARNESSES.iter().map(|m| m.name()).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(
        names, sorted,
        "SUPPORTED_HARNESSES must be lexicographically sorted by name()",
    );
}

#[test]
fn every_supported_harness_resolves_via_lookup() {
    for m in SUPPORTED_HARNESSES {
        let found = lookup(m.name())
            .unwrap_or_else(|| panic!("registered harness {} must resolve", m.name()));
        assert_eq!(found.name(), m.name());
    }
}

#[test]
fn lookup_returns_none_for_unknown_name() {
    assert!(lookup("definitely-not-a-real-harness-name").is_none());
}

#[test]
fn supported_harness_names_match_expected_set() {
    let names: Vec<&str> = SUPPORTED_HARNESSES.iter().map(|m| m.name()).collect();
    assert_eq!(
        names,
        vec!["claude-code", "codex", "cursor", "gemini", "opencode"],
    );
}

#[test]
fn mcp_config_key_is_tome() {
    // Every harness writes its tome-managed MCP entry under this key,
    // nested inside the harness-specific parent key.
    assert_eq!(MCP_CONFIG_KEY, "tome");
}

#[test]
fn every_harness_parent_key_is_one_of_the_two_documented_values() {
    // JSON harnesses → `"mcpServers"`. The single TOML harness (Codex)
    // → `"mcp_servers"`. Anything else would be a contract drift.
    for m in SUPPORTED_HARNESSES {
        let key = m.mcp_parent_key();
        assert!(
            key == "mcpServers" || key == "mcp_servers",
            "harness {} has unexpected mcp_parent_key {key:?}",
            m.name(),
        );
    }
}

#[test]
fn json_format_implies_camel_parent_key_toml_implies_snake() {
    // Pin the cross-axis invariant: format ↔ parent key naming.
    for m in SUPPORTED_HARNESSES {
        match m.mcp_config_format() {
            McpConfigFormat::Json => assert_eq!(
                m.mcp_parent_key(),
                "mcpServers",
                "JSON harness {} must use mcpServers",
                m.name(),
            ),
            McpConfigFormat::Toml => assert_eq!(
                m.mcp_parent_key(),
                "mcp_servers",
                "TOML harness {} must use mcp_servers",
                m.name(),
            ),
        }
    }
}

#[test]
fn standalone_file_strategy_implies_unused_block_body_style() {
    // Block body style is only consulted for BlockInExistingFile. For
    // StandaloneFile harnesses, the value is documented as a placeholder
    // — verify the contract surface (a defined enum value) without
    // pinning the specific variant beyond it being valid.
    for m in SUPPORTED_HARNESSES {
        let _style = m.block_body_style();
        if m.rules_file_strategy() == RulesFileStrategy::StandaloneFile {
            // Sanity: we currently expect Inline as the placeholder.
            assert_eq!(
                m.block_body_style(),
                BlockBodyStyle::Inline,
                "StandaloneFile harness {} placeholder should be Inline",
                m.name(),
            );
        }
    }
}

// ===========================================================================
// US5.a / T376b — explicit 5 × 9 method matrix
// ===========================================================================

/// Expected values for the nine `HarnessModule` trait methods for one
/// harness. The matrix below is driven from research §R-8 and lives
/// here as an explicit assertion that adding a new harness to
/// `SUPPORTED_HARNESSES` requires updating these documented values
/// (the per-harness sub-modules above cover the precedence-ladder
/// details for each one).
struct ExpectedValues {
    name: &'static str,
    description_starts_with: &'static str,
    detect_dir_name: &'static str,
    rules_strategy: RulesFileStrategy,
    block_body_style: BlockBodyStyle,
    mcp_format: McpConfigFormat,
    mcp_parent_key: &'static str,
}

#[test]
fn explicit_5x9_method_matrix_covers_every_supported_harness() {
    let matrix: &[(&dyn HarnessModule, ExpectedValues)] = &[
        (
            &CLAUDE_CODE,
            ExpectedValues {
                name: "claude-code",
                description_starts_with: "",
                detect_dir_name: ".claude",
                rules_strategy: RulesFileStrategy::BlockInExistingFile,
                block_body_style: BlockBodyStyle::AtInclude,
                mcp_format: McpConfigFormat::Json,
                mcp_parent_key: "mcpServers",
            },
        ),
        (
            &CODEX,
            ExpectedValues {
                name: "codex",
                description_starts_with: "",
                detect_dir_name: ".codex",
                rules_strategy: RulesFileStrategy::BlockInExistingFile,
                block_body_style: BlockBodyStyle::AtInclude,
                mcp_format: McpConfigFormat::Toml,
                mcp_parent_key: "mcp_servers",
            },
        ),
        (
            &CURSOR,
            ExpectedValues {
                name: "cursor",
                description_starts_with: "",
                detect_dir_name: ".cursor",
                rules_strategy: RulesFileStrategy::StandaloneFile,
                block_body_style: BlockBodyStyle::Inline,
                mcp_format: McpConfigFormat::Json,
                mcp_parent_key: "mcpServers",
            },
        ),
        (
            &GEMINI,
            ExpectedValues {
                name: "gemini",
                description_starts_with: "",
                detect_dir_name: ".gemini",
                rules_strategy: RulesFileStrategy::BlockInExistingFile,
                block_body_style: BlockBodyStyle::AtInclude,
                mcp_format: McpConfigFormat::Json,
                mcp_parent_key: "mcpServers",
            },
        ),
        (
            &OPENCODE,
            ExpectedValues {
                name: "opencode",
                description_starts_with: "",
                detect_dir_name: ".opencode",
                rules_strategy: RulesFileStrategy::BlockInExistingFile,
                block_body_style: BlockBodyStyle::Inline,
                mcp_format: McpConfigFormat::Json,
                mcp_parent_key: "mcpServers",
            },
        ),
    ];

    assert_eq!(
        matrix.len(),
        SUPPORTED_HARNESSES.len(),
        "T376b matrix must mention every registered harness exactly once",
    );

    for (module, expected) in matrix {
        // 1. name
        assert_eq!(module.name(), expected.name, "name() for {}", expected.name);
        // 2. description — non-empty, the prefix gate is informational.
        assert!(
            !module.description().is_empty(),
            "description() for {} must be non-empty",
            expected.name,
        );
        assert!(
            module
                .description()
                .starts_with(expected.description_starts_with),
            "description() for {} must start with {:?}",
            expected.name,
            expected.description_starts_with,
        );
        // 3. detect — `<home>/<detect_dir_name>/` must register true,
        //    absent must register false.
        let home_present = TempDir::new().unwrap();
        fs::create_dir(home_present.path().join(expected.detect_dir_name)).unwrap();
        assert!(
            module.detect(home_present.path()),
            "detect() must be true for {} when {} exists",
            expected.name,
            expected.detect_dir_name,
        );
        let home_absent = TempDir::new().unwrap();
        assert!(
            !module.detect(home_absent.path()),
            "detect() must be false for {} when {} is absent",
            expected.name,
            expected.detect_dir_name,
        );
        // 4. rules_file_target — non-empty path inside project root.
        let project = TempDir::new().unwrap();
        let target = module.rules_file_target(project.path());
        assert!(
            target.starts_with(project.path())
                || target
                    .to_string_lossy()
                    .contains(expected.detect_dir_name.trim_start_matches('.')),
            "rules_file_target for {} should be project-relative or harness-rooted",
            expected.name,
        );
        // 5. rules_file_strategy
        assert_eq!(
            module.rules_file_strategy(),
            expected.rules_strategy,
            "rules_file_strategy for {}",
            expected.name,
        );
        // 6. block_body_style
        assert_eq!(
            module.block_body_style(),
            expected.block_body_style,
            "block_body_style for {}",
            expected.name,
        );
        // 7. mcp_config_path — a non-empty path under either project or home.
        let home = TempDir::new().unwrap();
        let mcp_path = module.mcp_config_path(project.path(), home.path());
        assert!(
            !mcp_path.as_os_str().is_empty(),
            "mcp_config_path for {} must be non-empty",
            expected.name,
        );
        // 8. mcp_config_format
        assert_eq!(
            module.mcp_config_format(),
            expected.mcp_format,
            "mcp_config_format for {}",
            expected.name,
        );
        // 9. mcp_parent_key
        assert_eq!(
            module.mcp_parent_key(),
            expected.mcp_parent_key,
            "mcp_parent_key for {}",
            expected.name,
        );
    }
    // MCP_CONFIG_KEY is the cross-harness invariant (always "tome").
    assert_eq!(MCP_CONFIG_KEY, "tome");
    // Spot-check lookup() reaches each harness by name.
    for (module, _) in matrix {
        assert!(lookup(module.name()).is_some(), "lookup({})", module.name());
    }
}

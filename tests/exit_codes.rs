//! Exhaustive match over every `TomeError` variant. Adding a variant without
//! updating this test is a compile error, which is the entire point of the
//! closed-set guarantee (FR-022 carried forward + FR-048 additions).
//!
//! Phase 2 contracts: `specs/002-phase-2-plugins-index/contracts/exit-codes.md`.

use std::io;
use std::path::PathBuf;

use tome::error::{
    CompositionErrorKind, ManifestInvalid, PluginState, ShortOrLong, SummariserFailureKind,
    TomeError,
};

fn dummy_io_error() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "x")
}

fn build_each_variant() -> Vec<(TomeError, i32, &'static str)> {
    // The exhaustive arm in `exhaustive_match_compile_check` below is what
    // forces every variant to appear here: if you add a variant to
    // `TomeError`, this test stops compiling until you cover it.
    vec![
        // 1 — internal
        (TomeError::Internal(anyhow::anyhow!("boom")), 1, "internal"),
        // 2–8 — Phase 1
        (TomeError::Usage("bad flag".into()), 2, "usage"),
        (
            TomeError::CatalogNotFound("foo".into()),
            3,
            "catalog_not_found",
        ),
        (
            TomeError::CatalogAlreadyExists("foo".into()),
            4,
            "catalog_already_exists",
        ),
        (
            TomeError::ManifestInvalid(ManifestInvalid::MissingField {
                file: PathBuf::from("tome-catalog.toml"),
                key: "name".into(),
            }),
            5,
            "manifest_invalid",
        ),
        (
            TomeError::GitFailed {
                catalog: "foo".into(),
                detail: "fatal: …".into(),
            },
            6,
            "git_failed",
        ),
        (TomeError::Io(dummy_io_error()), 7, "io"),
        (TomeError::Interrupted, 8, "interrupted"),
        // 9 — Phase 5 / US1.d (R-M1): plugin data dir write failure
        // (split from `WorkspaceDataDirWriteFailed` so the variant name
        // + exit code carry the discriminator instead of the inner path).
        (
            TomeError::PluginDataDirWriteFailed {
                path: PathBuf::from("/home/u/.tome/plugin-data/midnight-expert/compact-dev"),
                source: dummy_io_error(),
            },
            9,
            "plugin_data_dir_write_failed",
        ),
        // 13–20 — Phase 4 new variants (pre-allocated by F3 — no production
        // wiring yet; see specs/004-phase-4-refactor-harnesses/tasks.md T032).
        // Note: `SummariserFailure` is mapped to exit 24 (not 20 as the
        // contract states) to avoid colliding with `PluginNotFound` (20).
        (
            TomeError::WorkspaceNotFound {
                name: "shared".into(),
            },
            13,
            "workspace_not_found",
        ),
        (
            TomeError::WorkspaceAlreadyExists {
                name: "shared".into(),
            },
            14,
            "workspace_already_exists",
        ),
        (
            TomeError::WorkspaceNameInvalid {
                name: "-bad".into(),
                reason: "leading hyphen".into(),
            },
            15,
            "workspace_name_invalid",
        ),
        (
            TomeError::WorkspaceHasBoundProjects {
                name: "shared".into(),
                count: 2,
                projects: vec!["/tmp/a".into(), "/tmp/b".into()],
            },
            16,
            "workspace_has_bound_projects",
        ),
        (
            TomeError::CompositionError {
                kind: CompositionErrorKind::Cycle {
                    path: vec!["project".into(), "shared".into(), "shared".into()],
                },
            },
            17,
            "composition_error",
        ),
        (
            TomeError::HarnessNotSupported {
                name: "made-up-harness".into(),
            },
            18,
            "harness_not_supported",
        ),
        (
            TomeError::HarnessClash {
                path: PathBuf::from("/tmp/.cursor/mcp.json"),
                command: "node".into(),
                first_arg: "/opt/custom-tome.js".into(),
            },
            19,
            "harness_clash",
        ),
        (
            TomeError::SummariserFailure {
                kind: SummariserFailureKind::OutputEmpty {
                    which: ShortOrLong::Long,
                },
            },
            24,
            "summariser_failure",
        ),
        // 20–23 — plugin lifecycle
        (
            TomeError::PluginNotFound("foo/bar".into()),
            20,
            "plugin_not_found",
        ),
        (
            TomeError::PluginAlreadyInState {
                plugin: "foo/bar".into(),
                state: PluginState::Enabled,
            },
            21,
            "plugin_already_in_state",
        ),
        (
            TomeError::PluginManifestParseError {
                file: PathBuf::from("plugin.json"),
                message: "expected `name`".into(),
            },
            22,
            "plugin_manifest_parse_error",
        ),
        (
            TomeError::SkillFrontmatterParseError {
                file: PathBuf::from("SKILL.md"),
                message: "unterminated YAML".into(),
            },
            23,
            "skill_frontmatter_parse_error",
        ),
        // 30–33 — models on disk
        (
            TomeError::ModelMissing {
                model: "bge-small-en-v1.5".into(),
            },
            30,
            "model_missing",
        ),
        (
            TomeError::ModelCorrupt {
                model: "bge-reranker-base".into(),
                detail: "truncated".into(),
            },
            31,
            "model_corrupt",
        ),
        (
            TomeError::ModelChecksumMismatch {
                model: "bge-small-en-v1.5".into(),
                expected: "aa".into(),
                got: "bb".into(),
            },
            32,
            "model_checksum_mismatch",
        ),
        (
            TomeError::ModelRegistrationParseError {
                file: PathBuf::from("manifest.json"),
                message: "unknown field `foo`".into(),
            },
            33,
            "model_registration_parse_error",
        ),
        // 34–37 — inference + vector engine init
        (
            TomeError::InferenceRuntimeInitFailure("missing libonnxruntime".into()),
            34,
            "inference_runtime_init_failure",
        ),
        (
            TomeError::VectorExtensionInitFailure("symbol not found".into()),
            35,
            "vector_extension_init_failure",
        ),
        (
            TomeError::EmbeddingGenerationFailure {
                input_desc: "skill `foo`".into(),
                detail: "OOM".into(),
            },
            36,
            "embedding_generation_failure",
        ),
        (
            TomeError::RerankingFailure("ORT runtime error".into()),
            37,
            "reranking_failure",
        ),
        // 40–42 — query + drift
        (
            TomeError::QueryNoResultsStrict { threshold: 0.5 },
            40,
            "query_no_results_strict",
        ),
        (
            TomeError::EmbedderNameDrift {
                stored: "bge-small-en-v1.5".into(),
                configured: "bge-base-en".into(),
            },
            41,
            "embedder_name_drift",
        ),
        (
            TomeError::EmbedderVersionDrift {
                stored: "1.5".into(),
                configured: "1.6".into(),
            },
            42,
            "embedder_version_drift",
        ),
        // 50–54 — index + catalog interaction
        (TomeError::IndexBusy, 50, "index_busy"),
        (
            TomeError::IndexIntegrityCheckFailure("page 17 malformed".into()),
            51,
            "index_integrity_check_failure",
        ),
        (
            TomeError::SchemaTooNew {
                on_disk: 2,
                compiled: 1,
            },
            52,
            "schema_too_new",
        ),
        (
            TomeError::CatalogHasEnabledPlugins {
                catalog: "midnight-experts".into(),
                plugins: vec!["midnight-experts/compact-expert".into()],
            },
            53,
            "catalog_has_enabled_plugins",
        ),
        (TomeError::NotATerminal, 54, "not_a_terminal"),
        // 60–61 — MCP server (Phase 3)
        (
            TomeError::McpStartupFailed {
                reason: "rmcp handshake rejected".into(),
            },
            60,
            "mcp_startup",
        ),
        (
            TomeError::McpProtocolIo {
                source: dummy_io_error(),
            },
            61,
            "mcp_io",
        ),
        // 70–75 — workspace + schema (Phase 3)
        (
            TomeError::WorkspaceMalformed {
                path: PathBuf::from("/tmp/ws"),
                reason: "invalid TOML in .tome/config.toml at line 4".into(),
            },
            70,
            "workspace_malformed",
        ),
        (
            TomeError::SchemaVersionTooNew {
                on_disk: 99,
                expected: 1,
            },
            73,
            "schema_too_new",
        ),
        (
            TomeError::SchemaMigrationFailed {
                from: 0,
                to: 1,
                source: anyhow::anyhow!("synthetic migration failure"),
            },
            74,
            "schema_migration",
        ),
        (
            TomeError::DoctorFixNotSafe {
                subsystem: "catalog_cache".into(),
            },
            75,
            "doctor_fix_unsafe",
        ),
        // 25–29 — Phase 5 commands-as-prompts + substitution layer
        // (pre-allocated by F1 — no production wiring yet; see
        // specs/005-phase-5-commands-prompts/tasks.md T012-T016).
        // Note: the contract `contracts/exit-codes-p5.md` originally proposed
        // codes 21/22/23 for `EntryNotFound`/`SubstitutionFailed`/
        // `InvalidArgumentFrontmatter` but those collide with Phase 2's plugin
        // lifecycle cluster (21/22/23). F1 reassigned to 27/28/29 — same
        // precedent as Phase 4 F3 which moved `SummariserFailure` from
        // contract-proposed 20 to actual 24 to dodge `PluginNotFound`.
        (
            TomeError::WorkspaceDataDirWriteFailed {
                path: PathBuf::from("/home/u/.tome/plugin-data/midnight-expert/compact-dev"),
                source: dummy_io_error(),
            },
            25,
            "workspace_data_dir_write_failed",
        ),
        (
            TomeError::PromptArgumentMismatch {
                expected: 3,
                supplied: 4,
            },
            26,
            "prompt_argument_mismatch",
        ),
        (
            TomeError::EntryNotFound {
                catalog: "midnight-expert".into(),
                plugin: "compact-dev".into(),
                name: "circuits".into(),
                kind: "skill".into(),
            },
            27,
            "entry_not_found",
        ),
        (
            TomeError::SubstitutionFailed {
                reason: "named arg `component` referenced but not declared".into(),
            },
            28,
            "substitution_failed",
        ),
        (
            TomeError::InvalidArgumentFrontmatter {
                file: PathBuf::from("SKILL.md"),
                reason: "argument name `1foo` does not match [a-z_][a-z0-9_]*".into(),
            },
            29,
            "invalid_argument_frontmatter",
        ),
        // 43–46 — Phase 6 hooks + agents (pre-allocated by F1 — no
        // production wiring yet; see contracts/exit-codes-p6.md). The
        // PRD-proposed 30–33 collided with the model-on-disk cluster, so
        // F1 claims the first free contiguous run.
        (
            TomeError::HookSpecParseError {
                path: PathBuf::from("plugins/foo/hooks/hooks.json"),
            },
            43,
            "hook_spec_parse_error",
        ),
        (
            TomeError::HookSettingsWriteFailed {
                path: PathBuf::from("/proj/.claude/settings.local.json"),
                source: dummy_io_error(),
            },
            44,
            "hook_settings_write_failed",
        ),
        (
            TomeError::AgentTranslationFailed {
                agent: "midnight-expert/compact-dev/reviewer".into(),
            },
            45,
            "agent_translation_failed",
        ),
        (
            TomeError::GuardrailsWriteFailed {
                path: PathBuf::from("/proj/.cursor/rules/TOME_GUARDRAILS.md"),
            },
            46,
            "guardrails_write_failed",
        ),
        // 80–86 — Phase 8 authoring & conversion. A fresh contiguous decade
        // (earlier blocks ran out of room); the lint verdict codes 85/86
        // follow the QueryNoResultsStrict(40) "successful run, non-zero
        // verdict" precedent. See contracts/exit-codes.md.
        (
            TomeError::PluginNotConverted {
                path: PathBuf::from("catalogs/acme/plugins/foo"),
            },
            80,
            "plugin_not_converted",
        ),
        (
            TomeError::OutputExists {
                path: PathBuf::from("./foo/SKILL.md"),
            },
            81,
            "output_exists",
        ),
        (
            TomeError::TemplateInvalid {
                template: "acme/tome-skill-template".into(),
                reason: "template file `SKILL.md` not found in the resolved template".into(),
            },
            82,
            "template_invalid",
        ),
        (
            TomeError::SourceFormatUnrecognized {
                path: PathBuf::from("./some-dir"),
            },
            83,
            "source_format_unrecognized",
        ),
        (
            TomeError::ConversionUnsupportedStrict {
                feature: "claude-code monitors/ directory".into(),
            },
            84,
            "conversion_unsupported_strict",
        ),
        (
            TomeError::ValidationFoundErrors { errors: 2 },
            85,
            "validation_found_errors",
        ),
        (
            TomeError::ValidationStrictWarnings { warnings: 3 },
            86,
            "validation_strict_warnings",
        ),
    ]
}

#[test]
fn every_variant_has_documented_exit_code_and_category() {
    for (err, expected_code, expected_category) in build_each_variant() {
        assert_eq!(
            err.exit_code(),
            expected_code,
            "variant {:?} produced unexpected exit code",
            err
        );
        assert_eq!(
            err.category(),
            expected_category,
            "variant {:?} produced unexpected category",
            err
        );
    }
}

#[test]
fn exit_codes_are_pairwise_unique() {
    // Defence against accidental re-use: every shipped error category gets a
    // distinct exit code (constitution principle II, NON-NEGOTIABLE).
    let mut seen = std::collections::HashMap::<i32, &'static str>::new();
    for (err, code, category) in build_each_variant() {
        if let Some(prev) = seen.insert(code, category) {
            panic!(
                "exit code {} is shared by `{}` and `{}` — codes must be unique",
                code, prev, category
            );
        }
        let _ = err; // moved
    }
}

#[test]
fn exhaustive_match_compile_check() {
    // If a new variant is added to `TomeError`, this match stops being
    // exhaustive and the file fails to compile. That compile failure is the
    // closed-set guarantee in action.
    fn _code_for(err: &TomeError) -> i32 {
        match err {
            TomeError::Internal(_) => 1,
            TomeError::Usage(_) => 2,
            TomeError::CatalogNotFound(_) => 3,
            TomeError::CatalogAlreadyExists(_) => 4,
            TomeError::ManifestInvalid(_) => 5,
            TomeError::GitFailed { .. } => 6,
            TomeError::Io(_) => 7,
            TomeError::Interrupted => 8,
            TomeError::PluginDataDirWriteFailed { .. } => 9,
            TomeError::WorkspaceNotFound { .. } => 13,
            TomeError::WorkspaceAlreadyExists { .. } => 14,
            TomeError::WorkspaceNameInvalid { .. } => 15,
            TomeError::WorkspaceHasBoundProjects { .. } => 16,
            TomeError::CompositionError { .. } => 17,
            TomeError::HarnessNotSupported { .. } => 18,
            TomeError::HarnessClash { .. } => 19,
            TomeError::SummariserFailure { .. } => 24,
            TomeError::WorkspaceDataDirWriteFailed { .. } => 25,
            TomeError::PromptArgumentMismatch { .. } => 26,
            TomeError::EntryNotFound { .. } => 27,
            TomeError::SubstitutionFailed { .. } => 28,
            TomeError::InvalidArgumentFrontmatter { .. } => 29,
            TomeError::HookSpecParseError { .. } => 43,
            TomeError::HookSettingsWriteFailed { .. } => 44,
            TomeError::AgentTranslationFailed { .. } => 45,
            TomeError::GuardrailsWriteFailed { .. } => 46,
            TomeError::PluginNotFound(_) => 20,
            TomeError::PluginAlreadyInState { .. } => 21,
            TomeError::PluginManifestParseError { .. } => 22,
            TomeError::SkillFrontmatterParseError { .. } => 23,
            TomeError::ModelMissing { .. } => 30,
            TomeError::ModelCorrupt { .. } => 31,
            TomeError::ModelChecksumMismatch { .. } => 32,
            TomeError::ModelRegistrationParseError { .. } => 33,
            TomeError::InferenceRuntimeInitFailure(_) => 34,
            TomeError::VectorExtensionInitFailure(_) => 35,
            TomeError::EmbeddingGenerationFailure { .. } => 36,
            TomeError::RerankingFailure(_) => 37,
            TomeError::QueryNoResultsStrict { .. } => 40,
            TomeError::EmbedderNameDrift { .. } => 41,
            TomeError::EmbedderVersionDrift { .. } => 42,
            TomeError::IndexBusy => 50,
            TomeError::IndexIntegrityCheckFailure(_) => 51,
            TomeError::SchemaTooNew { .. } => 52,
            TomeError::CatalogHasEnabledPlugins { .. } => 53,
            TomeError::NotATerminal => 54,
            TomeError::McpStartupFailed { .. } => 60,
            TomeError::McpProtocolIo { .. } => 61,
            TomeError::WorkspaceMalformed { .. } => 70,
            TomeError::SchemaVersionTooNew { .. } => 73,
            TomeError::SchemaMigrationFailed { .. } => 74,
            TomeError::DoctorFixNotSafe { .. } => 75,
            TomeError::PluginNotConverted { .. } => 80,
            TomeError::OutputExists { .. } => 81,
            TomeError::TemplateInvalid { .. } => 82,
            TomeError::SourceFormatUnrecognized { .. } => 83,
            TomeError::ConversionUnsupportedStrict { .. } => 84,
            TomeError::ValidationFoundErrors { .. } => 85,
            TomeError::ValidationStrictWarnings { .. } => 86,
        }
    }
}

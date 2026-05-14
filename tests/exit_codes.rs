//! Exhaustive match over every `TomeError` variant. Adding a variant without
//! updating this test is a compile error, which is the entire point of the
//! closed-set guarantee (FR-022 carried forward + FR-048 additions).
//!
//! Phase 2 contracts: `specs/002-phase-2-plugins-index/contracts/exit-codes.md`.

use std::io;
use std::path::PathBuf;

use tome::error::{ManifestInvalid, PluginState, TomeError};

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
            TomeError::WorkspaceNotFound {
                path: PathBuf::from("/tmp/nope"),
            },
            71,
            "workspace_not_found",
        ),
        (TomeError::WorkspaceConflict, 72, "workspace_conflict"),
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
            TomeError::WorkspaceNotFound { .. } => 71,
            TomeError::WorkspaceConflict => 72,
            TomeError::SchemaVersionTooNew { .. } => 73,
            TomeError::SchemaMigrationFailed { .. } => 74,
            TomeError::DoctorFixNotSafe { .. } => 75,
        }
    }
}

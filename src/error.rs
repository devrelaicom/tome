//! The closed `TomeError` enum is the single source of truth for exit codes.
//! Adding a variant here forces edits to `tests/exit_codes.rs`, FR-022 / FR-048
//! in the spec, and the PRD's exit-code table — the compiler enforces the chain.
//!
//! Phase 2 contracts: `specs/002-phase-2-plugins-index/contracts/exit-codes.md`.
//! Phase 3 contracts: `specs/003-phase-3-mcp-workspaces/contracts/exit-codes-p3.md`.

use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum TomeError {
    // -----------------------------------------------------------------------
    // Phase 1 (codes 2–8, plus Internal=1). Unchanged.
    // -----------------------------------------------------------------------
    #[error("invalid usage: {0}")]
    Usage(String),

    #[error("catalog `{0}` is not registered")]
    CatalogNotFound(String),

    #[error("catalog `{0}` is already registered")]
    CatalogAlreadyExists(String),

    #[error("manifest invalid: {0}")]
    ManifestInvalid(#[from] ManifestInvalid),

    #[error("git failed for `{catalog}`: {detail}")]
    GitFailed { catalog: String, detail: String },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("interrupted by user")]
    Interrupted,

    // -----------------------------------------------------------------------
    // Phase 2 — plugin lifecycle (codes 20–23).
    // -----------------------------------------------------------------------
    #[error("plugin `{0}` is not installed under any registered catalog")]
    PluginNotFound(String),

    #[error("plugin `{plugin}` is already {state}")]
    PluginAlreadyInState { plugin: String, state: PluginState },

    #[error("plugin manifest invalid in {}: {message}", file.display())]
    PluginManifestParseError { file: PathBuf, message: String },

    #[error("skill metadata header invalid in {}: {message}", file.display())]
    SkillFrontmatterParseError { file: PathBuf, message: String },

    // -----------------------------------------------------------------------
    // Phase 2 — models on disk (codes 30–33).
    // -----------------------------------------------------------------------
    #[error("model `{model}` is missing; run `tome models download`")]
    ModelMissing { model: String },

    #[error("model `{model}` is corrupt ({detail}); run `tome models download --force`")]
    ModelCorrupt { model: String, detail: String },

    #[error(
        "model `{model}` SHA-256 mismatch: expected {expected}, got {got}; \
         run `tome models download --force` to retry"
    )]
    ModelChecksumMismatch {
        model: String,
        expected: String,
        got: String,
    },

    #[error("model registration metadata invalid in {}: {message}", file.display())]
    ModelRegistrationParseError { file: PathBuf, message: String },

    // -----------------------------------------------------------------------
    // Phase 2 — inference + vector engine init (codes 34–37).
    // -----------------------------------------------------------------------
    #[error("inference runtime failed to initialise: {0}")]
    InferenceRuntimeInitFailure(String),

    #[error("vector-search engine failed to initialise: {0}")]
    VectorExtensionInitFailure(String),

    #[error("embedding generation failed for `{input_desc}`: {detail}")]
    EmbeddingGenerationFailure { input_desc: String, detail: String },

    #[error("reranking failed: {0}")]
    RerankingFailure(String),

    // -----------------------------------------------------------------------
    // Phase 2 — query + drift (codes 40–42).
    // -----------------------------------------------------------------------
    #[error("no results above threshold {threshold} (--strict mode)")]
    QueryNoResultsStrict { threshold: f32 },

    #[error(
        "stored vectors were produced by embedder `{stored}`; \
         currently configured embedder is `{configured}`. \
         Run `tome reindex --force` to rebuild the index."
    )]
    EmbedderNameDrift { stored: String, configured: String },

    #[error(
        "stored vectors were produced by embedder version `{stored}`; \
         currently configured is `{configured}`. \
         Run `tome reindex --force` to rebuild the index."
    )]
    EmbedderVersionDrift { stored: String, configured: String },

    // -----------------------------------------------------------------------
    // Phase 2 — index + catalog interaction (codes 50–54).
    // -----------------------------------------------------------------------
    #[error("another tome process is updating the index; retry once it has finished")]
    IndexBusy,

    #[error("index database integrity check failed: {0}")]
    IndexIntegrityCheckFailure(String),

    #[error(
        "on-disk index schema is version {on_disk}; this tome understands up to {compiled}. \
         Upgrade tome to read this index."
    )]
    SchemaTooNew { on_disk: u32, compiled: u32 },

    #[error(
        "catalog `{catalog}` has enabled plugins; disable them first or pass --force.\nEnabled: {}",
        plugins.join(", ")
    )]
    CatalogHasEnabledPlugins {
        catalog: String,
        plugins: Vec<String>,
    },

    #[error("this command requires a terminal; use the non-interactive subcommand or attach a TTY")]
    NotATerminal,

    // -----------------------------------------------------------------------
    // Phase 3 — MCP server (codes 60–61).
    // -----------------------------------------------------------------------
    #[error("MCP server failed to start: {reason}")]
    McpStartupFailed { reason: String },

    #[error("MCP protocol I/O error: {source}")]
    McpProtocolIo { source: std::io::Error },

    // -----------------------------------------------------------------------
    // Phase 3 — workspace + schema (codes 70–75).
    // -----------------------------------------------------------------------
    #[error("workspace malformed at {}: {reason}\nhint: run `tome doctor` for a full diagnosis", path.display())]
    WorkspaceMalformed { path: PathBuf, reason: String },

    #[error(
        "workspace not found: {} does not contain a .tome/ marker\nhint: run `tome workspace init {}` to create one",
        path.display(),
        path.display()
    )]
    WorkspaceNotFound { path: PathBuf },

    #[error("workspace conflict: --workspace and --global cannot be combined")]
    WorkspaceConflict,

    #[error(
        "schema version too new: on-disk schema is v{on_disk}, this Tome supports up to v{expected}\nhint: upgrade Tome to a version that supports schema v{on_disk}"
    )]
    SchemaVersionTooNew { on_disk: u32, expected: u32 },

    #[error(
        "schema migration v{from} → v{to} failed: {source}\nhint: file the error against your installed Tome version"
    )]
    SchemaMigrationFailed {
        from: u32,
        to: u32,
        source: anyhow::Error,
    },

    #[error(
        "doctor: subsystem `{subsystem}` cannot be auto-fixed\nhint: see the report's `suggested fixes` section for the manual command"
    )]
    DoctorFixNotSafe { subsystem: String },

    // -----------------------------------------------------------------------
    // Internal — last-resort variant for panics caught at top level, etc.
    // No named failure above may collapse into this — that would defeat the
    // closed-set guarantee.
    // -----------------------------------------------------------------------
    #[error("internal error: {0:#}")]
    Internal(anyhow::Error),
}

/// Two-state lifecycle used by `PluginAlreadyInState` for disambiguating
/// "enable on enabled" vs "disable on disabled" without an extra exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginState {
    Enabled,
    Disabled,
}

impl std::fmt::Display for PluginState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        })
    }
}

impl TomeError {
    pub fn exit_code(&self) -> i32 {
        match self {
            // 1 — internal
            Self::Internal(_) => 1,
            // 2–8 — Phase 1
            Self::Usage(_) => 2,
            Self::CatalogNotFound(_) => 3,
            Self::CatalogAlreadyExists(_) => 4,
            Self::ManifestInvalid(_) => 5,
            Self::GitFailed { .. } => 6,
            Self::Io(_) => 7,
            Self::Interrupted => 8,
            // 20–23 — plugin lifecycle
            Self::PluginNotFound(_) => 20,
            Self::PluginAlreadyInState { .. } => 21,
            Self::PluginManifestParseError { .. } => 22,
            Self::SkillFrontmatterParseError { .. } => 23,
            // 30–33 — models on disk
            Self::ModelMissing { .. } => 30,
            Self::ModelCorrupt { .. } => 31,
            Self::ModelChecksumMismatch { .. } => 32,
            Self::ModelRegistrationParseError { .. } => 33,
            // 34–37 — inference + vector engine init
            Self::InferenceRuntimeInitFailure(_) => 34,
            Self::VectorExtensionInitFailure(_) => 35,
            Self::EmbeddingGenerationFailure { .. } => 36,
            Self::RerankingFailure(_) => 37,
            // 40–42 — query + drift
            Self::QueryNoResultsStrict { .. } => 40,
            Self::EmbedderNameDrift { .. } => 41,
            Self::EmbedderVersionDrift { .. } => 42,
            // 50–54 — index + catalog interaction
            Self::IndexBusy => 50,
            Self::IndexIntegrityCheckFailure(_) => 51,
            Self::SchemaTooNew { .. } => 52,
            Self::CatalogHasEnabledPlugins { .. } => 53,
            Self::NotATerminal => 54,
            // 60–61 — MCP server (Phase 3)
            Self::McpStartupFailed { .. } => 60,
            Self::McpProtocolIo { .. } => 61,
            // 70–75 — workspace + schema (Phase 3)
            Self::WorkspaceMalformed { .. } => 70,
            Self::WorkspaceNotFound { .. } => 71,
            Self::WorkspaceConflict => 72,
            Self::SchemaVersionTooNew { .. } => 73,
            Self::SchemaMigrationFailed { .. } => 74,
            Self::DoctorFixNotSafe { .. } => 75,
        }
    }

    /// Snake-case identifier used in `--json` error records. Maps 1:1 to the
    /// closed error set (FR-022 carried forward + FR-048 additions).
    pub fn category(&self) -> &'static str {
        match self {
            Self::Internal(_) => "internal",
            Self::Usage(_) => "usage",
            Self::CatalogNotFound(_) => "catalog_not_found",
            Self::CatalogAlreadyExists(_) => "catalog_already_exists",
            Self::ManifestInvalid(_) => "manifest_invalid",
            Self::GitFailed { .. } => "git_failed",
            Self::Io(_) => "io",
            Self::Interrupted => "interrupted",
            Self::PluginNotFound(_) => "plugin_not_found",
            Self::PluginAlreadyInState { .. } => "plugin_already_in_state",
            Self::PluginManifestParseError { .. } => "plugin_manifest_parse_error",
            Self::SkillFrontmatterParseError { .. } => "skill_frontmatter_parse_error",
            Self::ModelMissing { .. } => "model_missing",
            Self::ModelCorrupt { .. } => "model_corrupt",
            Self::ModelChecksumMismatch { .. } => "model_checksum_mismatch",
            Self::ModelRegistrationParseError { .. } => "model_registration_parse_error",
            Self::InferenceRuntimeInitFailure(_) => "inference_runtime_init_failure",
            Self::VectorExtensionInitFailure(_) => "vector_extension_init_failure",
            Self::EmbeddingGenerationFailure { .. } => "embedding_generation_failure",
            Self::RerankingFailure(_) => "reranking_failure",
            Self::QueryNoResultsStrict { .. } => "query_no_results_strict",
            Self::EmbedderNameDrift { .. } => "embedder_name_drift",
            Self::EmbedderVersionDrift { .. } => "embedder_version_drift",
            Self::IndexBusy => "index_busy",
            Self::IndexIntegrityCheckFailure(_) => "index_integrity_check_failure",
            Self::SchemaTooNew { .. } => "schema_too_new",
            Self::CatalogHasEnabledPlugins { .. } => "catalog_has_enabled_plugins",
            Self::NotATerminal => "not_a_terminal",
            Self::McpStartupFailed { .. } => "mcp_startup",
            Self::McpProtocolIo { .. } => "mcp_io",
            Self::WorkspaceMalformed { .. } => "workspace_malformed",
            Self::WorkspaceNotFound { .. } => "workspace_not_found",
            Self::WorkspaceConflict => "workspace_conflict",
            Self::SchemaVersionTooNew { .. } => "schema_too_new",
            Self::SchemaMigrationFailed { .. } => "schema_migration",
            Self::DoctorFixNotSafe { .. } => "doctor_fix_unsafe",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestInvalid {
    #[error("unknown field `{key}` in {}: see {expected_schema_uri}", file.display())]
    UnknownField {
        file: PathBuf,
        key: String,
        expected_schema_uri: String,
    },

    #[error("missing required field `{key}` in {}", file.display())]
    MissingField { file: PathBuf, key: String },

    #[error("`version` in {} is not a valid semver: {got}", file.display())]
    InvalidVersion { file: PathBuf, got: String },

    #[error("`owner.email` in {} is not a valid email: {got}", file.display())]
    InvalidEmail { file: PathBuf, got: String },

    #[error("duplicate plugin name `{name}` in {}", file.display())]
    DuplicatePluginName { file: PathBuf, name: String },

    #[error(
        "`plugins[].source = \"{value}\"` in {} looks like a URL — Phase 1 supports relative paths only",
        file.display()
    )]
    SourceLooksLikeUrl { file: PathBuf, value: String },

    #[error(
        "`plugins[].source = \"{value}\"` in {} is an absolute path — must be a relative path within the catalog repo",
        file.display()
    )]
    SourceAbsolute { file: PathBuf, value: String },

    #[error(
        "`plugins[].source = \"{value}\"` in {} contains `..` — must be a normalised relative path",
        file.display()
    )]
    SourceParentTraversal { file: PathBuf, value: String },

    #[error("`plugins[].source = \"{value}\"` in {} resolves outside the catalog repo", file.display())]
    SourceEscapesRoot { file: PathBuf, value: String },

    #[error(
        "`plugins[].source = \"{value}\"` in {} does not exist or is unreachable: {cause}",
        file.display()
    )]
    SourceUnresolvable {
        file: PathBuf,
        value: String,
        cause: std::io::Error,
    },

    #[error("could not canonicalise catalog root {}: {cause}", root.display())]
    CatalogRootUnresolvable {
        root: PathBuf,
        cause: std::io::Error,
    },

    #[error("toml parse error in {}: {message}", file.display())]
    TomlParse { file: PathBuf, message: String },
}

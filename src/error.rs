//! The closed `TomeError` enum is the single source of truth for exit codes.
//! Adding a variant here forces edits to `tests/exit_codes.rs`, FR-022 / FR-048
//! in the spec, and the PRD's exit-code table — the compiler enforces the chain.
//!
//! Phase 2 contracts: `specs/002-phase-2-plugins-index/contracts/exit-codes.md`.
//! Phase 3 contracts: `specs/003-phase-3-mcp-workspaces/contracts/exit-codes-p3.md`.
//! Phase 4 contracts: `specs/004-phase-4-refactor-harnesses/contracts/exit-codes-p4.md`.

use std::path::PathBuf;

use crate::workspace::ScopeKind;

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

    /// Generic filesystem error. Phase 4 widens the semantic scope of this
    /// variant per FR-602 of `contracts/exit-codes-p4.md`: per-user state
    /// directory unwritable, project-binding I/O, and other Phase 4
    /// filesystem failures all collapse onto `Io` (code 7) rather than
    /// promoting new variants. The exit code and category string stay
    /// stable; the new failure surfaces are visually distinguishable in
    /// the inner `io::Error` payload.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("interrupted by user")]
    Interrupted,

    // -----------------------------------------------------------------------
    // Phase 4 — workspace name + project binding (codes 13–16).
    // Pre-allocated by F3; first wired by US1/US2 (workspace lifecycle +
    // project binding). See data-model.md §14 and contracts/exit-codes-p4.md.
    // -----------------------------------------------------------------------
    #[error("workspace `{name}` not found in the central registry")]
    WorkspaceNotFound { name: String },

    #[error("workspace `{name}` already exists")]
    WorkspaceAlreadyExists { name: String },

    #[error("workspace name `{name}` is invalid: {reason}")]
    WorkspaceNameInvalid { name: String, reason: String },

    #[error(
        "workspace `{name}` has {count} bound project(s); refusing without --force\nBound: {}",
        projects.join(", ")
    )]
    WorkspaceHasBoundProjects {
        name: String,
        count: usize,
        projects: Vec<String>,
    },

    // -----------------------------------------------------------------------
    // Phase 4 — harness composition + integration (codes 17–19).
    // -----------------------------------------------------------------------
    #[error("harness composition error: {kind}")]
    CompositionError { kind: CompositionErrorKind },

    #[error("harness `{name}` is not supported")]
    HarnessNotSupported { name: String },

    #[error(
        "harness MCP config clash in {}: existing entry named `tome` does not match Tome's expected shape (command=`{command}`, first_arg=`{first_arg}`)\nhint: rerun with --force to overwrite, or `tome workspace use <name> --force` to repair after the clash is resolved; `tome doctor --fix` will also surface the residual report",
        path.display()
    )]
    HarnessClash {
        path: PathBuf,
        command: String,
        first_arg: String,
    },

    // -----------------------------------------------------------------------
    // Phase 4 — summariser (code 24; contract says 20 but that conflicts with
    // Phase 2's `PluginNotFound`; see `exit_code()` for the resolution note).
    // -----------------------------------------------------------------------
    #[error("summariser failure: {kind}")]
    SummariserFailure { kind: SummariserFailureKind },

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
    /// Workspace's on-disk shape failed validation. Phase 4 widens the
    /// semantic scope of this variant per FR-602 of
    /// `contracts/exit-codes-p4.md`: it now also covers (a) a project
    /// marker `<project>/.tome/config.toml` that exists but is unparsable
    /// or names a workspace via a malformed `workspace` key, and (b) a
    /// workspace rename precondition where the bound project directory
    /// recorded in the central registry no longer exists on disk. The
    /// exit code (70) and category string (`"workspace_malformed"`) stay
    /// stable; the inner `reason` payload disambiguates.
    ///
    /// Exit codes 71 (`WorkspaceMarkerMissing`) and 72 (`WorkspaceConflict`)
    /// were live in Phase 3 but are deleted in Phase 4 / F10: the Phase 4
    /// resolver targets validated workspace **names** against the central
    /// registry (so a missing marker is no longer a distinct failure mode),
    /// and the `--global` flag is gone (so the workspace/global conflict is
    /// not expressible). Codes 71 and 72 stay reserved-but-unused — we do
    /// NOT reassign mid-phase to preserve the wire-stable closed set.
    #[error("workspace malformed at {}: {reason}\nhint: run `tome doctor` for a full diagnosis", path.display())]
    WorkspaceMalformed { path: PathBuf, reason: String },

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
            // 13–16 — Phase 4 workspace name + project binding
            Self::WorkspaceNotFound { .. } => 13,
            Self::WorkspaceAlreadyExists { .. } => 14,
            Self::WorkspaceNameInvalid { .. } => 15,
            Self::WorkspaceHasBoundProjects { .. } => 16,
            // 17–19 — Phase 4 harness composition + integration
            Self::CompositionError { .. } => 17,
            Self::HarnessNotSupported { .. } => 18,
            Self::HarnessClash { .. } => 19,
            // 24 — Phase 4 summariser. Note: `contracts/exit-codes-p4.md`
            // ships code 20 for this variant, which collides with Phase 2's
            // pre-existing `PluginNotFound` (20). Constitutional principle II
            // (NON-NEGOTIABLE) requires pairwise-unique exit codes; the
            // first numerically-adjacent free slot after the Phase 2 plugin
            // range (20–23) is 24. F3 lands `SummariserFailure` here and
            // flags the contract typo for reconciliation in F4+.
            Self::SummariserFailure { .. } => 24,
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
            // 70 — workspace malformed (Phase 3, widened in Phase 4 / F10).
            // 71 + 72 are reserved-but-unused as of Phase 4 / F10 (see the
            // `WorkspaceMalformed` doc comment).
            // 73–75 — schema migration + doctor (Phase 3).
            Self::WorkspaceMalformed { .. } => 70,
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
            // Phase 4 — workspace name + project binding
            Self::WorkspaceNotFound { .. } => "workspace_not_found",
            Self::WorkspaceAlreadyExists { .. } => "workspace_already_exists",
            Self::WorkspaceNameInvalid { .. } => "workspace_name_invalid",
            Self::WorkspaceHasBoundProjects { .. } => "workspace_has_bound_projects",
            // Phase 4 — harness composition + integration
            Self::CompositionError { .. } => "composition_error",
            Self::HarnessNotSupported { .. } => "harness_not_supported",
            Self::HarnessClash { .. } => "harness_clash",
            // Phase 4 — summariser
            Self::SummariserFailure { .. } => "summariser_failure",
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
            Self::SchemaVersionTooNew { .. } => "schema_too_new",
            Self::SchemaMigrationFailed { .. } => "schema_migration",
            Self::DoctorFixNotSafe { .. } => "doctor_fix_unsafe",
        }
    }
}

/// Sub-classification for `TomeError::CompositionError` (exit 17). Pre-allocated
/// by F3; consumers wire it in F8 (settings composition) and US3 (layered
/// settings + composition). The `WorkspaceRefOutsideProject` variant carries a
/// `ScopeKind` rather than a stringly-typed scope label so the type system
/// rejects malformed states. The `UnknownWorkspace` sub-variant is allowed by
/// the contract to map to exit 13 (`WorkspaceNotFound`) instead of 17 — the
/// caller chooses, by emitting whichever `TomeError` variant matches the
/// composition surface the failure came from.
#[derive(Debug)]
pub enum CompositionErrorKind {
    /// DFS cycle detected during workspace-reference resolution. The path
    /// names the chain that triggered the cycle, in the order it was walked.
    Cycle { path: Vec<String> },
    /// A `[workspace]` reference was found in a scope that may not carry
    /// one (workspace or global settings — only project-scoped settings
    /// may reference workspaces).
    WorkspaceRefOutsideProject { found_in: ScopeKind },
    /// A `[workspaces.<name>]` block named a workspace that does not exist
    /// in the central registry. Per `contracts/exit-codes-p4.md`, callers
    /// may surface this as exit 13 (`WorkspaceNotFound`) instead of exit
    /// 17 when the failure is reachable from the workspace-resolution
    /// surface.
    UnknownWorkspace(String),
    /// A `!`-prefixed exclusion in a composition was malformed (not a
    /// plain name, contained traversal characters, etc.). The string
    /// carries the offending token.
    BadExclusion(String),
    /// A `[workspaces.<name>]` reference resolved through the central
    /// registry membership check (the named workspace EXISTS), but the
    /// workspace's on-disk `settings.toml` could not be read or parsed.
    /// This is distinct from `UnknownWorkspace` (which means the registry
    /// has no row for `<name>`): the row is present but the data backing
    /// it is malformed. Routed to [`TomeError::WorkspaceMalformed`] (exit
    /// 70) by the boundary impl below.
    ///
    /// First field is the workspace name; second is a short human-readable
    /// reason (IO error message or parse failure detail).
    SettingsReadFailure(String, String),
    /// A harness name surfaced by composition resolution is not in
    /// [`crate::harness::SUPPORTED_HARNESSES`]. Internal carrier — the
    /// boundary `From<CompositionErrorKind> for TomeError` impl below
    /// rewrites this into [`TomeError::HarnessNotSupported`] (exit 18)
    /// so the wire-visible error code matches the contract.
    HarnessNotSupported(String),
}

impl std::fmt::Display for CompositionErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Cycle { path } => {
                write!(f, "composition cycle: {}", path.join(" → "))
            }
            Self::WorkspaceRefOutsideProject { found_in } => write!(
                f,
                "`[workspace]` reference found in {found_in:?} settings — only project-scoped settings may reference workspaces"
            ),
            Self::UnknownWorkspace(name) => {
                write!(f, "unknown workspace `{name}` referenced in composition")
            }
            Self::BadExclusion(token) => {
                write!(f, "malformed `!`-prefixed exclusion: `{token}`")
            }
            Self::SettingsReadFailure(name, reason) => {
                write!(f, "workspace `{name}` settings could not be read: {reason}")
            }
            Self::HarnessNotSupported(name) => {
                write!(f, "harness `{name}` is not supported")
            }
        }
    }
}

/// Boundary mapping from the resolver's internal error kind to the closed
/// `TomeError` enum. Two sub-variants escape the default `CompositionError`
/// (exit 17) wrapping per FR-602:
///
/// * [`CompositionErrorKind::UnknownWorkspace`] → [`TomeError::WorkspaceNotFound`]
///   (exit 13). The composition-vs-workspace-resolution surface distinction
///   doesn't matter to the user — an unknown workspace is an unknown
///   workspace, surfaced through the workspace-error path.
/// * [`CompositionErrorKind::HarnessNotSupported`] → [`TomeError::HarnessNotSupported`]
///   (exit 18). The unsupported-harness check lives inside composition but
///   reports through its own exit code per the
///   `contracts/settings-composition.md` error table.
///
/// Everything else (cycles, `[workspace]` outside project, bad exclusions)
/// maps to [`TomeError::CompositionError`] (exit 17).
impl From<CompositionErrorKind> for TomeError {
    fn from(kind: CompositionErrorKind) -> Self {
        match kind {
            CompositionErrorKind::UnknownWorkspace(name) => Self::WorkspaceNotFound { name },
            CompositionErrorKind::HarnessNotSupported(name) => Self::HarnessNotSupported { name },
            CompositionErrorKind::SettingsReadFailure(name, reason) => {
                // Promote the workspace name into a synthetic path so
                // `WorkspaceMalformed`'s Display can render it without
                // additional context. The reason captures the underlying
                // IO/parse failure verbatim.
                Self::WorkspaceMalformed {
                    path: PathBuf::from(format!("workspaces/{name}/settings.toml")),
                    reason,
                }
            }
            other => Self::CompositionError { kind: other },
        }
    }
}

/// Sub-classification for `TomeError::SummariserFailure` (exit 20 per the
/// contract; placed at exit 24 in implementation to avoid colliding with
/// Phase 2's `PluginNotFound`). Pre-allocated by F3; consumers wire it in
/// F6 (summariser skeleton + `StubSummariser`) and US4 (RULES.md
/// regeneration). `ShortOrLong` disambiguates which of the two prompt-driven
/// outputs failed for `OutputUnparsable` and `OutputEmpty`.
#[derive(Debug)]
pub enum SummariserFailureKind {
    /// The summariser model is not present on disk at the point summarisation
    /// was requested. Auto-fixable by `tome doctor --fix` (re-download).
    ModelMissing,
    /// The on-disk model file's SHA-256 disagrees with the registry pin.
    /// Distinct from the embedder/reranker variants because the summariser
    /// uses a different inference runtime (llama-cpp-2) and a different
    /// recovery flow.
    ModelChecksumMismatch { expected: String, observed: String },
    /// `LlamaBackend::init()` or similar initialisation failed. The
    /// `source` string preserves the underlying error message for the
    /// human reader; the type itself stays stringly-typed to avoid leaking
    /// the inference-runtime crate's error types into `TomeError`'s
    /// public API.
    BackendInitFailed { source: String },
    /// The model returned output that the parser could not interpret as
    /// the requested summary kind. `which` names which prompt was active
    /// when the failure occurred.
    OutputUnparsable { which: ShortOrLong },
    /// The model returned an empty string for the requested summary kind.
    OutputEmpty { which: ShortOrLong },
}

impl std::fmt::Display for SummariserFailureKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ModelMissing => f.write_str("model missing"),
            Self::ModelChecksumMismatch { expected, observed } => write!(
                f,
                "model checksum mismatch: expected {expected}, observed {observed}"
            ),
            Self::BackendInitFailed { source } => {
                write!(f, "backend init failed: {source}")
            }
            Self::OutputUnparsable { which } => {
                write!(f, "model output unparsable ({which} summary)")
            }
            Self::OutputEmpty { which } => {
                write!(f, "model output empty ({which} summary)")
            }
        }
    }
}

/// Discriminator for `SummariserFailureKind::OutputUnparsable` /
/// `OutputEmpty` — names which of the two prompt-driven outputs failed.
/// Pre-allocated by F3; the short/long distinction is wired in F6
/// alongside the summariser prompt set.
#[derive(Debug, Clone, Copy)]
pub enum ShortOrLong {
    Short,
    Long,
}

impl std::fmt::Display for ShortOrLong {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Short => "short",
            Self::Long => "long",
        })
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

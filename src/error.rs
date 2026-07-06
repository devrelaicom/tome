//! The closed `TomeError` enum is the single source of truth for exit codes.
//! Adding a variant here forces edits to `tests/exit_codes.rs`, FR-022 / FR-048
//! in the spec, and the PRD's exit-code table — the compiler enforces the chain.
//!
//! Phase 2 contracts: `specs/002-phase-2-plugins-index/contracts/exit-codes.md`.
//! Phase 3 contracts: `specs/003-phase-3-mcp-workspaces/contracts/exit-codes-p3.md`.
//! Phase 4 contracts: `specs/004-phase-4-refactor-harnesses/contracts/exit-codes-p4.md`.
//! Phase 5 contracts: `specs/005-phase-5-commands-prompts/contracts/exit-codes-p5.md`.

use std::path::PathBuf;

use crate::workspace::ScopeKind;

/// Raw process exit code for the **Degraded** overall-health verdict emitted by
/// `tome status` and `tome doctor` (issue #282).
///
/// These two commands never fail with a `TomeError` for a health verdict —
/// doing so would suppress the report they exist to print. Instead they render
/// the report and then call `std::process::exit` directly with a health code:
/// `0` when Healthy, this code when **Degraded**, and `1` when **Unhealthy**.
/// Splitting Degraded off from Unhealthy lets a CI gate fail-on-unhealthy-only
/// (branch on this code, or on the `--json` `overall` field) while a plain
/// "fail on any non-zero" gate is unaffected — Degraded is still non-zero.
///
/// `10` is the first free slot in the exit-code space and is NOT a `TomeError`
/// variant: it lives outside the closed 1:1 [`TomeError`] → exit-code map on
/// purpose, so adding it does not touch that contract. It must stay distinct
/// from every code returned by [`TomeError::exit_code`] and from the Unhealthy
/// code (`1`). `status` and `doctor` both reference this constant so the two
/// surfaces cannot diverge. See `site/docs/reference/exit-codes.md`.
pub const EXIT_HEALTH_DEGRADED: i32 = 10;

/// Raw process exit code for the **Unhealthy** overall-health verdict emitted by
/// `tome status` and `tome doctor` (issue #282). Unchanged from the historical
/// value (`1`) to minimise churn — see [`EXIT_HEALTH_DEGRADED`] for the scheme.
pub const EXIT_HEALTH_UNHEALTHY: i32 = 1;

/// One row of the exit-code reference table ([`EXIT_CODES`]).
#[derive(Debug, Clone, Copy)]
pub struct ExitCodeInfo {
    /// The process exit code.
    pub code: i32,
    /// The `--json` error-envelope `category` slug (exactly what
    /// [`ErrorCategory::as_str`] emits for the [`TomeError`] variant(s) behind
    /// this code). Two special non-error rows: `None` for success (`0`) and
    /// `"health_degraded"` for the status/doctor health verdict (`10`, which is
    /// not a `TomeError` — see [`EXIT_HEALTH_DEGRADED`]).
    pub category: Option<&'static str>,
    /// One-line human meaning.
    pub meaning: &'static str,
}

/// #436: the single static source `tome exit-codes` renders — every exit code
/// with its category slug and a one-line meaning, ascending.
///
/// THE DRIFT GUARD IS THE POINT. This table is pinned in both directions by
/// `tests/index_query_misc/exit_codes.rs`:
///
/// * every code [`TomeError::exit_code`] can return must appear here with
///   exactly its [`TomeError::category`] slug (driven off the same exhaustive
///   variant enumeration that pins the code mapping itself), and
/// * the docs page `site/docs/reference/exit-codes.md` must list the identical
///   `(code, category)` rows in the same order.
///
/// Codes `71`/`72` are reserved-but-unused (deleted in Phase 4 / F10, never
/// reassigned) and deliberately absent: `exit_code()` cannot return them and
/// the docs page does not list them. Codes `90`/`91` are vestigial — retained
/// for the closed-set contract, not constructed today — and say so.
pub static EXIT_CODES: &[ExitCodeInfo] = &[
    ExitCodeInfo {
        code: 0,
        category: None,
        meaning: "Success.",
    },
    ExitCodeInfo {
        code: 1,
        category: Some("internal"),
        meaning: "Internal error.",
    },
    ExitCodeInfo {
        code: 2,
        category: Some("usage"),
        meaning: "Invalid usage / arguments.",
    },
    ExitCodeInfo {
        code: 3,
        category: Some("catalog_not_found"),
        meaning: "Catalog not found.",
    },
    ExitCodeInfo {
        code: 4,
        category: Some("catalog_already_exists"),
        meaning: "Catalog already exists.",
    },
    ExitCodeInfo {
        code: 5,
        category: Some("manifest_invalid"),
        meaning: "Catalog manifest (tome-catalog.toml) invalid.",
    },
    ExitCodeInfo {
        code: 6,
        category: Some("git_failed"),
        meaning: "A git operation failed.",
    },
    ExitCodeInfo {
        code: 7,
        category: Some("io"),
        meaning: "I/O error.",
    },
    ExitCodeInfo {
        code: 8,
        category: Some("interrupted"),
        meaning: "Interrupted (SIGINT / Ctrl-C).",
    },
    ExitCodeInfo {
        code: 9,
        category: Some("plugin_data_dir_write_failed"),
        meaning: "Failed to write a plugin's data directory.",
    },
    ExitCodeInfo {
        code: 10,
        category: Some("health_degraded"),
        meaning: "tome status / tome doctor health verdict: degraded (a non-fatal issue — queries still serve).",
    },
    ExitCodeInfo {
        code: 12,
        category: Some("workspace_not_bound"),
        meaning: "No workspace is bound to the current directory (tome workspace current).",
    },
    ExitCodeInfo {
        code: 13,
        category: Some("workspace_not_found"),
        meaning: "Workspace not found.",
    },
    ExitCodeInfo {
        code: 14,
        category: Some("workspace_already_exists"),
        meaning: "Workspace already exists.",
    },
    ExitCodeInfo {
        code: 15,
        category: Some("workspace_name_invalid"),
        meaning: "Invalid workspace name.",
    },
    ExitCodeInfo {
        code: 16,
        category: Some("workspace_has_bound_projects"),
        meaning: "Workspace still has bound projects.",
    },
    ExitCodeInfo {
        code: 17,
        category: Some("composition_error"),
        meaning: "Workspace composition error.",
    },
    ExitCodeInfo {
        code: 18,
        category: Some("harness_not_supported"),
        meaning: "Unsupported harness.",
    },
    ExitCodeInfo {
        code: 19,
        category: Some("harness_clash"),
        meaning: "Harness configuration clash.",
    },
    ExitCodeInfo {
        code: 20,
        category: Some("plugin_not_found"),
        meaning: "Plugin not found.",
    },
    ExitCodeInfo {
        code: 21,
        category: Some("plugin_already_in_state"),
        meaning: "Plugin already in the requested state.",
    },
    ExitCodeInfo {
        code: 22,
        category: Some("plugin_manifest_parse_error"),
        meaning: "Plugin manifest (tome-plugin.toml) parse error.",
    },
    ExitCodeInfo {
        code: 23,
        category: Some("skill_frontmatter_parse_error"),
        meaning: "SKILL.md frontmatter parse error.",
    },
    ExitCodeInfo {
        code: 24,
        category: Some("summariser_failure"),
        meaning: "Summariser failure.",
    },
    ExitCodeInfo {
        code: 25,
        category: Some("workspace_data_dir_write_failed"),
        meaning: "Failed to write a workspace's data directory.",
    },
    ExitCodeInfo {
        code: 26,
        category: Some("prompt_argument_mismatch"),
        meaning: "MCP prompt argument mismatch.",
    },
    ExitCodeInfo {
        code: 27,
        category: Some("entry_not_found"),
        meaning: "Entry not found.",
    },
    ExitCodeInfo {
        code: 28,
        category: Some("substitution_failed"),
        meaning: "Variable substitution failed.",
    },
    ExitCodeInfo {
        code: 29,
        category: Some("invalid_argument_frontmatter"),
        meaning: "Invalid argument frontmatter.",
    },
    ExitCodeInfo {
        code: 30,
        category: Some("model_missing"),
        meaning: "A required model is missing.",
    },
    ExitCodeInfo {
        code: 31,
        category: Some("model_corrupt"),
        meaning: "A model file is corrupt.",
    },
    ExitCodeInfo {
        code: 32,
        category: Some("model_checksum_mismatch"),
        meaning: "Model checksum mismatch.",
    },
    ExitCodeInfo {
        code: 33,
        category: Some("model_registration_parse_error"),
        meaning: "Model registration parse error.",
    },
    ExitCodeInfo {
        code: 34,
        category: Some("inference_runtime_init_failure"),
        meaning: "Inference runtime failed to initialise.",
    },
    ExitCodeInfo {
        code: 35,
        category: Some("vector_extension_init_failure"),
        meaning: "Vector extension failed to initialise.",
    },
    ExitCodeInfo {
        code: 36,
        category: Some("embedding_generation_failure"),
        meaning: "Embedding generation failed.",
    },
    ExitCodeInfo {
        code: 37,
        category: Some("reranking_failure"),
        meaning: "Reranking failed.",
    },
    ExitCodeInfo {
        code: 40,
        category: Some("query_no_results_strict"),
        meaning: "--strict query returned no results.",
    },
    ExitCodeInfo {
        code: 41,
        category: Some("embedder_name_drift"),
        meaning: "Embedder name drift (index vs. configured model).",
    },
    ExitCodeInfo {
        code: 42,
        category: Some("embedder_version_drift"),
        meaning: "Embedder version drift.",
    },
    ExitCodeInfo {
        code: 43,
        category: Some("hook_spec_parse_error"),
        meaning: "Hook spec parse error.",
    },
    ExitCodeInfo {
        code: 44,
        category: Some("hook_settings_write_failed"),
        meaning: "Failed to write hook settings.",
    },
    ExitCodeInfo {
        code: 45,
        category: Some("agent_translation_failed"),
        meaning: "Agent translation failed.",
    },
    ExitCodeInfo {
        code: 46,
        category: Some("guardrails_write_failed"),
        meaning: "Failed to write the guardrails file.",
    },
    ExitCodeInfo {
        code: 47,
        category: Some("reindex_scoped_embedder_change"),
        meaning: "A scoped reindex was refused because the embedder changed — run a full tome reindex.",
    },
    ExitCodeInfo {
        code: 50,
        category: Some("index_busy"),
        meaning: "The index is locked by another process.",
    },
    ExitCodeInfo {
        code: 51,
        category: Some("index_integrity_check_failure"),
        meaning: "Index integrity check failed.",
    },
    ExitCodeInfo {
        code: 52,
        category: Some("schema_too_new"),
        meaning: "Index schema is newer than this binary supports.",
    },
    ExitCodeInfo {
        code: 53,
        category: Some("catalog_has_enabled_plugins"),
        meaning: "Catalog still has enabled plugins (use --force).",
    },
    ExitCodeInfo {
        code: 54,
        category: Some("not_a_terminal"),
        meaning: "An interactive command was run without a terminal.",
    },
    ExitCodeInfo {
        code: 60,
        category: Some("mcp_startup"),
        meaning: "MCP server failed to start.",
    },
    ExitCodeInfo {
        code: 61,
        category: Some("mcp_io"),
        meaning: "MCP protocol I/O error.",
    },
    ExitCodeInfo {
        code: 70,
        category: Some("workspace_malformed"),
        meaning: "Workspace data on disk is malformed.",
    },
    ExitCodeInfo {
        code: 73,
        category: Some("schema_too_new"),
        meaning: "Workspace schema version too new.",
    },
    ExitCodeInfo {
        code: 74,
        category: Some("schema_migration"),
        meaning: "Schema migration failed.",
    },
    ExitCodeInfo {
        code: 75,
        category: Some("doctor_fix_unsafe"),
        meaning: "A doctor --fix repair was not safe to apply.",
    },
    ExitCodeInfo {
        code: 80,
        category: Some("plugin_not_converted"),
        meaning: "Plugin not converted: legacy .claude-plugin/plugin.json exists but no tome-plugin.toml.",
    },
    ExitCodeInfo {
        code: 81,
        category: Some("output_exists"),
        meaning: "Refusing to overwrite existing output (pass --force).",
    },
    ExitCodeInfo {
        code: 82,
        category: Some("template_invalid"),
        meaning: "Template unusable (missing file, malformed template, render error).",
    },
    ExitCodeInfo {
        code: 83,
        category: Some("source_format_unrecognized"),
        meaning: "Could not auto-detect source format (pass --from <harness>).",
    },
    ExitCodeInfo {
        code: 84,
        category: Some("conversion_unsupported_strict"),
        meaning: "convert --strict hit an unsupported feature.",
    },
    ExitCodeInfo {
        code: 85,
        category: Some("validation_found_errors"),
        meaning: "lint found at least one error.",
    },
    ExitCodeInfo {
        code: 86,
        category: Some("validation_strict_warnings"),
        meaning: "lint --strict found warnings (and no errors).",
    },
    ExitCodeInfo {
        code: 87,
        category: Some("meta_skill_not_found"),
        meaning: "Unknown bundled meta skill id.",
    },
    ExitCodeInfo {
        code: 88,
        category: Some("meta_install_failed"),
        meaning: "Failed to install a meta skill.",
    },
    ExitCodeInfo {
        code: 89,
        category: Some("no_harness_detected"),
        meaning: "No supported harness detected (use --harness or install one).",
    },
    ExitCodeInfo {
        code: 90,
        category: Some("telemetry_endpoint_unreachable"),
        meaning: "Telemetry endpoint unreachable (vestigial — retained for the closed-set contract; not constructed today).",
    },
    ExitCodeInfo {
        code: 91,
        category: Some("telemetry_config_invalid"),
        meaning: "Telemetry config invalid (vestigial — retained for the closed-set contract; not constructed today).",
    },
    ExitCodeInfo {
        code: 92,
        category: Some("telemetry_queue_corrupt"),
        meaning: "Telemetry queue corrupt: unparsable lines were dropped (tome telemetry inspect).",
    },
    ExitCodeInfo {
        code: 93,
        category: Some("provider_config_invalid"),
        meaning: "Provider config invalid: an undefined provider reference, a kind illegal for the capability, a provider set without a model, or no resolvable credential.",
    },
    ExitCodeInfo {
        code: 94,
        category: Some("provider_request_failed"),
        meaning: "A remote provider request failed (auth, rate-limit, timeout, unreachable, malformed response).",
    },
    ExitCodeInfo {
        code: 95,
        category: Some("remote_embedding_invalid"),
        meaning: "A remote embedding failed content validation (empty / non-finite / wrong dimension).",
    },
];

#[derive(Debug, thiserror::Error)]
pub enum TomeError {
    // -----------------------------------------------------------------------
    // Phase 1 (codes 2–8, plus Internal=1). Unchanged.
    // -----------------------------------------------------------------------
    #[error("invalid usage: {0}")]
    Usage(String),

    #[error(
        "catalog `{0}` is not registered\nhint: list enrolled catalogs with `tome catalog list`, or enrol one with `tome catalog add <source>`"
    )]
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
    #[error(
        "workspace `{name}` not found in the central registry\nhint: create it with `tome workspace init {name}`"
    )]
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

    /// No workspace resolves to the current directory. Distinct from
    /// [`Self::WorkspaceNotFound`] (a *named* workspace that is absent from
    /// the central registry): here the resolver reached `GlobalFallback`
    /// with no `--workspace` flag, no `TOME_WORKSPACE`, no `[workspace]
    /// default`, and no project-marker binding — so there is nothing to
    /// name. Surfaced by `tome workspace current` for shell prompts /
    /// scripting; its message must be actionable (bind, or select), which
    /// `WorkspaceNotFound`'s registry-oriented "create it with `init`"
    /// wording is not. Exit code 12 — the free slot immediately below the
    /// 13–16 workspace name/binding cluster, keeping the variant in the
    /// workspace family (a genuinely new failure class gets a new code per
    /// constitution principle II; the strict 1:1 closed set forbids sharing
    /// code 13).
    #[error(
        "no workspace is bound to the current directory\nhint: bind one with `tome workspace use <name>`, or select one with `--workspace <name>`"
    )]
    WorkspaceNotBound,

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
    // Phase 5 — commands-as-prompts + substitution layer (codes 25–29).
    // Pre-allocated by F1; first wired by US1 (`EntryNotFound` via the read
    // tool / prompts/get / plugin-show), US2 (data-dir create_dir_all failures
    // — `WorkspaceDataDirWriteFailed`), US3 (argument substitution +
    // `PromptArgumentMismatch`), US1 plugin-enable
    // (`InvalidArgumentFrontmatter`), and the substitution engine's
    // unrecoverable failure surface (`SubstitutionFailed`).
    //
    // The contract `contracts/exit-codes-p5.md` originally proposed codes
    // 21/22/23 for `EntryNotFound`/`SubstitutionFailed`/`InvalidArgumentFrontmatter`
    // but those collide with Phase 2's `PluginAlreadyInState` (21),
    // `PluginManifestParseError` (22), and `SkillFrontmatterParseError` (23).
    // Per constitution principle II (NON-NEGOTIABLE pairwise-unique exit
    // codes) F1 reassigns them to 27/28/29 — same precedent as Phase 4 F3
    // which moved `SummariserFailure` from contract-proposed 20 to actual
    // 24 to dodge `PluginNotFound`. Phase 5 ends up occupying a clean
    // contiguous cluster at 25–29.
    // -----------------------------------------------------------------------
    #[error("workspace data directory write failed at {}: {source}", path.display())]
    WorkspaceDataDirWriteFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    /// Plugin-data directory `create_dir_all` failed. Companion to
    /// [`Self::WorkspaceDataDirWriteFailed`] (exit 25). US1.d reviewer
    /// pass (R-M1) split the original combined variant in two because
    /// `${TOME_PLUGIN_DATA}` and `${TOME_WORKSPACE_DATA}` live under
    /// distinct directory roots and the `path` payload alone is a
    /// confusing way for a reader to learn which one failed — variant
    /// name + exit code should carry the discriminator. Exit code 9 is
    /// the lowest free slot in the Phase 1 I/O cluster (Phase 1 occupies
    /// 1–8, then 13+); semantically I/O-adjacent. Mirrors the substitution
    /// engine's matching split (`SubstitutionError::PluginDataDirCreationFailed`
    /// vs `WorkspaceDataDirCreationFailed`).
    #[error("plugin data dir write failed at {}: {source}", path.display())]
    PluginDataDirWriteFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("prompt argument mismatch: expected {expected}, supplied {supplied}")]
    PromptArgumentMismatch { expected: usize, supplied: usize },

    #[error("entry not found: {catalog}/{plugin}/{name} (kind: {kind})")]
    EntryNotFound {
        catalog: String,
        plugin: String,
        name: String,
        /// Stringly-typed for now ("skill" or "command"); promotion to an
        /// `EntryKind` enum can wait until US1 wires the read tool and the
        /// discriminator's two-valued domain becomes load-bearing on the
        /// receiver side.
        kind: String,
    },

    #[error("substitution failed: {reason}")]
    SubstitutionFailed { reason: String },

    #[error("invalid argument frontmatter in {}: {reason}", file.display())]
    InvalidArgumentFrontmatter { file: PathBuf, reason: String },

    // -----------------------------------------------------------------------
    // Phase 6 — hooks + agents (codes 43–46).
    //
    // The PRD's first draft proposed 30–33 but those collide with the
    // model-on-disk cluster (`ModelMissing` 30 … `ModelRegistrationParseError`
    // 33) and 34–37 are the inference/vector cluster. Per
    // `contracts/exit-codes-p6.md` (research R-1) Phase 6 claims the first
    // free contiguous run, 43–46 — same reassignment precedent as the
    // Phase 4 summariser (proposed 20 → shipped 24) and the Phase 5 cluster
    // (proposed 21–23 → shipped 25–29).
    // -----------------------------------------------------------------------
    #[error("hook spec parse error in {}: malformed or unparsable hooks.json", path.display())]
    HookSpecParseError { path: PathBuf },

    // `source` is auto-recognised by thiserror as the error source; we do
    // NOT use `#[from]` here — that would clash with the existing `Io`
    // variant's blanket `From<std::io::Error>`.
    #[error("hook settings write failed at {}: {source}", path.display())]
    HookSettingsWriteFailed {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("agent translation failed: {agent}")]
    AgentTranslationFailed { agent: String },

    #[error("guardrails write failed at {}", path.display())]
    GuardrailsWriteFailed { path: PathBuf },

    // -----------------------------------------------------------------------
    // Phase 2 — plugin lifecycle (codes 20–23).
    // -----------------------------------------------------------------------
    #[error(
        "plugin `{0}` is not installed under any registered catalog\nhint: list valid plugin ids with `tome plugin list`, or run `tome plugin` to browse and enable interactively"
    )]
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
    // Phase p11 / model tiering — scoped reindex under embedder change (47).
    //
    // A profile switch that changes the embedder needs a WHOLE-INDEX re-embed:
    // re-embedding only some plugins while the GLOBAL `meta` embedder rows are
    // stamped would leave out-of-scope vectors at the old dimension — the
    // mixed-dimension corruption B1 guards. 47 is the first free slot after
    // the Phase 6 hooks/agents block (43–46); the query+drift cluster (40–42)
    // is full.
    // -----------------------------------------------------------------------
    #[error(
        "embedder changed (`{stored}` -> `{configured}`); a scoped reindex \
         cannot switch the embedder safely. Run a full `tome reindex` (no \
         catalog/plugin scope) to re-embed every plugin and switch profiles."
    )]
    ReindexScopedEmbedderChange { stored: String, configured: String },

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
    // Phase 8 — authoring & conversion (codes 80–86).
    //
    // A contiguous block of seven NEW failure classes (principle II — new
    // classes get new codes, none repurposed). Earlier blocks ran out of
    // contiguous room (Phase 6 ends at 46; 50/60/70 clusters are taken), so
    // Phase 8 starts a fresh decade at 80. The two `lint` verdict codes
    // (85/86) follow the `QueryNoResultsStrict`(40) precedent — the command
    // ran successfully; the *result* is non-zero — giving CI clean pass/fail
    // semantics. See `specs/008-phase-8-authoring-conversion/contracts/exit-codes.md`.
    // -----------------------------------------------------------------------
    /// A plugin directory carries a legacy `.claude-plugin/plugin.json` but no
    /// native `tome-plugin.toml`. The Phase 8 cutover reads only the latter;
    /// this is the migration nudge (names the dir + the `convert` command).
    #[error(
        "plugin at {} is not converted: found legacy `.claude-plugin/plugin.json` but no `tome-plugin.toml`\nhint: run `tome plugin convert {}` to migrate it to the native format",
        path.display(),
        path.display()
    )]
    PluginNotConverted { path: PathBuf },

    /// `create`/`convert` would write over an existing file and `--force` was
    /// not given. Names the colliding path; `--force` overwrites only the
    /// colliding files (never a directory wipe).
    #[error(
        "refusing to overwrite existing output at {}\nhint: pass --force to overwrite the colliding file(s)",
        path.display()
    )]
    OutputExists { path: PathBuf },

    /// A `--template` resolved (built-in name or fetched source) but could not
    /// be used: missing template file, malformed minijinja, or a render error.
    #[error("template `{template}` is unusable: {reason}")]
    TemplateInvalid { template: String, reason: String },

    /// `convert` could not auto-detect the source harness/level and no
    /// `--from` override was supplied.
    #[error(
        "could not detect the source format at {}\nhint: pass --from <harness> (claude-code|codex|cursor|opencode|cline|agent-skills)",
        path.display()
    )]
    SourceFormatUnrecognized { path: PathBuf },

    /// `convert --strict` hit a feature Tome cannot represent. Nothing was
    /// written (the abort happens before any emit).
    #[error(
        "conversion aborted under --strict at an unsupported feature: {feature}\nhint: drop --strict to convert with warnings instead (nothing was written)"
    )]
    ConversionUnsupportedStrict { feature: String },

    /// `lint` found ≥1 error — the CI-fail verdict code. Verdict, not crash:
    /// the run itself succeeded (cf. `QueryNoResultsStrict`).
    #[error("validation found {errors} error(s)")]
    ValidationFoundErrors { errors: usize },

    /// `lint --strict` found warnings (and no errors) — the strict CI-fail
    /// verdict code. Without `--strict`, warnings exit 0.
    #[error("validation found {warnings} warning(s) under --strict (no errors)")]
    ValidationStrictWarnings { warnings: usize },

    // -----------------------------------------------------------------------
    // Phase 9 — meta skills (codes 87–89).
    //
    // Three NEW failure classes (principle II — new classes get new codes,
    // none repurposed), continuing the fresh decade Phase 8 opened at 80.
    // `install` failures get a dedicated code (88) rather than collapsing to
    // `Io` (7): the agent sink set the precedent of a sink-owned exit code
    // over `Io` for native-file emit (P6/P8, CON-1). See
    // `specs/009-phase-9-meta-skills/data-model.md` §4.
    // -----------------------------------------------------------------------
    /// `meta add`/`meta remove`/the MCP `meta` tool was given a skill id that
    /// is not in the embedded registry. `available` is the comma-joined list of
    /// bundled ids (FR-033 — the message enumerates them, like
    /// `SourceFormatUnrecognized` inlines its `--from` options); the producer
    /// fills it from `authoring::meta::all()`.
    #[error("no embedded meta skill with id `{id}`\navailable: {available}")]
    MetaSkillNotFound { id: String, available: String },

    /// Staging/landing/symlink-guard failure while installing a meta skill
    /// (includes an unsafe skill id or a refused symlinked target component).
    /// No write lands outside `dir`.
    #[error("failed to install meta skill `{skill_id}` into {}: {source}", dir.display())]
    MetaInstallFailed {
        skill_id: String,
        dir: PathBuf,
        source: std::io::Error,
    },

    /// `meta add`/`meta remove` ran the all-detected default but found no
    /// supported harness installed and no `--harness` was given; also the MCP
    /// fail-closed when the host harness is unknown/unstamped.
    #[error(
        "no supported harness detected\nhint: install a supported harness (claude-code, cursor, codex, opencode) or pass --harness <name>"
    )]
    NoHarnessDetected,

    // -----------------------------------------------------------------------
    // Phase 10 — telemetry (codes 90–92).
    //
    // Three NEW failure classes (principle II — new classes get new codes,
    // none repurposed), opening the 90s decade. Retained for the closed-set
    // contract after the gauge-telemetry kernel migration (Phase 13).
    // See `specs/010-phase-10-telemetry/data-model.md`.
    // -----------------------------------------------------------------------
    /// **VESTIGIAL (exit 90).** Retained for the closed-set contract.
    ///
    /// After the `gauge-telemetry` kernel migration the foreground flush
    /// (`tome telemetry flush`) routes through `handle().run_flush()`, which is
    /// best-effort and never returns an error to the caller. This variant is
    /// therefore **not constructed anywhere in `src/`** — the exit-90 path no
    /// longer exists in practice. The variant (and its `ErrorCategory` mapping)
    /// are kept so the closed error set stays monotonic: no code in the 90s
    /// decade is ever repurposed.
    ///
    /// If a future foreground-flush failure surface is added (e.g. a
    /// `gauge_telemetry::BuildError::InsecureEndpoint` check on the init path),
    /// this is the variant it should map to.
    #[error("telemetry endpoint unreachable: {endpoint}")]
    TelemetryEndpointUnreachable { endpoint: String },

    /// **VESTIGIAL (exit 91).** Retained for the closed-set contract.
    ///
    /// Originally surfaced when the standalone `~/.tome/telemetry/config.toml`
    /// was malformed. That file was removed when telemetry opt-out was folded
    /// into the unified `~/.tome/config.toml` (parse errors on that file are
    /// exit 5 / `ManifestInvalid::TomlParse`). This variant is therefore **not
    /// constructed anywhere in `src/`**.
    ///
    /// If a `gauge_telemetry::BuildError::InsecureEndpoint` (non-HTTPS endpoint
    /// supplied) is ever surfaced on a foreground telemetry path, this is the
    /// appropriate mapping: it is a resolve-time config semantic error, analogous
    /// to exit 93 (`ProviderConfigInvalid`) on the provider path.
    #[error("telemetry config invalid at {}: {detail}", path.display())]
    TelemetryConfigInvalid { path: PathBuf, detail: String },

    /// The local JSONL queue contains lines that are not parseable as telemetry
    /// events — `count` records were dropped while self-healing the file.
    ///
    /// **Actively constructed** by `tome telemetry inspect` (exit 92) when any
    /// queue line is unparsable. Distinct from `Io` (7): the bytes were readable
    /// but not valid events (e.g. a torn non-local line), which the background
    /// flusher recovers from rather than aborting. The foreground `inspect`
    /// command surfaces this to the user so they can see the damage before it is
    /// silently healed on the next flush.
    #[error("telemetry queue corrupt at {}: dropped {count} record(s)", path.display())]
    TelemetryQueueCorrupt { path: PathBuf, count: usize },

    // -----------------------------------------------------------------------
    // Phase 12 — BYOK/BYOM providers (codes 93–95).
    //
    // Three NEW failure classes (principle II — new classes get new codes,
    // none repurposed), continuing the 90s decade Phase 10 opened. They split
    // along the resolve-time / runtime / content-validation boundary:
    //
    //  * 93 `ProviderConfigInvalid` — resolve-time semantic config error: a
    //    capability references an undefined provider, a kind illegal for the
    //    capability, or a `provider` set without `model`. (A malformed config
    //    *field* stays exit 5 / `ManifestInvalid::TomlParse`.)
    //  * 94 `ProviderRequestFailed` — a remote call failed. Phase 2 maps the
    //    structured `ProviderError` (kind + retryable, credentials redacted)
    //    into the `detail` carrier; for now it is a plain string.
    //  * 95 `RemoteEmbeddingInvalid` — a remote embedding failed content
    //    validation (empty / non-finite / wrong dimension). Fail-closed: never
    //    written to the index, never used for KNN.
    //
    // See `specs/012-phase-12-byok-providers/contracts/error-and-validation.md`.
    // -----------------------------------------------------------------------
    /// A capability section references a provider that cannot be resolved into a
    /// usable connection: undefined provider name, a kind not legal for the
    /// capability, or a `provider` set without a `model`. `detail` names the
    /// offending provider/capability.
    #[error("provider config invalid: {detail}")]
    ProviderConfigInvalid { detail: String },

    /// A remote provider call failed (auth, rate-limit, timeout, unreachable,
    /// malformed response, …). `detail` is the redacted, structured
    /// `ProviderError` summary (Phase 2); credentials never reach this field.
    #[error("provider request failed: {detail}")]
    ProviderRequestFailed { detail: String },

    /// A remote embedding failed content validation. `detail` states which
    /// check failed (empty / non-finite / dimension). The vector is discarded —
    /// never stored, never used for KNN.
    #[error("remote embedding invalid: {detail}")]
    RemoteEmbeddingInvalid { detail: String },

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
            // 9 — Phase 5 / US1.d (R-M1): plugin data dir write failure.
            // Split from `WorkspaceDataDirWriteFailed` (25) so the variant
            // name + exit code carry the discriminator instead of relying
            // on the inner `path` field. Lowest free slot in Phase 1's
            // 1–8 cluster; semantically I/O-adjacent.
            Self::PluginDataDirWriteFailed { .. } => 9,
            // 12 — no workspace bound to CWD (the free slot immediately
            // below the 13–16 workspace name/binding cluster; kept in the
            // workspace family).
            Self::WorkspaceNotBound => 12,
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
            // 25–29 — Phase 5 commands-as-prompts + substitution layer.
            // Contract proposed 21/22/23 for `EntryNotFound`/
            // `SubstitutionFailed`/`InvalidArgumentFrontmatter` but those
            // collide with Phase 2's plugin lifecycle (21/22/23). Per
            // constitution principle II (NON-NEGOTIABLE), F1 reassigns to
            // 27/28/29 — same precedent as the summariser-vs-PluginNotFound
            // reassignment above.
            Self::WorkspaceDataDirWriteFailed { .. } => 25,
            Self::PromptArgumentMismatch { .. } => 26,
            Self::EntryNotFound { .. } => 27,
            Self::SubstitutionFailed { .. } => 28,
            Self::InvalidArgumentFrontmatter { .. } => 29,
            // 43–46 — Phase 6 hooks + agents. PRD-proposed 30–33 collided
            // with the model-on-disk cluster; reassigned to the first free
            // contiguous run (contracts/exit-codes-p6.md, research R-1).
            Self::HookSpecParseError { .. } => 43,
            Self::HookSettingsWriteFailed { .. } => 44,
            Self::AgentTranslationFailed { .. } => 45,
            Self::GuardrailsWriteFailed { .. } => 46,
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
            // 47 — model tiering: scoped reindex under embedder change
            Self::ReindexScopedEmbedderChange { .. } => 47,
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
            // 80–86 — Phase 8 authoring & conversion. A fresh contiguous
            // decade; the lint verdict codes (85/86) follow the
            // QueryNoResultsStrict(40) "successful run, non-zero verdict"
            // precedent.
            Self::PluginNotConverted { .. } => 80,
            Self::OutputExists { .. } => 81,
            Self::TemplateInvalid { .. } => 82,
            Self::SourceFormatUnrecognized { .. } => 83,
            Self::ConversionUnsupportedStrict { .. } => 84,
            Self::ValidationFoundErrors { .. } => 85,
            Self::ValidationStrictWarnings { .. } => 86,
            // 87–89 — Phase 9 meta skills. A dedicated install code (88) over
            // `Io` (7), mirroring the agent sink's dedicated-code precedent.
            Self::MetaSkillNotFound { .. } => 87,
            Self::MetaInstallFailed { .. } => 88,
            Self::NoHarnessDetected => 89,
            // 90–92 — Phase 10 telemetry. A fresh contiguous decade; only the
            // foreground `tome telemetry flush` ever returns these.
            Self::TelemetryEndpointUnreachable { .. } => 90,
            Self::TelemetryConfigInvalid { .. } => 91,
            Self::TelemetryQueueCorrupt { .. } => 92,
            // 93–95 — Phase 12 BYOK/BYOM providers. Resolve-time config error
            // (93), runtime remote-call failure (94), and remote-embedding
            // content-validation failure (95).
            Self::ProviderConfigInvalid { .. } => 93,
            Self::ProviderRequestFailed { .. } => 94,
            Self::RemoteEmbeddingInvalid { .. } => 95,
        }
    }

    /// Closed category for `--json` error records and MCP error-code slugs.
    /// Maps 1:1 to the closed error set (FR-022 carried forward + FR-048 +
    /// the Phase 10 telemetry block). Returning the typed [`ErrorCategory`]
    /// (rather than a bare `&'static str`) keeps the wire slug stable while
    /// giving the telemetry `error_class` field a closed enum to serialise —
    /// the compiler still enforces exhaustiveness, so a new `TomeError`
    /// variant fails to build until it is categorised here.
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::Internal(_) => ErrorCategory::Internal,
            Self::Usage(_) => ErrorCategory::Usage,
            Self::CatalogNotFound(_) => ErrorCategory::CatalogNotFound,
            Self::CatalogAlreadyExists(_) => ErrorCategory::CatalogAlreadyExists,
            Self::ManifestInvalid(_) => ErrorCategory::ManifestInvalid,
            Self::GitFailed { .. } => ErrorCategory::GitFailed,
            Self::Io(_) => ErrorCategory::Io,
            Self::Interrupted => ErrorCategory::Interrupted,
            // Phase 5 / US1.d (R-M1): plugin data dir write failure.
            Self::PluginDataDirWriteFailed { .. } => ErrorCategory::PluginDataDirWriteFailed,
            // Phase 4 — workspace name + project binding
            Self::WorkspaceNotBound => ErrorCategory::WorkspaceNotBound,
            Self::WorkspaceNotFound { .. } => ErrorCategory::WorkspaceNotFound,
            Self::WorkspaceAlreadyExists { .. } => ErrorCategory::WorkspaceAlreadyExists,
            Self::WorkspaceNameInvalid { .. } => ErrorCategory::WorkspaceNameInvalid,
            Self::WorkspaceHasBoundProjects { .. } => ErrorCategory::WorkspaceHasBoundProjects,
            // Phase 4 — harness composition + integration
            Self::CompositionError { .. } => ErrorCategory::CompositionError,
            Self::HarnessNotSupported { .. } => ErrorCategory::HarnessNotSupported,
            Self::HarnessClash { .. } => ErrorCategory::HarnessClash,
            // Phase 4 — summariser
            Self::SummariserFailure { .. } => ErrorCategory::SummariserFailure,
            // Phase 5 — commands-as-prompts + substitution layer
            Self::WorkspaceDataDirWriteFailed { .. } => ErrorCategory::WorkspaceDataDirWriteFailed,
            Self::PromptArgumentMismatch { .. } => ErrorCategory::PromptArgumentMismatch,
            Self::EntryNotFound { .. } => ErrorCategory::EntryNotFound,
            Self::SubstitutionFailed { .. } => ErrorCategory::SubstitutionFailed,
            Self::InvalidArgumentFrontmatter { .. } => ErrorCategory::InvalidArgumentFrontmatter,
            // Phase 6 — hooks + agents
            Self::HookSpecParseError { .. } => ErrorCategory::HookSpecParseError,
            Self::HookSettingsWriteFailed { .. } => ErrorCategory::HookSettingsWriteFailed,
            Self::AgentTranslationFailed { .. } => ErrorCategory::AgentTranslationFailed,
            Self::GuardrailsWriteFailed { .. } => ErrorCategory::GuardrailsWriteFailed,
            Self::PluginNotFound(_) => ErrorCategory::PluginNotFound,
            Self::PluginAlreadyInState { .. } => ErrorCategory::PluginAlreadyInState,
            Self::PluginManifestParseError { .. } => ErrorCategory::PluginManifestParseError,
            Self::SkillFrontmatterParseError { .. } => ErrorCategory::SkillFrontmatterParseError,
            Self::ModelMissing { .. } => ErrorCategory::ModelMissing,
            Self::ModelCorrupt { .. } => ErrorCategory::ModelCorrupt,
            Self::ModelChecksumMismatch { .. } => ErrorCategory::ModelChecksumMismatch,
            Self::ModelRegistrationParseError { .. } => ErrorCategory::ModelRegistrationParseError,
            Self::InferenceRuntimeInitFailure(_) => ErrorCategory::InferenceRuntimeInitFailure,
            Self::VectorExtensionInitFailure(_) => ErrorCategory::VectorExtensionInitFailure,
            Self::EmbeddingGenerationFailure { .. } => ErrorCategory::EmbeddingGenerationFailure,
            Self::RerankingFailure(_) => ErrorCategory::RerankingFailure,
            Self::QueryNoResultsStrict { .. } => ErrorCategory::QueryNoResultsStrict,
            Self::EmbedderNameDrift { .. } => ErrorCategory::EmbedderNameDrift,
            Self::EmbedderVersionDrift { .. } => ErrorCategory::EmbedderVersionDrift,
            Self::ReindexScopedEmbedderChange { .. } => ErrorCategory::ReindexScopedEmbedderChange,
            Self::IndexBusy => ErrorCategory::IndexBusy,
            Self::IndexIntegrityCheckFailure(_) => ErrorCategory::IndexIntegrityCheckFailure,
            Self::SchemaTooNew { .. } => ErrorCategory::SchemaTooNew,
            Self::CatalogHasEnabledPlugins { .. } => ErrorCategory::CatalogHasEnabledPlugins,
            Self::NotATerminal => ErrorCategory::NotATerminal,
            Self::McpStartupFailed { .. } => ErrorCategory::McpStartup,
            Self::McpProtocolIo { .. } => ErrorCategory::McpIo,
            Self::WorkspaceMalformed { .. } => ErrorCategory::WorkspaceMalformed,
            // NOTE: shares the `schema_too_new` slug with `SchemaTooNew` above —
            // byte-identical to today's output; preserved deliberately.
            Self::SchemaVersionTooNew { .. } => ErrorCategory::SchemaTooNew,
            Self::SchemaMigrationFailed { .. } => ErrorCategory::SchemaMigration,
            Self::DoctorFixNotSafe { .. } => ErrorCategory::DoctorFixUnsafe,
            // Phase 8 — authoring & conversion
            Self::PluginNotConverted { .. } => ErrorCategory::PluginNotConverted,
            Self::OutputExists { .. } => ErrorCategory::OutputExists,
            Self::TemplateInvalid { .. } => ErrorCategory::TemplateInvalid,
            Self::SourceFormatUnrecognized { .. } => ErrorCategory::SourceFormatUnrecognized,
            Self::ConversionUnsupportedStrict { .. } => ErrorCategory::ConversionUnsupportedStrict,
            Self::ValidationFoundErrors { .. } => ErrorCategory::ValidationFoundErrors,
            Self::ValidationStrictWarnings { .. } => ErrorCategory::ValidationStrictWarnings,
            // Phase 9 — meta skills
            Self::MetaSkillNotFound { .. } => ErrorCategory::MetaSkillNotFound,
            Self::MetaInstallFailed { .. } => ErrorCategory::MetaInstallFailed,
            Self::NoHarnessDetected => ErrorCategory::NoHarnessDetected,
            // Phase 10 — telemetry
            Self::TelemetryEndpointUnreachable { .. } => {
                ErrorCategory::TelemetryEndpointUnreachable
            }
            Self::TelemetryConfigInvalid { .. } => ErrorCategory::TelemetryConfigInvalid,
            Self::TelemetryQueueCorrupt { .. } => ErrorCategory::TelemetryQueueCorrupt,
            // Phase 12 — BYOK/BYOM providers
            Self::ProviderConfigInvalid { .. } => ErrorCategory::ProviderConfigInvalid,
            Self::ProviderRequestFailed { .. } => ErrorCategory::ProviderRequestFailed,
            Self::RemoteEmbeddingInvalid { .. } => ErrorCategory::RemoteEmbeddingInvalid,
        }
    }
}

/// Closed, wire-stable category for the `--json` error envelope and MCP
/// error-code slugs. One variant per `TomeError` failure class; [`Self::as_str`]
/// returns the byte-identical snake_case slug the error set has always emitted.
///
/// This is an emit-only `Serialize` record (the telemetry `error_class` field
/// plus the existing JSON/MCP surfaces) — there is no `deny_unknown_fields`,
/// which is reserved for *inputs*. Both the `SchemaTooNew` and
/// `SchemaVersionTooNew` error classes deliberately map onto the single
/// `SchemaTooNew` category, preserving the slug overlap that predates this
/// refactor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    Internal,
    Usage,
    CatalogNotFound,
    CatalogAlreadyExists,
    ManifestInvalid,
    GitFailed,
    Io,
    Interrupted,
    PluginDataDirWriteFailed,
    WorkspaceNotBound,
    WorkspaceNotFound,
    WorkspaceAlreadyExists,
    WorkspaceNameInvalid,
    WorkspaceHasBoundProjects,
    CompositionError,
    HarnessNotSupported,
    HarnessClash,
    SummariserFailure,
    WorkspaceDataDirWriteFailed,
    PromptArgumentMismatch,
    EntryNotFound,
    SubstitutionFailed,
    InvalidArgumentFrontmatter,
    HookSpecParseError,
    HookSettingsWriteFailed,
    AgentTranslationFailed,
    GuardrailsWriteFailed,
    PluginNotFound,
    PluginAlreadyInState,
    PluginManifestParseError,
    SkillFrontmatterParseError,
    ModelMissing,
    ModelCorrupt,
    ModelChecksumMismatch,
    ModelRegistrationParseError,
    InferenceRuntimeInitFailure,
    VectorExtensionInitFailure,
    EmbeddingGenerationFailure,
    RerankingFailure,
    QueryNoResultsStrict,
    EmbedderNameDrift,
    EmbedderVersionDrift,
    ReindexScopedEmbedderChange,
    IndexBusy,
    IndexIntegrityCheckFailure,
    SchemaTooNew,
    CatalogHasEnabledPlugins,
    NotATerminal,
    McpStartup,
    McpIo,
    WorkspaceMalformed,
    SchemaMigration,
    DoctorFixUnsafe,
    PluginNotConverted,
    OutputExists,
    TemplateInvalid,
    SourceFormatUnrecognized,
    ConversionUnsupportedStrict,
    ValidationFoundErrors,
    ValidationStrictWarnings,
    MetaSkillNotFound,
    MetaInstallFailed,
    NoHarnessDetected,
    TelemetryEndpointUnreachable,
    TelemetryConfigInvalid,
    TelemetryQueueCorrupt,
    ProviderConfigInvalid,
    ProviderRequestFailed,
    RemoteEmbeddingInvalid,
}

impl ErrorCategory {
    /// The byte-stable snake_case slug. Wire-facing — changing any string here
    /// breaks the `--json` error envelope and MCP error-code contracts.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Internal => "internal",
            Self::Usage => "usage",
            Self::CatalogNotFound => "catalog_not_found",
            Self::CatalogAlreadyExists => "catalog_already_exists",
            Self::ManifestInvalid => "manifest_invalid",
            Self::GitFailed => "git_failed",
            Self::Io => "io",
            Self::Interrupted => "interrupted",
            Self::PluginDataDirWriteFailed => "plugin_data_dir_write_failed",
            Self::WorkspaceNotBound => "workspace_not_bound",
            Self::WorkspaceNotFound => "workspace_not_found",
            Self::WorkspaceAlreadyExists => "workspace_already_exists",
            Self::WorkspaceNameInvalid => "workspace_name_invalid",
            Self::WorkspaceHasBoundProjects => "workspace_has_bound_projects",
            Self::CompositionError => "composition_error",
            Self::HarnessNotSupported => "harness_not_supported",
            Self::HarnessClash => "harness_clash",
            Self::SummariserFailure => "summariser_failure",
            Self::WorkspaceDataDirWriteFailed => "workspace_data_dir_write_failed",
            Self::PromptArgumentMismatch => "prompt_argument_mismatch",
            Self::EntryNotFound => "entry_not_found",
            Self::SubstitutionFailed => "substitution_failed",
            Self::InvalidArgumentFrontmatter => "invalid_argument_frontmatter",
            Self::HookSpecParseError => "hook_spec_parse_error",
            Self::HookSettingsWriteFailed => "hook_settings_write_failed",
            Self::AgentTranslationFailed => "agent_translation_failed",
            Self::GuardrailsWriteFailed => "guardrails_write_failed",
            Self::PluginNotFound => "plugin_not_found",
            Self::PluginAlreadyInState => "plugin_already_in_state",
            Self::PluginManifestParseError => "plugin_manifest_parse_error",
            Self::SkillFrontmatterParseError => "skill_frontmatter_parse_error",
            Self::ModelMissing => "model_missing",
            Self::ModelCorrupt => "model_corrupt",
            Self::ModelChecksumMismatch => "model_checksum_mismatch",
            Self::ModelRegistrationParseError => "model_registration_parse_error",
            Self::InferenceRuntimeInitFailure => "inference_runtime_init_failure",
            Self::VectorExtensionInitFailure => "vector_extension_init_failure",
            Self::EmbeddingGenerationFailure => "embedding_generation_failure",
            Self::RerankingFailure => "reranking_failure",
            Self::QueryNoResultsStrict => "query_no_results_strict",
            Self::EmbedderNameDrift => "embedder_name_drift",
            Self::EmbedderVersionDrift => "embedder_version_drift",
            Self::ReindexScopedEmbedderChange => "reindex_scoped_embedder_change",
            Self::IndexBusy => "index_busy",
            Self::IndexIntegrityCheckFailure => "index_integrity_check_failure",
            Self::SchemaTooNew => "schema_too_new",
            Self::CatalogHasEnabledPlugins => "catalog_has_enabled_plugins",
            Self::NotATerminal => "not_a_terminal",
            Self::McpStartup => "mcp_startup",
            Self::McpIo => "mcp_io",
            Self::WorkspaceMalformed => "workspace_malformed",
            Self::SchemaMigration => "schema_migration",
            Self::DoctorFixUnsafe => "doctor_fix_unsafe",
            Self::PluginNotConverted => "plugin_not_converted",
            Self::OutputExists => "output_exists",
            Self::TemplateInvalid => "template_invalid",
            Self::SourceFormatUnrecognized => "source_format_unrecognized",
            Self::ConversionUnsupportedStrict => "conversion_unsupported_strict",
            Self::ValidationFoundErrors => "validation_found_errors",
            Self::ValidationStrictWarnings => "validation_strict_warnings",
            Self::MetaSkillNotFound => "meta_skill_not_found",
            Self::MetaInstallFailed => "meta_install_failed",
            Self::NoHarnessDetected => "no_harness_detected",
            Self::TelemetryEndpointUnreachable => "telemetry_endpoint_unreachable",
            Self::TelemetryConfigInvalid => "telemetry_config_invalid",
            Self::TelemetryQueueCorrupt => "telemetry_queue_corrupt",
            Self::ProviderConfigInvalid => "provider_config_invalid",
            Self::ProviderRequestFailed => "provider_request_failed",
            Self::RemoteEmbeddingInvalid => "remote_embedding_invalid",
        }
    }

    /// Whether retrying the same operation, unchanged, could plausibly succeed.
    ///
    /// `true` only for **transient or contended** failure classes — another
    /// process holds a lock, a settings file is in a clashing state that clears
    /// once resolved, or a network/remote call failed for a reason that may not
    /// recur (rate-limit, timeout, unreachable). Everything deterministic
    /// (a malformed manifest, an unknown catalog, a strict-mode verdict) is
    /// `false`: retrying it verbatim reproduces the same error.
    ///
    /// #296: this replaces the "regex the English message" contract — callers
    /// (agents in other harnesses) branch on this machine-readable flag on both
    /// the CLI `--json` error envelope and the MCP tool error `data` payload.
    /// Wire-facing: exhaustive `match` (no wildcard) so a new [`ErrorCategory`]
    /// variant fails to compile until its retry semantics are decided here.
    pub fn retryable(&self) -> bool {
        match self {
            // Contended: another `tome` process holds the index lock. The
            // canonical retry case (`IndexBusy` — "retry once it has finished").
            Self::IndexBusy => true,
            // Contended: a harness MCP config clash. This is the sole member of
            // the `retryable && remediation.is_some()` quadrant — BOTH signals
            // are set deliberately, and they are SEQUENCED: `retryable` here
            // means "retry AFTER applying the `remediation`", not "retry the
            // identical command". A naive agent that honours `retryable` by
            // re-running the same command verbatim will loop forever, because the
            // clash clears only once the `--force` remediation overwrites the
            // clashing entry. Honour `remediation` first, then retry.
            Self::HarnessClash => true,
            // Network / remote: a git fetch, a BYOK/BYOM provider call, or the
            // telemetry endpoint failed for a reason (rate-limit, timeout,
            // transient unreachability) that may not recur on the next attempt.
            Self::GitFailed
            | Self::ProviderRequestFailed
            | Self::TelemetryEndpointUnreachable
            | Self::McpIo => true,

            // Everything else is deterministic — retrying verbatim reproduces
            // the same failure. Enumerated (no wildcard) so a future variant is
            // a compile error until its retry semantics are classified above.
            Self::Internal
            | Self::Usage
            | Self::CatalogNotFound
            | Self::CatalogAlreadyExists
            | Self::ManifestInvalid
            | Self::Io
            | Self::Interrupted
            | Self::PluginDataDirWriteFailed
            | Self::WorkspaceNotBound
            | Self::WorkspaceNotFound
            | Self::WorkspaceAlreadyExists
            | Self::WorkspaceNameInvalid
            | Self::WorkspaceHasBoundProjects
            | Self::CompositionError
            | Self::HarnessNotSupported
            | Self::SummariserFailure
            | Self::WorkspaceDataDirWriteFailed
            | Self::PromptArgumentMismatch
            | Self::EntryNotFound
            | Self::SubstitutionFailed
            | Self::InvalidArgumentFrontmatter
            | Self::HookSpecParseError
            | Self::HookSettingsWriteFailed
            | Self::AgentTranslationFailed
            | Self::GuardrailsWriteFailed
            | Self::PluginNotFound
            | Self::PluginAlreadyInState
            | Self::PluginManifestParseError
            | Self::SkillFrontmatterParseError
            | Self::ModelMissing
            | Self::ModelCorrupt
            | Self::ModelChecksumMismatch
            | Self::ModelRegistrationParseError
            | Self::InferenceRuntimeInitFailure
            | Self::VectorExtensionInitFailure
            | Self::EmbeddingGenerationFailure
            | Self::RerankingFailure
            | Self::QueryNoResultsStrict
            | Self::EmbedderNameDrift
            | Self::EmbedderVersionDrift
            | Self::ReindexScopedEmbedderChange
            | Self::IndexIntegrityCheckFailure
            | Self::SchemaTooNew
            | Self::CatalogHasEnabledPlugins
            | Self::NotATerminal
            | Self::McpStartup
            | Self::WorkspaceMalformed
            | Self::SchemaMigration
            | Self::DoctorFixUnsafe
            | Self::PluginNotConverted
            | Self::OutputExists
            | Self::TemplateInvalid
            | Self::SourceFormatUnrecognized
            | Self::ConversionUnsupportedStrict
            | Self::ValidationFoundErrors
            | Self::ValidationStrictWarnings
            | Self::MetaSkillNotFound
            | Self::MetaInstallFailed
            | Self::NoHarnessDetected
            | Self::TelemetryConfigInvalid
            | Self::TelemetryQueueCorrupt
            | Self::ProviderConfigInvalid
            | Self::RemoteEmbeddingInvalid => false,
        }
    }

    /// The coarse, category-level `tome` command that fixes this failure class,
    /// if a single one exists — `None` otherwise.
    ///
    /// #296: a **machine-readable** command hint so callers no longer regex the
    /// fix out of the English `Display` string. Deliberately coarse: it names
    /// the command family (`tome reindex --force`, `tome plugin convert`), while
    /// the instance-specific detail (the exact path, model name, or env var)
    /// stays in the human `message`. NEVER embed a credential or any
    /// instance-specific secret here — every value is a `&'static str` literal,
    /// so by construction nothing dynamic (and thus nothing credential-shaped)
    /// can reach this field.
    ///
    /// Wire-facing on both the CLI `--json` envelope and the MCP `data` payload;
    /// exhaustive `match` (no wildcard) so a new variant must decide its hint.
    pub fn remediation(&self) -> Option<&'static str> {
        match self {
            // Embedder drift / corrupt or inconsistent index → rebuild it.
            Self::EmbedderNameDrift
            | Self::EmbedderVersionDrift
            | Self::ReindexScopedEmbedderChange
            | Self::IndexIntegrityCheckFailure => Some("tome reindex --force"),

            // A model is absent → download it; corrupt/checksum-mismatch → force
            // a re-download.
            Self::ModelMissing => Some("tome models download"),
            Self::ModelCorrupt | Self::ModelChecksumMismatch => {
                Some("tome models download --force")
            }

            // A legacy plugin needs the native-format cutover.
            Self::PluginNotConverted => Some("tome plugin convert"),

            // A harness MCP config clash clears once the clashing entry is
            // overwritten with the expected shape. `HarnessClash` is reachable
            // from several commands (`harness use`/`harness sync`/`workspace
            // use`/`plugin enable --sync`/`doctor --fix`); this is ONE
            // representative coarse class-fix — the human `message` names the
            // command(s) applicable to the surface the clash actually came from.
            // This is the only category where both `retryable` and `remediation`
            // are set (see the sequencing note on the `retryable()` arm).
            Self::HarnessClash => Some("tome harness use --force"),

            // No workspace named / bound → create or select one.
            Self::WorkspaceNotFound => Some("tome workspace init"),
            Self::WorkspaceNotBound => Some("tome workspace use"),

            // A summariser fault (missing/corrupt model, backend init) is what
            // `doctor --fix` re-provisions.
            Self::SummariserFailure => Some("tome doctor --fix"),

            // No single command fixes the rest — the human `message` carries the
            // specifics (or there is nothing to "fix", e.g. a strict-mode
            // verdict or a usage error). Enumerated (no wildcard) so a future
            // variant must decide its hint rather than silently defaulting.
            Self::Internal
            | Self::Usage
            | Self::CatalogNotFound
            | Self::CatalogAlreadyExists
            | Self::ManifestInvalid
            | Self::GitFailed
            | Self::Io
            | Self::Interrupted
            | Self::PluginDataDirWriteFailed
            | Self::WorkspaceAlreadyExists
            | Self::WorkspaceNameInvalid
            | Self::WorkspaceHasBoundProjects
            | Self::CompositionError
            | Self::HarnessNotSupported
            | Self::WorkspaceDataDirWriteFailed
            | Self::PromptArgumentMismatch
            | Self::EntryNotFound
            | Self::SubstitutionFailed
            | Self::InvalidArgumentFrontmatter
            | Self::HookSpecParseError
            | Self::HookSettingsWriteFailed
            | Self::AgentTranslationFailed
            | Self::GuardrailsWriteFailed
            | Self::PluginNotFound
            | Self::PluginAlreadyInState
            | Self::PluginManifestParseError
            | Self::SkillFrontmatterParseError
            | Self::ModelRegistrationParseError
            | Self::InferenceRuntimeInitFailure
            | Self::VectorExtensionInitFailure
            | Self::EmbeddingGenerationFailure
            | Self::RerankingFailure
            | Self::QueryNoResultsStrict
            | Self::IndexBusy
            | Self::SchemaTooNew
            | Self::CatalogHasEnabledPlugins
            | Self::NotATerminal
            | Self::McpStartup
            | Self::McpIo
            | Self::WorkspaceMalformed
            | Self::SchemaMigration
            | Self::DoctorFixUnsafe
            | Self::OutputExists
            | Self::TemplateInvalid
            | Self::SourceFormatUnrecognized
            | Self::ConversionUnsupportedStrict
            | Self::ValidationFoundErrors
            | Self::ValidationStrictWarnings
            | Self::MetaSkillNotFound
            | Self::MetaInstallFailed
            | Self::NoHarnessDetected
            | Self::TelemetryEndpointUnreachable
            | Self::TelemetryConfigInvalid
            | Self::TelemetryQueueCorrupt
            | Self::ProviderConfigInvalid
            | Self::ProviderRequestFailed
            | Self::RemoteEmbeddingInvalid => None,
        }
    }
}

impl std::fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl serde::Serialize for ErrorCategory {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
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

/// Sub-classification for `TomeError::SummariserFailure` (exit 24). The
/// Phase 4 design contracts originally proposed exit 20 but exit 20 is
/// already taken by Phase 2's `PluginNotFound`; the closed-set
/// implementation routes summariser failures to exit 24. Contract docs
/// were corrected in US4.d-1 (PR #74). Pre-allocated by F3; consumers wire it in
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

#[cfg(test)]
mod tests {
    use super::*;

    /// #436: structural invariants of the [`EXIT_CODES`] reference table —
    /// codes strictly ascending (⇒ unique ⇒ unique `(code, category)` pairs),
    /// the two special non-error rows present with their special categories,
    /// and non-empty meanings throughout. Coverage against every
    /// `TomeError::exit_code()` value and the docs page lives in
    /// `tests/index_query_misc/exit_codes.rs` beside the existing exhaustive
    /// variant enumeration.
    #[test]
    fn exit_codes_table_is_sorted_unique_and_carries_the_special_rows() {
        assert!(
            EXIT_CODES.windows(2).all(|w| w[0].code < w[1].code),
            "EXIT_CODES must be strictly ascending by code",
        );
        for row in EXIT_CODES {
            assert!(
                !row.meaning.is_empty(),
                "code {} has an empty meaning",
                row.code
            );
        }
        let lookup = |code: i32| EXIT_CODES.iter().find(|r| r.code == code);
        // Success has no category; the health verdict carries the non-error
        // `health_degraded` label and matches the constant.
        assert!(matches!(lookup(0), Some(r) if r.category.is_none()));
        assert!(
            matches!(lookup(EXIT_HEALTH_DEGRADED), Some(r) if r.category == Some("health_degraded")),
        );
        // Reserved-but-unused codes stay absent.
        assert!(lookup(71).is_none() && lookup(72).is_none());
    }

    #[test]
    fn workspace_not_found_hints_at_init() {
        let e = TomeError::WorkspaceNotFound {
            name: "myws".into(),
        };
        let msg = e.to_string();
        assert!(
            msg.contains("hint: create it with `tome workspace init myws`"),
            "{msg}"
        );
    }

    /// T-L1 — exhaustive `ErrorCategory::as_str()` wire-token table.
    ///
    /// Every `ErrorCategory` variant is paired with its EXACT snake_case slug in
    /// a hand-frozen table, and the table is asserted against `as_str()`. The
    /// `match category` block below is `#[deny]`-non-exhaustive by construction:
    /// adding a variant without extending this table fails to compile, and
    /// renaming a slug in `as_str()` fails the assertion. Either way a drift in a
    /// telemetry `error_class` slug (the `tome.error` / `tome.model_download`
    /// `error_class` field, the `--json` envelope, and the MCP error-code
    /// contract all read this) breaks CI here rather than silently re-keying the
    /// collector.
    #[test]
    fn error_category_wire_tokens_are_pinned_and_exhaustive() {
        use ErrorCategory::*;

        // The frozen `(variant, slug)` table. Each entry's slug is a LITERAL —
        // it is NOT recomputed from `as_str()`.
        let table: &[(ErrorCategory, &str)] = &[
            (Internal, "internal"),
            (Usage, "usage"),
            (CatalogNotFound, "catalog_not_found"),
            (CatalogAlreadyExists, "catalog_already_exists"),
            (ManifestInvalid, "manifest_invalid"),
            (GitFailed, "git_failed"),
            (Io, "io"),
            (Interrupted, "interrupted"),
            (PluginDataDirWriteFailed, "plugin_data_dir_write_failed"),
            (WorkspaceNotBound, "workspace_not_bound"),
            (WorkspaceNotFound, "workspace_not_found"),
            (WorkspaceAlreadyExists, "workspace_already_exists"),
            (WorkspaceNameInvalid, "workspace_name_invalid"),
            (WorkspaceHasBoundProjects, "workspace_has_bound_projects"),
            (CompositionError, "composition_error"),
            (HarnessNotSupported, "harness_not_supported"),
            (HarnessClash, "harness_clash"),
            (SummariserFailure, "summariser_failure"),
            (
                WorkspaceDataDirWriteFailed,
                "workspace_data_dir_write_failed",
            ),
            (PromptArgumentMismatch, "prompt_argument_mismatch"),
            (EntryNotFound, "entry_not_found"),
            (SubstitutionFailed, "substitution_failed"),
            (InvalidArgumentFrontmatter, "invalid_argument_frontmatter"),
            (HookSpecParseError, "hook_spec_parse_error"),
            (HookSettingsWriteFailed, "hook_settings_write_failed"),
            (AgentTranslationFailed, "agent_translation_failed"),
            (GuardrailsWriteFailed, "guardrails_write_failed"),
            (PluginNotFound, "plugin_not_found"),
            (PluginAlreadyInState, "plugin_already_in_state"),
            (PluginManifestParseError, "plugin_manifest_parse_error"),
            (SkillFrontmatterParseError, "skill_frontmatter_parse_error"),
            (ModelMissing, "model_missing"),
            (ModelCorrupt, "model_corrupt"),
            (ModelChecksumMismatch, "model_checksum_mismatch"),
            (
                ModelRegistrationParseError,
                "model_registration_parse_error",
            ),
            (
                InferenceRuntimeInitFailure,
                "inference_runtime_init_failure",
            ),
            (VectorExtensionInitFailure, "vector_extension_init_failure"),
            (EmbeddingGenerationFailure, "embedding_generation_failure"),
            (RerankingFailure, "reranking_failure"),
            (QueryNoResultsStrict, "query_no_results_strict"),
            (EmbedderNameDrift, "embedder_name_drift"),
            (EmbedderVersionDrift, "embedder_version_drift"),
            (
                ReindexScopedEmbedderChange,
                "reindex_scoped_embedder_change",
            ),
            (IndexBusy, "index_busy"),
            (IndexIntegrityCheckFailure, "index_integrity_check_failure"),
            (SchemaTooNew, "schema_too_new"),
            (CatalogHasEnabledPlugins, "catalog_has_enabled_plugins"),
            (NotATerminal, "not_a_terminal"),
            (McpStartup, "mcp_startup"),
            (McpIo, "mcp_io"),
            (WorkspaceMalformed, "workspace_malformed"),
            (SchemaMigration, "schema_migration"),
            (DoctorFixUnsafe, "doctor_fix_unsafe"),
            (PluginNotConverted, "plugin_not_converted"),
            (OutputExists, "output_exists"),
            (TemplateInvalid, "template_invalid"),
            (SourceFormatUnrecognized, "source_format_unrecognized"),
            (ConversionUnsupportedStrict, "conversion_unsupported_strict"),
            (ValidationFoundErrors, "validation_found_errors"),
            (ValidationStrictWarnings, "validation_strict_warnings"),
            (MetaSkillNotFound, "meta_skill_not_found"),
            (MetaInstallFailed, "meta_install_failed"),
            (NoHarnessDetected, "no_harness_detected"),
            (
                TelemetryEndpointUnreachable,
                "telemetry_endpoint_unreachable",
            ),
            (TelemetryConfigInvalid, "telemetry_config_invalid"),
            (TelemetryQueueCorrupt, "telemetry_queue_corrupt"),
            (ProviderConfigInvalid, "provider_config_invalid"),
            (ProviderRequestFailed, "provider_request_failed"),
            (RemoteEmbeddingInvalid, "remote_embedding_invalid"),
        ];

        // Each table row's slug matches `as_str()`, and Serialize agrees.
        for (variant, slug) in table {
            assert_eq!(
                variant.as_str(),
                *slug,
                "ErrorCategory::{variant:?} wire token drifted",
            );
            assert_eq!(
                serde_json::to_string(variant).unwrap(),
                format!("\"{slug}\""),
                "ErrorCategory::{variant:?} Serialize must match as_str()",
            );
        }

        // EXHAUSTIVENESS: this `match` has one arm per variant and NO wildcard,
        // so the compiler rejects any future variant that is not added to the
        // table above. Each arm asserts the table contains exactly that variant
        // mapped to the same slug `as_str()` returns, closing the loop in both
        // directions (table ⊇ variants here; variants ⊇ table by the loop above).
        let lookup = |needle: ErrorCategory| -> Option<&'static str> {
            table
                .iter()
                .find(|(v, _)| *v == needle)
                .map(|(_, slug)| *slug)
        };
        let assert_covered = |variant: ErrorCategory| {
            assert_eq!(
                lookup(variant),
                Some(variant.as_str()),
                "ErrorCategory::{variant:?} is missing from the wire-token table",
            );
        };
        // The non-exhaustive `match` is the compile-time guard. Every variant
        // funnels through `assert_covered`, which proves the table carries it.
        let probe = |c: ErrorCategory| match c {
            Internal => assert_covered(Internal),
            Usage => assert_covered(Usage),
            CatalogNotFound => assert_covered(CatalogNotFound),
            CatalogAlreadyExists => assert_covered(CatalogAlreadyExists),
            ManifestInvalid => assert_covered(ManifestInvalid),
            GitFailed => assert_covered(GitFailed),
            Io => assert_covered(Io),
            Interrupted => assert_covered(Interrupted),
            PluginDataDirWriteFailed => assert_covered(PluginDataDirWriteFailed),
            WorkspaceNotBound => assert_covered(WorkspaceNotBound),
            WorkspaceNotFound => assert_covered(WorkspaceNotFound),
            WorkspaceAlreadyExists => assert_covered(WorkspaceAlreadyExists),
            WorkspaceNameInvalid => assert_covered(WorkspaceNameInvalid),
            WorkspaceHasBoundProjects => assert_covered(WorkspaceHasBoundProjects),
            CompositionError => assert_covered(CompositionError),
            HarnessNotSupported => assert_covered(HarnessNotSupported),
            HarnessClash => assert_covered(HarnessClash),
            SummariserFailure => assert_covered(SummariserFailure),
            WorkspaceDataDirWriteFailed => assert_covered(WorkspaceDataDirWriteFailed),
            PromptArgumentMismatch => assert_covered(PromptArgumentMismatch),
            EntryNotFound => assert_covered(EntryNotFound),
            SubstitutionFailed => assert_covered(SubstitutionFailed),
            InvalidArgumentFrontmatter => assert_covered(InvalidArgumentFrontmatter),
            HookSpecParseError => assert_covered(HookSpecParseError),
            HookSettingsWriteFailed => assert_covered(HookSettingsWriteFailed),
            AgentTranslationFailed => assert_covered(AgentTranslationFailed),
            GuardrailsWriteFailed => assert_covered(GuardrailsWriteFailed),
            PluginNotFound => assert_covered(PluginNotFound),
            PluginAlreadyInState => assert_covered(PluginAlreadyInState),
            PluginManifestParseError => assert_covered(PluginManifestParseError),
            SkillFrontmatterParseError => assert_covered(SkillFrontmatterParseError),
            ModelMissing => assert_covered(ModelMissing),
            ModelCorrupt => assert_covered(ModelCorrupt),
            ModelChecksumMismatch => assert_covered(ModelChecksumMismatch),
            ModelRegistrationParseError => assert_covered(ModelRegistrationParseError),
            InferenceRuntimeInitFailure => assert_covered(InferenceRuntimeInitFailure),
            VectorExtensionInitFailure => assert_covered(VectorExtensionInitFailure),
            EmbeddingGenerationFailure => assert_covered(EmbeddingGenerationFailure),
            RerankingFailure => assert_covered(RerankingFailure),
            QueryNoResultsStrict => assert_covered(QueryNoResultsStrict),
            EmbedderNameDrift => assert_covered(EmbedderNameDrift),
            EmbedderVersionDrift => assert_covered(EmbedderVersionDrift),
            ReindexScopedEmbedderChange => assert_covered(ReindexScopedEmbedderChange),
            IndexBusy => assert_covered(IndexBusy),
            IndexIntegrityCheckFailure => assert_covered(IndexIntegrityCheckFailure),
            SchemaTooNew => assert_covered(SchemaTooNew),
            CatalogHasEnabledPlugins => assert_covered(CatalogHasEnabledPlugins),
            NotATerminal => assert_covered(NotATerminal),
            McpStartup => assert_covered(McpStartup),
            McpIo => assert_covered(McpIo),
            WorkspaceMalformed => assert_covered(WorkspaceMalformed),
            SchemaMigration => assert_covered(SchemaMigration),
            DoctorFixUnsafe => assert_covered(DoctorFixUnsafe),
            PluginNotConverted => assert_covered(PluginNotConverted),
            OutputExists => assert_covered(OutputExists),
            TemplateInvalid => assert_covered(TemplateInvalid),
            SourceFormatUnrecognized => assert_covered(SourceFormatUnrecognized),
            ConversionUnsupportedStrict => assert_covered(ConversionUnsupportedStrict),
            ValidationFoundErrors => assert_covered(ValidationFoundErrors),
            ValidationStrictWarnings => assert_covered(ValidationStrictWarnings),
            MetaSkillNotFound => assert_covered(MetaSkillNotFound),
            MetaInstallFailed => assert_covered(MetaInstallFailed),
            NoHarnessDetected => assert_covered(NoHarnessDetected),
            TelemetryEndpointUnreachable => assert_covered(TelemetryEndpointUnreachable),
            TelemetryConfigInvalid => assert_covered(TelemetryConfigInvalid),
            TelemetryQueueCorrupt => assert_covered(TelemetryQueueCorrupt),
            ProviderConfigInvalid => assert_covered(ProviderConfigInvalid),
            ProviderRequestFailed => assert_covered(ProviderRequestFailed),
            RemoteEmbeddingInvalid => assert_covered(RemoteEmbeddingInvalid),
        };
        for (variant, _) in table {
            probe(*variant);
        }
    }

    // ---- #296: structured retryable / remediation accessors ----------------

    /// Representative retryable case: `IndexBusy` (another process holds the
    /// index lock) → `retryable: true`. Its message ("retry once it has
    /// finished") is now backed by structured data.
    #[test]
    fn index_busy_is_retryable() {
        assert!(TomeError::IndexBusy.category().retryable());
        // And its coarse category has no single fix command — the retry IS the
        // remedy — so `remediation` is absent.
        assert_eq!(TomeError::IndexBusy.category().remediation(), None);
    }

    /// The issue explicitly calls out `HarnessClash` needing a machine-readable
    /// retryable flag.
    #[test]
    fn harness_clash_is_retryable_with_force_remediation() {
        let e = TomeError::HarnessClash {
            path: PathBuf::from("/x/.mcp.json"),
            command: "tome".into(),
            first_arg: "mcp".into(),
        };
        assert!(e.category().retryable());
        assert_eq!(e.category().remediation(), Some("tome harness use --force"));
    }

    /// Representative remediation case: embedder drift → the fix that used to
    /// live only in the prose (`Run `tome reindex --force``) now rides the
    /// structured `remediation` field. Drift is NOT retryable (retrying the
    /// same stale query reproduces the drift).
    #[test]
    fn embedder_drift_remediation_is_reindex_force() {
        let name = TomeError::EmbedderNameDrift {
            stored: "a".into(),
            configured: "b".into(),
        };
        let version = TomeError::EmbedderVersionDrift {
            stored: "1".into(),
            configured: "2".into(),
        };
        for e in [&name, &version] {
            assert!(!e.category().retryable(), "drift is deterministic");
            assert_eq!(
                e.category().remediation(),
                Some("tome reindex --force"),
                "drift remediation must be the reindex command",
            );
        }
    }

    /// A non-retryable / no-remediation case: a usage error is deterministic and
    /// has no single fix command.
    #[test]
    fn usage_is_not_retryable_and_has_no_remediation() {
        let e = TomeError::Usage("bad flag".into());
        assert!(!e.category().retryable());
        assert_eq!(e.category().remediation(), None);
    }

    /// The `PluginNotConverted` remediation names the cutover command (coarse —
    /// the exact path stays in the message).
    #[test]
    fn plugin_not_converted_remediation_is_convert() {
        let e = TomeError::PluginNotConverted {
            path: PathBuf::from("/x"),
        };
        assert_eq!(e.category().remediation(), Some("tome plugin convert"));
        assert!(!e.category().retryable());
    }

    /// Hidden compile-time guard: an EXHAUSTIVE `match` over every category with
    /// NO wildcard, invoking both accessors. A future `ErrorCategory` variant
    /// fails to compile here (and in the accessor `match`es themselves) until its
    /// retry semantics + remediation are decided — the same guard the wire-token
    /// table gives `as_str()`. It also asserts the two invariants that keep the
    /// field meaningful: (a) a `None` remediation with `retryable == false` is a
    /// dead-end error class (no assertion — just that both accessors are total),
    /// and (b) no remediation string can carry a credential (every value is a
    /// static literal that starts with `tome ` — nothing dynamic reaches it).
    #[test]
    fn retryable_and_remediation_are_exhaustive_and_safe() {
        use ErrorCategory::*;
        let all = [
            Internal,
            Usage,
            CatalogNotFound,
            CatalogAlreadyExists,
            ManifestInvalid,
            GitFailed,
            Io,
            Interrupted,
            PluginDataDirWriteFailed,
            WorkspaceNotBound,
            WorkspaceNotFound,
            WorkspaceAlreadyExists,
            WorkspaceNameInvalid,
            WorkspaceHasBoundProjects,
            CompositionError,
            HarnessNotSupported,
            HarnessClash,
            SummariserFailure,
            WorkspaceDataDirWriteFailed,
            PromptArgumentMismatch,
            EntryNotFound,
            SubstitutionFailed,
            InvalidArgumentFrontmatter,
            HookSpecParseError,
            HookSettingsWriteFailed,
            AgentTranslationFailed,
            GuardrailsWriteFailed,
            PluginNotFound,
            PluginAlreadyInState,
            PluginManifestParseError,
            SkillFrontmatterParseError,
            ModelMissing,
            ModelCorrupt,
            ModelChecksumMismatch,
            ModelRegistrationParseError,
            InferenceRuntimeInitFailure,
            VectorExtensionInitFailure,
            EmbeddingGenerationFailure,
            RerankingFailure,
            QueryNoResultsStrict,
            EmbedderNameDrift,
            EmbedderVersionDrift,
            ReindexScopedEmbedderChange,
            IndexBusy,
            IndexIntegrityCheckFailure,
            SchemaTooNew,
            CatalogHasEnabledPlugins,
            NotATerminal,
            McpStartup,
            McpIo,
            WorkspaceMalformed,
            SchemaMigration,
            DoctorFixUnsafe,
            PluginNotConverted,
            OutputExists,
            TemplateInvalid,
            SourceFormatUnrecognized,
            ConversionUnsupportedStrict,
            ValidationFoundErrors,
            ValidationStrictWarnings,
            MetaSkillNotFound,
            MetaInstallFailed,
            NoHarnessDetected,
            TelemetryEndpointUnreachable,
            TelemetryConfigInvalid,
            TelemetryQueueCorrupt,
            ProviderConfigInvalid,
            ProviderRequestFailed,
            RemoteEmbeddingInvalid,
        ];
        for c in all {
            // The compile-time guard: a non-exhaustive `match` (no `_` arm)
            // touching every variant — adding a variant breaks the build here.
            let _both = match c {
                Internal
                | Usage
                | CatalogNotFound
                | CatalogAlreadyExists
                | ManifestInvalid
                | GitFailed
                | Io
                | Interrupted
                | PluginDataDirWriteFailed
                | WorkspaceNotBound
                | WorkspaceNotFound
                | WorkspaceAlreadyExists
                | WorkspaceNameInvalid
                | WorkspaceHasBoundProjects
                | CompositionError
                | HarnessNotSupported
                | HarnessClash
                | SummariserFailure
                | WorkspaceDataDirWriteFailed
                | PromptArgumentMismatch
                | EntryNotFound
                | SubstitutionFailed
                | InvalidArgumentFrontmatter
                | HookSpecParseError
                | HookSettingsWriteFailed
                | AgentTranslationFailed
                | GuardrailsWriteFailed
                | PluginNotFound
                | PluginAlreadyInState
                | PluginManifestParseError
                | SkillFrontmatterParseError
                | ModelMissing
                | ModelCorrupt
                | ModelChecksumMismatch
                | ModelRegistrationParseError
                | InferenceRuntimeInitFailure
                | VectorExtensionInitFailure
                | EmbeddingGenerationFailure
                | RerankingFailure
                | QueryNoResultsStrict
                | EmbedderNameDrift
                | EmbedderVersionDrift
                | ReindexScopedEmbedderChange
                | IndexBusy
                | IndexIntegrityCheckFailure
                | SchemaTooNew
                | CatalogHasEnabledPlugins
                | NotATerminal
                | McpStartup
                | McpIo
                | WorkspaceMalformed
                | SchemaMigration
                | DoctorFixUnsafe
                | PluginNotConverted
                | OutputExists
                | TemplateInvalid
                | SourceFormatUnrecognized
                | ConversionUnsupportedStrict
                | ValidationFoundErrors
                | ValidationStrictWarnings
                | MetaSkillNotFound
                | MetaInstallFailed
                | NoHarnessDetected
                | TelemetryEndpointUnreachable
                | TelemetryConfigInvalid
                | TelemetryQueueCorrupt
                | ProviderConfigInvalid
                | ProviderRequestFailed
                | RemoteEmbeddingInvalid => (c.retryable(), c.remediation()),
            };
            // No remediation string may carry a credential: every value is a
            // static `tome …` command literal, so it cannot contain a secret.
            if let Some(cmd) = c.remediation() {
                assert!(
                    cmd.starts_with("tome "),
                    "remediation for {c:?} must be a `tome …` command literal, got {cmd:?}",
                );
            }
        }
    }
}

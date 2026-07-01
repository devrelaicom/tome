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
        "plugin `{0}` is not installed under any registered catalog\nhint: list valid plugin ids with `tome plugin list`"
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
}

//! Typed, emit-only telemetry event records (Phase 10).
//!
//! Everything here is a closed, `Serialize`-only record — the anonymous stream
//! never lets a free-form string off the box. There is no `deny_unknown_fields`
//! (that is reserved for *inputs*; these are outputs).

use serde::Serialize;

/// The host operating system, as a closed enum (NFR-012). `Windows` is a
/// RESERVED value: no runtime target on our build matrix maps to it today, but
/// it stays in the enum so a future Windows port serialises a known token
/// rather than a junk string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Macos,
    Linux,
    Windows,
}

/// The host CPU architecture, as a closed enum. The per-variant renames pin the
/// wire tokens exactly (`x86_64`/`aarch64`) — `rename_all = "lowercase"` would
/// not reproduce the underscores/digits faithfully.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Arch {
    #[serde(rename = "x86_64")]
    X86_64,
    #[serde(rename = "aarch64")]
    Aarch64,
}

// The enums are TOTAL by construction (FR-023a, research §R-3): a source build
// for a target outside the supported matrix fails HERE rather than shipping a
// value the `CURRENT_*` resolvers below cannot map. Supported matrix:
// (macos | linux) × (x86_64 | aarch64).
#[cfg(not(any(
    all(target_os = "macos", target_arch = "x86_64"),
    all(target_os = "macos", target_arch = "aarch64"),
    all(target_os = "linux", target_arch = "x86_64"),
    all(target_os = "linux", target_arch = "aarch64"),
)))]
compile_error!("telemetry os/arch enum: unsupported target — extend src/telemetry/event.rs");

/// This binary's OS, resolved at compile time from `cfg!(target_os)`. The
/// compile-error guard above guarantees exactly one arm matches.
pub const CURRENT_OS: Os = if cfg!(target_os = "macos") {
    Os::Macos
} else {
    // Only `linux` remains after the compile-error guard rules out everything
    // else; `Windows` is reserved and never reached at runtime on our matrix.
    Os::Linux
};

/// This binary's architecture, resolved at compile time from `cfg!(target_arch)`.
pub const CURRENT_ARCH: Arch = if cfg!(target_arch = "x86_64") {
    Arch::X86_64
} else {
    // Only `aarch64` remains after the compile-error guard.
    Arch::Aarch64
};

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// A validated RFC-4122 version-4 UUID — the ONLY identifier type that ever
/// appears on the wire (install id + per-process session id).
///
/// The inner string is private so the only ways to obtain one are [`Uuid::mint`]
/// (mints a fresh random v4) and [`Uuid::parse`] (validates a stored value).
/// That keeps "a malformed / non-v4 id reached an event" unrepresentable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uuid(String);

impl Uuid {
    /// Mint a fresh v4 UUID from 16 OS-random bytes (research §R-2).
    ///
    /// We hand-roll the 16-byte → version/variant-stamped → hyphenated render
    /// rather than pull a `uuid` crate: `getrandom` is already a direct dep for
    /// exactly this, and the format is trivial and fully pinned by tests.
    ///
    /// Returns `None` if the OS RNG is unavailable (fd exhaustion / seccomp /
    /// early boot). This is the silent best-effort telemetry path: under
    /// `panic = "abort"` an `expect` here would abort the user's foreground
    /// command from a telemetry append, contradicting the "INFALLIBLE, never
    /// crash/propagate" invariant. Callers degrade an RNG failure to the
    /// existing best-effort drop (silent paths) or an `Err` (explicit
    /// `telemetry on`/`reset` commands).
    pub fn mint() -> Option<Uuid> {
        let mut buf = [0u8; 16];
        // `getrandom::fill` errs only if the OS RNG is unavailable. We never
        // panic on this path; bubble the failure up as `None`. This path runs
        // once per install (id mint) / once per process (session), never in a
        // hot loop.
        getrandom::fill(&mut buf).ok()?;

        // RFC 4122: high nibble of byte 6 = version (4); top two bits of byte 8
        // = variant (0b10).
        buf[6] = (buf[6] & 0x0f) | 0x40;
        buf[8] = (buf[8] & 0x3f) | 0x80;

        Some(Uuid(render_hyphenated(&buf)))
    }

    /// Parse and validate a stored v4 UUID string. Returns `None` for anything
    /// that is not lowercase-hex `8-4-4-4-12` with version nibble `4` and a
    /// variant nibble in `{8,9,a,b}`.
    pub fn parse(s: &str) -> Option<Uuid> {
        let bytes = s.as_bytes();
        if bytes.len() != 36 {
            return None;
        }
        // Hyphen positions for `8-4-4-4-12`.
        for &i in &[8usize, 13, 18, 23] {
            if bytes[i] != b'-' {
                return None;
            }
        }
        for (i, &b) in bytes.iter().enumerate() {
            if i == 8 || i == 13 || i == 18 || i == 23 {
                continue;
            }
            // Lowercase hex only (uppercase would round-trip-mismatch our mint).
            if !(b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                return None;
            }
        }
        // Version nibble: first char of the third group (index 14) must be '4'.
        if bytes[14] != b'4' {
            return None;
        }
        // Variant nibble: first char of the fourth group (index 19) in 8/9/a/b.
        if !matches!(bytes[19], b'8' | b'9' | b'a' | b'b') {
            return None;
        }
        Some(Uuid(s.to_string()))
    }

    /// The canonical lowercase-hyphenated string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Render 16 bytes as lowercase hyphenated `8-4-4-4-12`.
fn render_hyphenated(buf: &[u8; 16]) -> String {
    let mut out = String::with_capacity(36);
    for (i, b) in buf.iter().enumerate() {
        if matches!(i, 4 | 6 | 8 | 10) {
            out.push('-');
        }
        // `{:02x}` is lowercase, zero-padded — exactly the canonical form.
        out.push_str(&format!("{b:02x}"));
    }
    out
}

impl serde::Serialize for Uuid {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl std::fmt::Display for Uuid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// Version-string carve-out
// ---------------------------------------------------------------------------

/// A version string read from Tome's OWN `last-version` stamp file.
///
/// WHY this exists at all given the "no free-form `String` in event fields" rule:
/// the single unavoidable runtime-string field is `tome.upgrade.from_version` —
/// the binary's *prior* version. It is Tome-authored content (it was written by a
/// previous Tome run into a `0600` stamp file), never user input, and a version
/// string is not a fingerprint. Wrapping it in a dedicated newtype rather than a
/// bare `String` keeps the carve-out auditable: a reader can `grep VersionStr` to
/// find every place a runtime string is permitted, and the type can only be built
/// from a `last-version` read at the call site. It serializes as the bare string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionStr(String);

impl VersionStr {
    /// Construct from a `last-version` read. Named to make the provenance
    /// explicit at every call site (the ONLY sanctioned source).
    pub fn from_last_version(s: impl Into<String>) -> VersionStr {
        VersionStr(s.into())
    }

    /// The underlying version string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl serde::Serialize for VersionStr {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

// ---------------------------------------------------------------------------
// Envelope (FR-023) — field order is LOAD-BEARING (pinned in TELEMETRY.md)
// ---------------------------------------------------------------------------

/// The shared envelope stamped onto every event at enqueue time. The field
/// declaration order below is the on-wire order (serde preserves struct order)
/// and is pinned byte-for-byte by the worked examples — do NOT reorder.
#[derive(Debug, Clone, Serialize)]
pub struct Envelope {
    /// EVENT schema version, starting at `1`. This is the telemetry *event*
    /// schema and is unrelated to the SQLite DB `SCHEMA_VERSION` (which stays 4);
    /// telemetry adds no table/column so the DB schema is untouched.
    schema_version: u32,
    install_uuid: Uuid,
    session_uuid: Uuid,
    tome_version: &'static str,
    os: Os,
    arch: Arch,
    /// RFC3339, millisecond precision, UTC (e.g. `2026-06-11T14:11:45.123Z`).
    /// Pre-formatted by the caller (not here) so the envelope stays trivially
    /// injectable/testable without pulling time formatting into this type.
    timestamp: String,
    /// The dotted event type. `String` (not `&'static str`) because the
    /// catalog-attributed stream builds it DYNAMICALLY at enqueue time —
    /// `catalog.<short_id>.<suffix>` (e.g. `catalog.midnight.entry_invoked`). The
    /// anonymous path still passes a `&'static str` const (`E::EVENT_TYPE`) and
    /// `.to_string()`s it — one tiny per-event allocation, negligible against the
    /// JSON serialize that follows. The wire shape is IDENTICAL to `&str`: serde
    /// emits a JSON string either way, so the existing byte-stable anonymous pins
    /// do not move.
    event_type: String,
    /// The applied sampling rate (FR-060), `Some(1.0)` for anonymous events.
    /// `Option` + `skip_serializing_if` so the ATTRIBUTED stream — which is NEVER
    /// sampled (FR-058) — OMITS the field entirely (matching data-model §10
    /// worked example 2, whose attributed line carries no `sample_rate`). The
    /// anonymous envelope always sets `Some(1.0)`, so its pinned line is unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    sample_rate: Option<f32>,
}

impl Envelope {
    /// Build an ANONYMOUS-stream envelope from the injectable pieces; fills the
    /// constants (`schema_version = 1`, `tome_version = env!(CARGO_PKG_VERSION)`,
    /// `sample_rate = Some(1.0)`). The `event_type` is a `&'static str` const at
    /// every anonymous call site, `.to_string()`d into the dynamic field.
    pub fn new(
        install_uuid: Uuid,
        session_uuid: Uuid,
        os: Os,
        arch: Arch,
        timestamp: String,
        event_type: &'static str,
    ) -> Envelope {
        Envelope::with_event_type(
            install_uuid,
            session_uuid,
            os,
            arch,
            timestamp,
            event_type.to_string(),
            Some(1.0),
        )
    }

    /// Build a CATALOG-ATTRIBUTED-stream envelope. The `event_type` is built
    /// dynamically by the caller (`catalog.<id>.<suffix>`) and `sample_rate` is
    /// `None` — attributed events are never sampled (FR-058), so the field is
    /// omitted on the wire entirely.
    pub fn new_attributed(
        install_uuid: Uuid,
        session_uuid: Uuid,
        os: Os,
        arch: Arch,
        timestamp: String,
        event_type: String,
    ) -> Envelope {
        Envelope::with_event_type(
            install_uuid,
            session_uuid,
            os,
            arch,
            timestamp,
            event_type,
            None,
        )
    }

    /// Shared constructor: fills the constants and takes the two stream-varying
    /// pieces (the dynamic `event_type` and the `sample_rate`).
    fn with_event_type(
        install_uuid: Uuid,
        session_uuid: Uuid,
        os: Os,
        arch: Arch,
        timestamp: String,
        event_type: String,
        sample_rate: Option<f32>,
    ) -> Envelope {
        Envelope {
            schema_version: 1,
            install_uuid,
            session_uuid,
            tome_version: env!("CARGO_PKG_VERSION"),
            os,
            arch,
            timestamp,
            event_type,
            sample_rate,
        }
    }
}

// ---------------------------------------------------------------------------
// Closed enums (event-specific dimensions)
// ---------------------------------------------------------------------------

/// Which agentic harness an action concerns / originates from. Serializes kebab
/// so the wire tokens match the harness ids used everywhere else in Tome — with
/// ONE exception: `GeminiCli` renders to `gemini-cli` while the harness module
/// names itself `gemini` (the rename bridged by
/// [`crate::commands::harness::harness_name_to_enum`]). Every other variant's
/// `kebab-case` token equals its module `name()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Cursor,
    Codex,
    Opencode,
    GeminiCli,
    // Phase 11 — additional harness support. For these, `kebab-case` renders
    // each wire token to match the harness module's `name()` exactly (the
    // `gemini`→`gemini-cli` rename above is the only mismatch). No
    // `antigravity-cli` variant: it is an alias of `gemini` and resolves before
    // any emit.
    CopilotCli,
    Copilot,
    Devin,
    Cline,
    Junie,
    JetbrainsAi,
    Antigravity,
    Pi,
    Crush,
    Zed,
    Kiro,
    Generic,
    GenericOp,
    Goose,
}

/// Whether the event originated from the CLI or the MCP server surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    Cli,
    Mcp,
}

/// Best-effort heuristic of how the running binary was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallMethod {
    Cargo,
    Brew,
    Curl,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogAction {
    Added,
    Removed,
    Updated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginAction {
    Enabled,
    Disabled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceAction {
    Init,
    Use,
    Rename,
    Remove,
    Sync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HarnessAction {
    Use,
    Sync,
    Remove,
}

/// Coarse three-state outcome for actions where success can be partial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok,
    Partial,
    Failed,
}

/// Outcome specialised for the authoring (`create`/`convert`/`lint`) verbs,
/// which distinguish warnings/errors/strict-refusal rather than partial.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoringOutcome {
    Ok,
    Warnings,
    Errors,
    StrictRefused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthoringVerb {
    Create,
    Convert,
    Lint,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Artifact {
    Catalog,
    Plugin,
    Skill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    ClaudeCode,
    Codex,
    NativeSkill,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MetaAction {
    Add,
    Remove,
    Fix,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryKind {
    Skill,
    Command,
    Agent,
}

impl From<crate::plugin::identity::EntryKind> for EntryKind {
    /// Bridge the registry's `identity::EntryKind` (the index-row discriminator)
    /// into the closed telemetry enum. Both are total over the same three
    /// variants; this exhaustive match surfaces a future variant addition on
    /// either side as a compile error rather than a silent miscategorisation.
    fn from(kind: crate::plugin::identity::EntryKind) -> Self {
        match kind {
            crate::plugin::identity::EntryKind::Skill => EntryKind::Skill,
            crate::plugin::identity::EntryKind::Command => EntryKind::Command,
            crate::plugin::identity::EntryKind::Agent => EntryKind::Agent,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    Command,
    Persona,
    Builtin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceType {
    Git,
    Local,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReindexScope {
    All,
    Catalog,
    Plugin,
}

/// Phase 12 — which model PROVIDER served a capability, as a CLOSED enum. This
/// is the ONLY thing telemetry records about provider configuration: the kind,
/// never the registry name / model id / `base_url` / credential. `Bundled` is
/// the default-local path (no provider configured). Being a closed enum, it can
/// NEVER carry a free-form string — the typed-event privacy guarantee
/// (data-model / contracts/telemetry.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Bundled,
    Openai,
    Anthropic,
    Gemini,
    Voyage,
}

impl From<crate::config::ProviderKind> for ProviderKind {
    /// Bridge the config provider kind into the closed telemetry enum. The
    /// config enum has no `Bundled` (it only describes a configured remote
    /// provider); the `Bundled` telemetry value is supplied at the emit site
    /// when no provider is configured. This exhaustive match surfaces a future
    /// config-kind addition as a compile error.
    fn from(kind: crate::config::ProviderKind) -> Self {
        match kind {
            crate::config::ProviderKind::Openai => ProviderKind::Openai,
            crate::config::ProviderKind::Anthropic => ProviderKind::Anthropic,
            crate::config::ProviderKind::Gemini => ProviderKind::Gemini,
            crate::config::ProviderKind::Voyage => ProviderKind::Voyage,
        }
    }
}

impl ProviderKind {
    /// Map the configured EMBEDDING provider to the closed telemetry kind:
    /// `Bundled` when no `[embedding]` provider is referenced (or the reference
    /// can't be resolved to a registry entry), else the entry's kind. This is
    /// the SSOT both the CLI (`query::run_with_deps`) and the MCP
    /// (`search_skills`) emit sites call so they can never diverge. Records ONLY
    /// the kind — never the provider name / model / `base_url` (the typed-event
    /// privacy guarantee). A missing/unresolvable reference degrades to
    /// `Bundled`; telemetry never propagates a config error.
    pub fn for_embedding(cfg: &crate::config::Config) -> Self {
        let Some(name) = cfg.embedding.provider.as_deref() else {
            return ProviderKind::Bundled;
        };
        match cfg.providers.get(name) {
            Some(entry) => ProviderKind::from(entry.kind),
            None => ProviderKind::Bundled,
        }
    }

    /// Map the configured RERANKING provider to the closed telemetry kind:
    /// `Bundled` when no `[reranker]` provider is referenced (or the reference
    /// can't be resolved to a registry entry), else the entry's kind (Voyage in
    /// v1). The SSOT both the CLI (`query::run_with_deps`) and the MCP
    /// (`search_skills`) emit sites call so the per-capability kind can never
    /// diverge (FR-022 — three independent fields). Records ONLY the kind, never
    /// the provider name / model / `base_url`. A missing/unresolvable reference
    /// degrades to `Bundled`; telemetry never propagates a config error.
    pub fn for_reranker(cfg: &crate::config::Config) -> Self {
        let Some(name) = cfg.reranker.provider.as_deref() else {
            return ProviderKind::Bundled;
        };
        match cfg.providers.get(name) {
            Some(entry) => ProviderKind::from(entry.kind),
            None => ProviderKind::Bundled,
        }
    }
}

// ---------------------------------------------------------------------------
// Anonymous event trait + the 18 event structs
// ---------------------------------------------------------------------------

use crate::telemetry::buckets::{
    CountBucket, FindingsBucket, LatencyBucket, LoadBucket, RankBucket,
};

/// Every anonymous (`tome.*`) event carries its dotted event type as an
/// associated const. The bound on `Serialize` lets the [`Wire`] wrapper flatten
/// any event uniformly behind the shared envelope.
pub trait AnonymousEvent: Serialize {
    /// The dotted event type written to the envelope's `event_type` field.
    const EVENT_TYPE: &'static str;
}

/// `tome.install` — emitted once when the install id is first minted.
#[derive(Debug, Clone, Serialize)]
pub struct Install {
    pub install_method: InstallMethod,
}

/// `tome.upgrade` — emitted when `last-version` differs from the running binary.
/// `from_version` is Tome's OWN prior version (see [`VersionStr`]).
#[derive(Debug, Clone, Serialize)]
pub struct Upgrade {
    pub from_version: VersionStr,
}

/// `tome.heartbeat` — once-per-UTC-day inventory snapshot (all bucketed).
#[derive(Debug, Clone, Serialize)]
pub struct Heartbeat {
    pub skills_bucket: CountBucket,
    pub commands_bucket: CountBucket,
    pub agents_bucket: CountBucket,
    pub workspaces_bucket: CountBucket,
    pub catalogs_bucket: CountBucket,
    /// Detected harnesses, sorted by the caller for a deterministic wire shape.
    pub harnesses_detected: Vec<Harness>,
}

/// `tome.search` — one semantic search round-trip.
#[derive(Debug, Clone, Serialize)]
pub struct Search {
    pub surface: Surface,
    pub latency_bucket: LatencyBucket,
    pub candidates_returned: CountBucket,
    pub reranker_used: bool,
    pub strict: bool,
    pub corpus_size_bucket: CountBucket,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedder_model_id: Option<&'static str>,
    /// Phase 12 — which PROVIDER served the embedding for this search, as the
    /// closed [`ProviderKind`] (`Bundled` when no `[embedding]` provider is
    /// configured). NEVER the provider name / model id / `base_url` — only the
    /// kind. Always serialised (no `skip`) so the wire shape is stable.
    pub embedding_provider_kind: ProviderKind,
    /// Phase 12 / US3 — which PROVIDER served the RERANKING for this search, as
    /// the closed [`ProviderKind`] (`Bundled` when no `[reranker]` provider is
    /// configured, including when reranking is disabled — `reranker_used` already
    /// distinguishes that). Independent of `embedding_provider_kind` (FR-022 —
    /// a remote reranker with a bundled embedder is attributed accurately). NEVER
    /// the provider name / model id / `base_url`. Always serialised (no `skip`).
    pub reranker_provider_kind: ProviderKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}

/// `tome.entry_info` — a `get_skill_info` lookup.
#[derive(Debug, Clone, Serialize)]
pub struct EntryInfo {
    pub rank_bucket: RankBucket,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}

/// `tome.entry_invoked` — an entry body was fetched/invoked.
#[derive(Debug, Clone, Serialize)]
pub struct EntryInvoked {
    pub entry_kind: EntryKind,
    pub rank_bucket: RankBucket,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}

/// `tome.prompt_invoked` — an MCP prompt was invoked.
#[derive(Debug, Clone, Serialize)]
pub struct PromptInvoked {
    pub prompt_kind: PromptKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}

/// `tome.catalog_action` — catalog add/remove/update.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogActionEvent {
    pub action: CatalogAction,
    pub source_type: SourceType,
}

/// `tome.plugin_action` — plugin enable/disable.
#[derive(Debug, Clone, Serialize)]
pub struct PluginActionEvent {
    pub action: PluginAction,
}

/// `tome.workspace_action` — workspace lifecycle verb.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceActionEvent {
    pub action: WorkspaceAction,
}

/// `tome.harness_action` — harness use/sync/remove.
#[derive(Debug, Clone, Serialize)]
pub struct HarnessActionEvent {
    pub action: HarnessAction,
    pub harness: Harness,
}

/// `tome.authoring_action` — a create/convert/lint run.
#[derive(Debug, Clone, Serialize)]
pub struct AuthoringActionEvent {
    pub verb: AuthoringVerb,
    pub artifact: Artifact,
    pub source_format: SourceFormat,
    pub outcome: AuthoringOutcome,
}

/// `tome.meta_action` — a meta-skill add/remove/fix.
#[derive(Debug, Clone, Serialize)]
pub struct MetaActionEvent {
    pub action: MetaAction,
    pub outcome: Outcome,
}

/// `tome.model_download` — a model download attempt. `model_id` is a closed set
/// by construction (the pinned `MODEL_REGISTRY`), hence `&'static str`.
#[derive(Debug, Clone, Serialize)]
pub struct ModelDownload {
    pub model_id: &'static str,
    pub outcome: Outcome,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_class: Option<crate::error::ErrorCategory>,
}

/// `tome.cold_start` — process-start embedder/index readiness timings.
#[derive(Debug, Clone, Serialize)]
pub struct ColdStart {
    pub embedder_load_bucket: LoadBucket,
    pub index_ready_bucket: LoadBucket,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedder_model_id: Option<&'static str>,
}

/// `tome.doctor_run` — a `tome doctor` invocation.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorRun {
    pub fix: bool,
    pub findings_bucket: FindingsBucket,
}

/// `tome.reindex` — a `tome reindex` invocation.
#[derive(Debug, Clone, Serialize)]
pub struct Reindex {
    pub scope: ReindexScope,
    pub forced: bool,
    pub outcome: Outcome,
}

/// `tome.summary` — a workspace summary was (re)generated. Records WHICH
/// provider kind served the summariser (Phase 12) plus the outcome. NO provider
/// name / model id / `base_url` is recorded — only the closed [`ProviderKind`].
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub summariser_provider_kind: ProviderKind,
    pub outcome: Outcome,
}

/// `tome.error` — a classified error surfaced to a user-facing command.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorEvent {
    pub error_class: crate::error::ErrorCategory,
    pub surface: Surface,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}

impl AnonymousEvent for Install {
    const EVENT_TYPE: &'static str = "tome.install";
}
impl AnonymousEvent for Upgrade {
    const EVENT_TYPE: &'static str = "tome.upgrade";
}
impl AnonymousEvent for Heartbeat {
    const EVENT_TYPE: &'static str = "tome.heartbeat";
}
impl AnonymousEvent for Search {
    const EVENT_TYPE: &'static str = "tome.search";
}
impl AnonymousEvent for EntryInfo {
    const EVENT_TYPE: &'static str = "tome.entry_info";
}
impl AnonymousEvent for EntryInvoked {
    const EVENT_TYPE: &'static str = "tome.entry_invoked";
}
impl AnonymousEvent for PromptInvoked {
    const EVENT_TYPE: &'static str = "tome.prompt_invoked";
}
impl AnonymousEvent for CatalogActionEvent {
    const EVENT_TYPE: &'static str = "tome.catalog_action";
}
impl AnonymousEvent for PluginActionEvent {
    const EVENT_TYPE: &'static str = "tome.plugin_action";
}
impl AnonymousEvent for WorkspaceActionEvent {
    const EVENT_TYPE: &'static str = "tome.workspace_action";
}
impl AnonymousEvent for HarnessActionEvent {
    const EVENT_TYPE: &'static str = "tome.harness_action";
}
impl AnonymousEvent for AuthoringActionEvent {
    const EVENT_TYPE: &'static str = "tome.authoring_action";
}
impl AnonymousEvent for MetaActionEvent {
    const EVENT_TYPE: &'static str = "tome.meta_action";
}
impl AnonymousEvent for ModelDownload {
    const EVENT_TYPE: &'static str = "tome.model_download";
}
impl AnonymousEvent for ColdStart {
    const EVENT_TYPE: &'static str = "tome.cold_start";
}
impl AnonymousEvent for DoctorRun {
    const EVENT_TYPE: &'static str = "tome.doctor_run";
}
impl AnonymousEvent for Reindex {
    const EVENT_TYPE: &'static str = "tome.reindex";
}
impl AnonymousEvent for Summary {
    const EVENT_TYPE: &'static str = "tome.summary";
}
impl AnonymousEvent for ErrorEvent {
    const EVENT_TYPE: &'static str = "tome.error";
}

// ---------------------------------------------------------------------------
// Catalog-attributed events (`catalog.<id>.*`) — the ONLY bounded-`String`
// carve-out (FR-059)
// ---------------------------------------------------------------------------
//
// THE SINGLE EXCEPTION to the "no free-form `String` in an event field" rule
// (FR-034): the structs below carry PUBLISHED artefact names + versions
// (`plugin_name`, `entry_name`, `plugin_version`, …) as bounded `String`s. These
// are not user secrets and not a fingerprint — they are the names a maintainer
// already published to a public catalog, and they are emitted ONLY on the
// attributed stream for an allowlisted catalog (see `allowlist.rs`). Every other
// event type in this module is bucketed ints / closed enums / UUIDs. If you are
// adding a `String` field ANYWHERE ELSE in this file, stop: it almost certainly
// belongs in a closed enum or a bucket instead.

/// Every catalog-attributed (`catalog.<id>.*`) event carries an event-type
/// SUFFIX (the part after `catalog.<catalog_id>.`) plus the resolved short id.
/// The full `event_type` is assembled at enqueue time as
/// `format!("catalog.{}.{}", self.catalog_id(), E::EVENT_SUFFIX)`.
pub trait AttributedEvent: Serialize {
    /// The trailing event-type segment (e.g. `plugin_enabled`, `entry_invoked`).
    const EVENT_SUFFIX: &'static str;
    /// The allowlist short id (also the `catalog.<id>.` prefix). Stored on every
    /// struct so the enqueue path can build the full type without a re-resolve.
    fn catalog_id(&self) -> &'static str;
}

/// `catalog.<id>.plugin_enabled` — an allowlisted-catalog plugin was enabled.
#[derive(Debug, Clone, Serialize)]
pub struct PluginEnabled {
    pub plugin_name: String,
    pub plugin_version: String,
    pub catalog_id: &'static str,
}

/// `catalog.<id>.plugin_disabled` — an allowlisted-catalog plugin was disabled.
#[derive(Debug, Clone, Serialize)]
pub struct PluginDisabled {
    pub plugin_name: String,
    pub plugin_version: String,
    pub catalog_id: &'static str,
}

/// `catalog.<id>.plugin_updated` — per plugin whose version changed during a
/// `catalog update` of an allowlisted catalog.
#[derive(Debug, Clone, Serialize)]
pub struct PluginUpdated {
    pub plugin_name: String,
    pub from_version: String,
    pub to_version: String,
    pub catalog_id: &'static str,
}

/// `catalog.<id>.entry_invoked` — an allowlisted-catalog entry body was invoked.
#[derive(Debug, Clone, Serialize)]
pub struct AttributedEntryInvoked {
    pub entry_name: String,
    pub entry_kind: EntryKind,
    pub plugin_name: String,
    pub plugin_version: String,
    pub catalog_id: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}

/// `catalog.<id>.search_result` — fires once per allowlisted-catalog entry that
/// appears in a result. `rank` is EXACT (FR-057), not bucketed: selection
/// attribution is a server-side join on `(session_uuid, entry_name)` against the
/// later `entry_invoked`; the client NEVER back-edits queued events.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub entry_name: String,
    pub entry_kind: EntryKind,
    pub plugin_name: String,
    pub rank: u32,
    pub catalog_id: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}

/// `catalog.<id>.error` — a classified error involving an allowlisted-catalog
/// plugin. `entry_name` is optional (some errors are plugin-level, not entry).
#[derive(Debug, Clone, Serialize)]
pub struct AttributedError {
    pub plugin_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_name: Option<String>,
    pub error_class: crate::error::ErrorCategory,
    pub plugin_version: String,
    pub catalog_id: &'static str,
}

impl AttributedEvent for PluginEnabled {
    const EVENT_SUFFIX: &'static str = "plugin_enabled";
    fn catalog_id(&self) -> &'static str {
        self.catalog_id
    }
}
impl AttributedEvent for PluginDisabled {
    const EVENT_SUFFIX: &'static str = "plugin_disabled";
    fn catalog_id(&self) -> &'static str {
        self.catalog_id
    }
}
impl AttributedEvent for PluginUpdated {
    const EVENT_SUFFIX: &'static str = "plugin_updated";
    fn catalog_id(&self) -> &'static str {
        self.catalog_id
    }
}
impl AttributedEvent for AttributedEntryInvoked {
    const EVENT_SUFFIX: &'static str = "entry_invoked";
    fn catalog_id(&self) -> &'static str {
        self.catalog_id
    }
}
impl AttributedEvent for SearchResult {
    const EVENT_SUFFIX: &'static str = "search_result";
    fn catalog_id(&self) -> &'static str {
        self.catalog_id
    }
}
impl AttributedEvent for AttributedError {
    const EVENT_SUFFIX: &'static str = "error";
    fn catalog_id(&self) -> &'static str {
        self.catalog_id
    }
}

// ---------------------------------------------------------------------------
// Timestamp formatting
// ---------------------------------------------------------------------------

/// Format an instant as the EXACT envelope timestamp shape:
/// `YYYY-MM-DDTHH:MM:SS.mmmZ` — UTC, always exactly 3 subsecond digits, a
/// literal `Z` (never `+00:00`). Matches the data-model worked example
/// `2026-06-11T14:11:45.123Z`.
///
/// Hand-rolled from the date/time components rather than via a
/// `format_description` string: we need an UNCONDITIONAL 3-digit millisecond
/// field (`time`'s subsecond formatters drop trailing zeros or require a fixed
/// nanosecond width) and a literal `Z`, both of which are trivial to pin by
/// hand. Sub-millisecond precision is TRUNCATED (not rounded) — `.123999` ⇒
/// `.123` — so the field is the integer-millisecond floor, deterministic for a
/// given instant.
pub fn format_rfc3339_millis(dt: time::OffsetDateTime) -> String {
    // Normalise to UTC first so a non-UTC input still renders with `Z`.
    let dt = dt.to_offset(time::UtcOffset::UTC);
    let date = dt.date();
    let time_of_day = dt.time();
    // Nanoseconds since the second, floored to whole milliseconds.
    let millis = time_of_day.nanosecond() / 1_000_000;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        date.year(),
        date.month() as u8,
        date.day(),
        time_of_day.hour(),
        time_of_day.minute(),
        time_of_day.second(),
        millis,
    )
}

// ---------------------------------------------------------------------------
// Wire serialization (envelope-first, byte-stable)
// ---------------------------------------------------------------------------

/// Flattens the shared envelope (9 fields, IN ORDER) ahead of an event's own
/// fields, yielding one JSONL line. Borrows both sides so no event needs to own
/// or clone its envelope to be serialized.
#[derive(Serialize)]
pub struct Wire<'a, E: Serialize> {
    #[serde(flatten)]
    envelope: &'a Envelope,
    #[serde(flatten)]
    event: &'a E,
}

/// Serialize an event behind its envelope as a compact single-line JSON string
/// (NO trailing newline — the queue appends `\n`).
pub fn to_line(envelope: &Envelope, event: &impl Serialize) -> Result<String, serde_json::Error> {
    let wire = Wire { envelope, event };
    serde_json::to_string(&wire)
}

/// The FIXED `tome_version` token used by the byte-stable pin envelopes below.
///
/// WHY a fixed const and NOT `env!("CARGO_PKG_VERSION")`: the `TELEMETRY.md` pin
/// (US5) compares the document's two worked JSON examples byte-for-byte against
/// these constructors. Those examples show `"0.6.0"`. `tome_version` is *data*
/// (a value on the wire), not *schema* (the field order / set), so a future
/// crate-version bump must NOT break the schema pin — otherwise every release
/// would have to re-edit the worked examples in lockstep with the crate version
/// for no schema reason. Pinning the test-only envelopes to a fixed version
/// decouples the schema-stability guarantee from the crate's marketing version.
/// The current crate version IS `0.6.0`, so this changes nothing today; it only
/// keeps the pins green across the next version bump.
const TEST_TOME_VERSION: &str = "0.6.0";

/// The ONE canonical fixed [`Envelope`] for byte-stable pin tests across the
/// suite (data-model §10). The `TELEMETRY.md` pin (US5) and the
/// `telemetry_events.rs` wire pins (US2) all build their expected lines off
/// this so the fixed install/session uuids, os/arch, timestamp, sample rate,
/// AND `tome_version` live in exactly one place — change them here and every
/// pin updates in lockstep.
///
/// Doc-hidden: it constructs from arbitrary (test-only) fixed uuids and is not
/// part of the published API. `tome_version` is fixed to [`TEST_TOME_VERSION`]
/// (NOT `env!("CARGO_PKG_VERSION")`) so the schema pin survives a crate-version
/// bump — see the const's doc-comment.
#[doc(hidden)]
pub fn fixed_envelope_for_tests(event_type: &'static str) -> Envelope {
    let mut envelope = Envelope::new(
        Uuid::parse("0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f")
            .expect("canonical fixed install uuid is valid"),
        Uuid::parse("7f6e5d4c-3b2a-4f1e-9c8b-1a2b3c4d5e6f")
            .expect("canonical fixed session uuid is valid"),
        Os::Macos,
        Arch::Aarch64,
        "2026-06-11T14:11:45.123Z".to_string(),
        event_type,
    );
    envelope.tome_version = TEST_TOME_VERSION;
    envelope
}

/// The CATALOG-ATTRIBUTED counterpart of [`fixed_envelope_for_tests`]: same fixed
/// install/session uuids + os/arch + `tome_version`, but built via
/// [`Envelope::new_attributed`] (so NO `sample_rate`) with a DYNAMIC
/// `event_type`. The timestamp here (`2026-06-11T14:12:03.456Z`) matches
/// data-model §10 worked example 2 exactly, so the attributed pin is
/// byte-for-byte against the data-model.
#[doc(hidden)]
pub fn fixed_attributed_envelope_for_tests(event_type: String) -> Envelope {
    let mut envelope = Envelope::new_attributed(
        Uuid::parse("0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f")
            .expect("canonical fixed install uuid is valid"),
        Uuid::parse("7f6e5d4c-3b2a-4f1e-9c8b-1a2b3c4d5e6f")
            .expect("canonical fixed session uuid is valid"),
        Os::Macos,
        Arch::Aarch64,
        "2026-06-11T14:12:03.456Z".to_string(),
        event_type,
    );
    envelope.tome_version = TEST_TOME_VERSION;
    envelope
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn os_serialises_lowercase() {
        assert_eq!(serde_json::to_string(&Os::Macos).unwrap(), "\"macos\"");
        assert_eq!(serde_json::to_string(&Os::Linux).unwrap(), "\"linux\"");
        assert_eq!(serde_json::to_string(&Os::Windows).unwrap(), "\"windows\"");
    }

    #[test]
    fn arch_serialises_with_pinned_tokens() {
        assert_eq!(serde_json::to_string(&Arch::X86_64).unwrap(), "\"x86_64\"");
        assert_eq!(
            serde_json::to_string(&Arch::Aarch64).unwrap(),
            "\"aarch64\""
        );
    }

    #[test]
    fn harness_serialises_with_pinned_kebab_tokens() {
        // Existing tokens — byte-stable pins must not drift.
        assert_eq!(
            serde_json::to_string(&Harness::ClaudeCode).unwrap(),
            "\"claude-code\""
        );
        assert_eq!(
            serde_json::to_string(&Harness::GeminiCli).unwrap(),
            "\"gemini-cli\""
        );
        // Phase 11 additions — `kebab-case` must render the exact harness ids.
        assert_eq!(
            serde_json::to_string(&Harness::CopilotCli).unwrap(),
            "\"copilot-cli\""
        );
        assert_eq!(
            serde_json::to_string(&Harness::JetbrainsAi).unwrap(),
            "\"jetbrains-ai\""
        );
        assert_eq!(
            serde_json::to_string(&Harness::GenericOp).unwrap(),
            "\"generic-op\""
        );
        assert_eq!(serde_json::to_string(&Harness::Pi).unwrap(), "\"pi\"");
        assert_eq!(serde_json::to_string(&Harness::Goose).unwrap(), "\"goose\"");
    }

    #[test]
    fn current_target_resolves_to_a_known_value() {
        // Compiles + runs only on the supported matrix (the compile-error guard
        // enforces that), so these are always among the mapped variants.
        assert!(matches!(CURRENT_OS, Os::Macos | Os::Linux));
        assert!(matches!(CURRENT_ARCH, Arch::X86_64 | Arch::Aarch64));
    }

    #[test]
    fn uuid_mint_round_trips_through_parse() {
        let u = Uuid::mint().expect("OS RNG available in tests");
        let reparsed = Uuid::parse(u.as_str()).expect("a freshly minted uuid must parse");
        assert_eq!(u, reparsed);
        assert_eq!(u.as_str(), reparsed.as_str());
    }

    #[test]
    fn uuid_mint_sets_version_and_variant() {
        let u = Uuid::mint().expect("OS RNG available in tests");
        let s = u.as_str();
        // Version nibble at index 14 must be '4'.
        assert_eq!(s.as_bytes()[14], b'4', "version nibble must be 4: {s}");
        // Variant nibble at index 19 must be one of 8/9/a/b.
        assert!(
            matches!(s.as_bytes()[19], b'8' | b'9' | b'a' | b'b'),
            "variant nibble must be 8/9/a/b: {s}"
        );
    }

    #[test]
    fn uuid_parse_rejects_malformed() {
        // Too short.
        assert!(Uuid::parse("not-a-uuid").is_none());
        // Right length but wrong version nibble (3 instead of 4).
        assert!(Uuid::parse("0b9c1f2e-3a4d-3b6c-8e1f-2a3b4c5d6e7f").is_none());
        // Right length but wrong variant nibble (c).
        assert!(Uuid::parse("0b9c1f2e-3a4d-4b6c-ce1f-2a3b4c5d6e7f").is_none());
        // Uppercase hex (we require lowercase to match mint output).
        assert!(Uuid::parse("0B9C1F2E-3A4D-4B6C-8E1F-2A3B4C5D6E7F").is_none());
        // Missing a hyphen.
        assert!(Uuid::parse("0b9c1f2e3a4d-4b6c-8e1f-2a3b4c5d6e7f").is_none());
        // A known-good value parses.
        assert!(Uuid::parse("0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f").is_some());
    }

    #[test]
    fn install_line_matches_data_model_worked_example() {
        let envelope = fixed_envelope_for_tests(Install::EVENT_TYPE);
        let event = Install {
            install_method: InstallMethod::Brew,
        };
        let line = to_line(&envelope, &event).unwrap();

        // Pinned byte-for-byte against `specs/010-phase-10-telemetry/data-model.md`
        // §10 (the seed of the later TELEMETRY.md pin). The worked example uses
        // `tome_version` "0.6.0", which is the crate version this builds against.
        let expected = "{\"schema_version\":1,\"install_uuid\":\"0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f\",\"session_uuid\":\"7f6e5d4c-3b2a-4f1e-9c8b-1a2b3c4d5e6f\",\"tome_version\":\"0.6.0\",\"os\":\"macos\",\"arch\":\"aarch64\",\"timestamp\":\"2026-06-11T14:11:45.123Z\",\"event_type\":\"tome.install\",\"sample_rate\":1.0,\"install_method\":\"brew\"}";
        assert_eq!(line, expected);
    }

    #[test]
    fn attributed_entry_invoked_matches_data_model_worked_example_2() {
        let envelope =
            fixed_attributed_envelope_for_tests("catalog.midnight.entry_invoked".to_string());
        let event = AttributedEntryInvoked {
            entry_name: "midnight-compact-debug".to_string(),
            entry_kind: EntryKind::Skill,
            plugin_name: "midnight-expert".to_string(),
            plugin_version: "1.2.0".to_string(),
            catalog_id: "midnight",
            calling_harness: Some(Harness::ClaudeCode),
        };
        let line = to_line(&envelope, &event).unwrap();

        // Pinned byte-for-byte against `specs/010-phase-10-telemetry/data-model.md`
        // §10 worked example 2. Note: NO `sample_rate` field (attributed events
        // are never sampled, FR-058).
        let expected = "{\"schema_version\":1,\"install_uuid\":\"0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f\",\"session_uuid\":\"7f6e5d4c-3b2a-4f1e-9c8b-1a2b3c4d5e6f\",\"tome_version\":\"0.6.0\",\"os\":\"macos\",\"arch\":\"aarch64\",\"timestamp\":\"2026-06-11T14:12:03.456Z\",\"event_type\":\"catalog.midnight.entry_invoked\",\"entry_name\":\"midnight-compact-debug\",\"entry_kind\":\"skill\",\"plugin_name\":\"midnight-expert\",\"plugin_version\":\"1.2.0\",\"catalog_id\":\"midnight\",\"calling_harness\":\"claude-code\"}";
        assert_eq!(line, expected);
    }

    #[test]
    fn format_rfc3339_millis_pins_the_worked_example() {
        use time::{Date, Month, Time};
        // 2026-06-11 14:11:45.123 UTC ⇒ exactly the data-model worked example.
        let dt = Date::from_calendar_date(2026, Month::June, 11)
            .unwrap()
            .with_time(Time::from_hms_milli(14, 11, 45, 123).unwrap())
            .assume_utc();
        assert_eq!(format_rfc3339_millis(dt), "2026-06-11T14:11:45.123Z");
    }

    #[test]
    fn format_rfc3339_millis_truncates_sub_millisecond() {
        use time::{Date, Month, Time};
        // .123999 (123 ms + 999 µs) must TRUNCATE to `.123`, not round to `.124`.
        let dt = Date::from_calendar_date(2026, Month::June, 11)
            .unwrap()
            .with_time(Time::from_hms_nano(14, 11, 45, 123_999_000).unwrap())
            .assume_utc();
        assert_eq!(format_rfc3339_millis(dt), "2026-06-11T14:11:45.123Z");
    }

    #[test]
    fn format_rfc3339_millis_pads_zero_millis_and_components() {
        use time::{Date, Month, Time};
        // Single-digit month/day/h/m/s and zero subseconds ⇒ all zero-padded,
        // `.000`, literal `Z`.
        let dt = Date::from_calendar_date(2026, Month::January, 5)
            .unwrap()
            .with_time(Time::from_hms(3, 7, 9).unwrap())
            .assume_utc();
        assert_eq!(format_rfc3339_millis(dt), "2026-01-05T03:07:09.000Z");
    }

    #[test]
    fn format_rfc3339_millis_normalises_non_utc_to_z() {
        use time::{Date, Month, Time, UtcOffset};
        // An input at +02:00 must render as the equivalent UTC instant with `Z`.
        let dt = Date::from_calendar_date(2026, Month::June, 11)
            .unwrap()
            .with_time(Time::from_hms_milli(16, 11, 45, 123).unwrap())
            .assume_offset(UtcOffset::from_hms(2, 0, 0).unwrap());
        assert_eq!(format_rfc3339_millis(dt), "2026-06-11T14:11:45.123Z");
    }

    #[test]
    fn provider_kind_serialises_lowercase_closed_tokens() {
        // Closed enum — exactly these five wire tokens, never a free-form string.
        assert_eq!(
            serde_json::to_string(&ProviderKind::Bundled).unwrap(),
            "\"bundled\""
        );
        assert_eq!(
            serde_json::to_string(&ProviderKind::Openai).unwrap(),
            "\"openai\""
        );
        assert_eq!(
            serde_json::to_string(&ProviderKind::Anthropic).unwrap(),
            "\"anthropic\""
        );
        assert_eq!(
            serde_json::to_string(&ProviderKind::Gemini).unwrap(),
            "\"gemini\""
        );
        assert_eq!(
            serde_json::to_string(&ProviderKind::Voyage).unwrap(),
            "\"voyage\""
        );
    }

    #[test]
    fn config_provider_kind_bridges_to_telemetry_kind() {
        use crate::config::ProviderKind as Cfg;
        assert_eq!(ProviderKind::from(Cfg::Openai), ProviderKind::Openai);
        assert_eq!(ProviderKind::from(Cfg::Anthropic), ProviderKind::Anthropic);
        assert_eq!(ProviderKind::from(Cfg::Gemini), ProviderKind::Gemini);
        assert_eq!(ProviderKind::from(Cfg::Voyage), ProviderKind::Voyage);
    }

    #[test]
    fn for_reranker_maps_configured_kind_and_defaults_bundled() {
        use crate::config::{Config, ProviderEntry, ProviderKind as Cfg};

        // No `[reranker]` provider → Bundled.
        let bare = Config::default();
        assert_eq!(ProviderKind::for_reranker(&bare), ProviderKind::Bundled);

        // A configured Voyage `[reranker]` → Voyage.
        let mut config = Config::default();
        config.providers.insert(
            "vp".to_string(),
            ProviderEntry {
                kind: Cfg::Voyage,
                base_url: None,
                api_key: None,
            },
        );
        config.reranker.provider = Some("vp".to_string());
        config.reranker.model = Some("rerank-2".to_string());
        assert_eq!(ProviderKind::for_reranker(&config), ProviderKind::Voyage);

        // An UNRESOLVABLE reference (name not in [providers]) degrades to Bundled
        // — telemetry never propagates a config error.
        let mut dangling = Config::default();
        dangling.reranker.provider = Some("ghost".to_string());
        assert_eq!(ProviderKind::for_reranker(&dangling), ProviderKind::Bundled);
    }

    #[test]
    fn summary_event_wire_shape_is_pinned() {
        // Byte-stable pin: closed enum tokens only, never a provider name/model.
        let envelope = fixed_envelope_for_tests(Summary::EVENT_TYPE);
        let event = Summary {
            summariser_provider_kind: ProviderKind::Openai,
            outcome: Outcome::Ok,
        };
        let line = to_line(&envelope, &event).unwrap();
        let expected = "{\"schema_version\":1,\"install_uuid\":\"0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f\",\"session_uuid\":\"7f6e5d4c-3b2a-4f1e-9c8b-1a2b3c4d5e6f\",\"tome_version\":\"0.6.0\",\"os\":\"macos\",\"arch\":\"aarch64\",\"timestamp\":\"2026-06-11T14:11:45.123Z\",\"event_type\":\"tome.summary\",\"sample_rate\":1.0,\"summariser_provider_kind\":\"openai\",\"outcome\":\"ok\"}";
        assert_eq!(line, expected);
    }

    #[test]
    fn optional_event_fields_are_skipped_when_none() {
        // A `Search` with both optionals `None` must omit those keys entirely.
        let envelope = fixed_envelope_for_tests(Search::EVENT_TYPE);
        let event = Search {
            surface: Surface::Cli,
            latency_bucket: LatencyBucket::Under50,
            candidates_returned: CountBucket::OneToFour,
            reranker_used: true,
            strict: false,
            corpus_size_bucket: CountBucket::FiveToNineteen,
            embedder_model_id: None,
            embedding_provider_kind: ProviderKind::Bundled,
            reranker_provider_kind: ProviderKind::Bundled,
            calling_harness: None,
        };
        let line = to_line(&envelope, &event).unwrap();
        assert!(!line.contains("embedder_model_id"));
        assert!(!line.contains("calling_harness"));
        // `embedding_provider_kind` is always serialised (no `skip`).
        assert!(line.contains("\"embedding_provider_kind\":\"bundled\""));
        // `reranker_provider_kind` is always serialised (no `skip`).
        assert!(line.contains("\"reranker_provider_kind\":\"bundled\""));
        // And present when `Some`.
        let event2 = Search {
            calling_harness: Some(Harness::ClaudeCode),
            ..event
        };
        let line2 = to_line(&envelope, &event2).unwrap();
        assert!(line2.contains("\"calling_harness\":\"claude-code\""));
    }
}

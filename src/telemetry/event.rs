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
    pub fn mint() -> Uuid {
        let mut buf = [0u8; 16];
        // `getrandom::fill` only errs if the OS RNG is unavailable, which on our
        // supported matrix is a non-recoverable environment fault — there is no
        // sensible fallback for a unique id, so we surface it as a panic. This
        // path runs once per install (id mint) / once per process (session),
        // never in a hot loop.
        getrandom::fill(&mut buf).expect("OS RNG unavailable while minting telemetry UUID");

        // RFC 4122: high nibble of byte 6 = version (4); top two bits of byte 8
        // = variant (0b10).
        buf[6] = (buf[6] & 0x0f) | 0x40;
        buf[8] = (buf[8] & 0x3f) | 0x80;

        Uuid(render_hyphenated(&buf))
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
    event_type: &'static str,
    sample_rate: f32,
}

impl Envelope {
    /// Build an envelope from the injectable pieces; fills the constants
    /// (`schema_version = 1`, `tome_version = env!(CARGO_PKG_VERSION)`,
    /// `sample_rate = 1.0`).
    pub fn new(
        install_uuid: Uuid,
        session_uuid: Uuid,
        os: Os,
        arch: Arch,
        timestamp: String,
        event_type: &'static str,
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
            sample_rate: 1.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Closed enums (event-specific dimensions)
// ---------------------------------------------------------------------------

/// Which agentic harness an action concerns / originates from. Serializes kebab
/// so the wire tokens match the harness ids used everywhere else in Tome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Cursor,
    Codex,
    Opencode,
    GeminiCli,
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
impl AnonymousEvent for ErrorEvent {
    const EVENT_TYPE: &'static str = "tome.error";
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

/// The ONE canonical fixed [`Envelope`] for byte-stable pin tests across the
/// suite (data-model §10). The `TELEMETRY.md` pin (US5) and the
/// `telemetry_events.rs` wire pins (US2) all build their expected lines off
/// this so the fixed install/session uuids, os/arch, timestamp, and sample
/// rate live in exactly one place — change them here and every pin updates in
/// lockstep.
///
/// Doc-hidden: it constructs from arbitrary (test-only) fixed uuids and is not
/// part of the published API. `tome_version` is NOT fixed here — it comes from
/// `env!("CARGO_PKG_VERSION")` via [`Envelope::new`], so a version bump updates
/// every pin together (matching the existing data-model worked example).
#[doc(hidden)]
pub fn fixed_envelope_for_tests(event_type: &'static str) -> Envelope {
    Envelope::new(
        Uuid::parse("0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f")
            .expect("canonical fixed install uuid is valid"),
        Uuid::parse("7f6e5d4c-3b2a-4f1e-9c8b-1a2b3c4d5e6f")
            .expect("canonical fixed session uuid is valid"),
        Os::Macos,
        Arch::Aarch64,
        "2026-06-11T14:11:45.123Z".to_string(),
        event_type,
    )
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
    fn current_target_resolves_to_a_known_value() {
        // Compiles + runs only on the supported matrix (the compile-error guard
        // enforces that), so these are always among the mapped variants.
        assert!(matches!(CURRENT_OS, Os::Macos | Os::Linux));
        assert!(matches!(CURRENT_ARCH, Arch::X86_64 | Arch::Aarch64));
    }

    #[test]
    fn uuid_mint_round_trips_through_parse() {
        let u = Uuid::mint();
        let reparsed = Uuid::parse(u.as_str()).expect("a freshly minted uuid must parse");
        assert_eq!(u, reparsed);
        assert_eq!(u.as_str(), reparsed.as_str());
    }

    #[test]
    fn uuid_mint_sets_version_and_variant() {
        let u = Uuid::mint();
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
            calling_harness: None,
        };
        let line = to_line(&envelope, &event).unwrap();
        assert!(!line.contains("embedder_model_id"));
        assert!(!line.contains("calling_harness"));
        // And present when `Some`.
        let event2 = Search {
            calling_harness: Some(Harness::ClaudeCode),
            ..event
        };
        let line2 = to_line(&envelope, &event2).unwrap();
        assert!(line2.contains("\"calling_harness\":\"claude-code\""));
    }
}

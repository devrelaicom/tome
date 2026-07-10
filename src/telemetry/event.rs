//! Typed, emit-only telemetry event records (re-homed onto the
//! `gauge-telemetry` kernel).
//!
//! Each struct here implements [`gauge_telemetry::event::Event`] with a bare
//! `name()` (the kernel namespaces it `tome.<name>`). The kernel owns the wire
//! envelope (install/session id, os/arch, timestamp); these structs carry ONLY
//! the per-event fields. Quantities are raw integers (the kernel buckets at read
//! time); attribute values are scalars only — `gauge_telemetry::event::to_attributes`
//! rejects any non-scalar field, so a free-form structure can never reach the
//! wire. There is no `deny_unknown_fields` (that is reserved for *inputs*; these
//! are outputs).
//!
//! Closed enums are preserved verbatim — the privacy guarantee is that every
//! field is a closed enum / bounded artefact name / number / bool, never a
//! free-form string except the documented attributed-name carve-out below.

use std::borrow::Cow;

use gauge_telemetry::env::EnvAttributes;
use gauge_telemetry::event::Event;
use serde::Serialize;

/// The host operating system, as a closed enum. `Windows` is a RESERVED value:
/// no runtime target on our build matrix maps to it today, but it stays in the
/// enum so a future Windows port serialises a known token rather than a junk
/// string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Os {
    Macos,
    Linux,
    Windows,
}

/// The host CPU architecture, as a closed enum. The per-variant renames pin the
/// wire tokens exactly (`x86_64`/`aarch64`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Arch {
    #[serde(rename = "x86_64")]
    X86_64,
    #[serde(rename = "aarch64")]
    Aarch64,
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

impl Harness {
    /// The snake_case-ish kebab wire token for a [`Harness`], matching exactly
    /// what its `Serialize` produces. Used for the comma-joined
    /// `harnesses_detected` heartbeat field (the kernel rejects arrays) and as a
    /// deterministic sort key. Hand-written so the order/value are legible and
    /// independent of the serializer; the
    /// `harness_serialises_with_pinned_kebab_tokens` test pins both in lockstep.
    pub(crate) fn as_wire_token(&self) -> &'static str {
        match self {
            Harness::ClaudeCode => "claude-code",
            Harness::Cursor => "cursor",
            Harness::Codex => "codex",
            Harness::Opencode => "opencode",
            Harness::GeminiCli => "gemini-cli",
            Harness::CopilotCli => "copilot-cli",
            Harness::Copilot => "copilot",
            Harness::Devin => "devin",
            Harness::Cline => "cline",
            Harness::Junie => "junie",
            Harness::JetbrainsAi => "jetbrains-ai",
            Harness::Antigravity => "antigravity",
            Harness::Pi => "pi",
            Harness::Crush => "crush",
            Harness::Zed => "zed",
            Harness::Kiro => "kiro",
            Harness::Generic => "generic",
            Harness::GenericOp => "generic-op",
            Harness::Goose => "goose",
        }
    }
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
/// NEVER carry a free-form string — the typed-event privacy guarantee.
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
    /// the kind — never the provider name / model / `base_url`. A
    /// missing/unresolvable reference degrades to `Bundled`; telemetry never
    /// propagates a config error.
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
    /// diverge (FR-022). Records ONLY the kind, never the provider name / model
    /// / `base_url`. A missing/unresolvable reference degrades to `Bundled`.
    pub fn for_reranker(cfg: &crate::config::Config) -> Self {
        let Some(name) = cfg.reranker.provider.as_deref() else {
            return ProviderKind::Bundled;
        };
        match cfg.providers.get(name) {
            Some(entry) => ProviderKind::from(entry.kind),
            None => ProviderKind::Bundled,
        }
    }

    /// Map the configured SUMMARISER provider to the closed telemetry kind:
    /// `Bundled` when no `[summariser]` provider is referenced (or the reference
    /// can't be resolved to a registry entry), else the entry's kind. This is the
    /// SSOT the summary emit site (`workspace::regen_summary`) calls so it can
    /// never diverge from the `[embedding]`/`[reranker]` mappers above. Records
    /// ONLY the kind — never the provider name / model / `base_url`. A
    /// missing/unresolvable reference degrades to `Bundled`.
    pub fn for_summariser(cfg: &crate::config::Config) -> Self {
        let Some(name) = cfg.summariser.provider.as_deref() else {
            return ProviderKind::Bundled;
        };
        match cfg.providers.get(name) {
            Some(entry) => ProviderKind::from(entry.kind),
            None => ProviderKind::Bundled,
        }
    }
}

// ---------------------------------------------------------------------------
// Anonymous (`tome.*`) events
// ---------------------------------------------------------------------------

/// `tome.install` — emitted once when the install id is first minted. Flattens
/// the kernel's environment snapshot (`os_version`/`cpu_cores`/`ram_gb`/`accel`/…)
/// and keeps the closed [`InstallMethod`] token.
#[derive(Debug, Clone, Serialize)]
pub struct Install {
    pub install_method: InstallMethod,
    #[serde(flatten)]
    pub env: EnvAttributes,
}
impl Event for Install {
    fn name(&self) -> Cow<'_, str> {
        "install".into()
    }
}

/// `tome.upgrade` — emitted when `last-version` differs from the running binary.
/// `from_version` is Tome's OWN prior version (read from the `0600` `last-version`
/// stamp file Tome itself wrote — never user input).
#[derive(Debug, Clone, Serialize)]
pub struct Upgrade {
    pub from_version: String,
}
impl Event for Upgrade {
    fn name(&self) -> Cow<'_, str> {
        "upgrade".into()
    }
}

/// `tome.heartbeat` — once-per-UTC-day inventory snapshot. Counts are raw ints;
/// `harnesses_detected` is a sorted, comma-joined closed-vocabulary string (the
/// kernel rejects arrays). Flattens the kernel environment snapshot.
#[derive(Debug, Clone, Serialize)]
pub struct Heartbeat {
    pub skills: u32,
    pub commands: u32,
    pub agents: u32,
    pub workspaces: u32,
    pub catalogs: u32,
    pub harnesses_detected: String,
    #[serde(flatten)]
    pub env: EnvAttributes,
}
impl Event for Heartbeat {
    fn name(&self) -> Cow<'_, str> {
        "heartbeat".into()
    }
}

/// `tome.search` — one semantic search round-trip.
#[derive(Debug, Clone, Serialize)]
pub struct Search {
    pub surface: Surface,
    pub latency_ms: u32,
    pub candidates_returned: u32,
    pub reranker_used: bool,
    pub strict: bool,
    pub corpus_size: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedder_model_id: Option<&'static str>,
    /// Phase 12 — which PROVIDER served the embedding for this search, as the
    /// closed [`ProviderKind`] (`Bundled` when no `[embedding]` provider is
    /// configured). NEVER the provider name / model id / `base_url`.
    pub embedding_provider_kind: ProviderKind,
    /// Phase 12 / US3 — which PROVIDER served the RERANKING for this search.
    /// Independent of `embedding_provider_kind` (FR-022).
    pub reranker_provider_kind: ProviderKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}
impl Event for Search {
    fn name(&self) -> Cow<'_, str> {
        "search".into()
    }
}

/// `tome.entry_info` — a metadata-only `get_skill` lookup (`metadata_only:
/// true`, formerly the `get_skill_info` tool). `rank` is this entry's exact
/// 1-indexed position in the preceding search this session (`0` ⇒ no rank).
#[derive(Debug, Clone, Serialize)]
pub struct EntryInfo {
    pub rank: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}
impl Event for EntryInfo {
    fn name(&self) -> Cow<'_, str> {
        "entry_info".into()
    }
}

/// `tome.entry_invoked` — an entry body was fetched/invoked.
#[derive(Debug, Clone, Serialize)]
pub struct EntryInvoked {
    pub entry_kind: EntryKind,
    pub rank: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}
impl Event for EntryInvoked {
    fn name(&self) -> Cow<'_, str> {
        "entry_invoked".into()
    }
}

/// `tome.prompt_invoked` — an MCP prompt was invoked.
#[derive(Debug, Clone, Serialize)]
pub struct PromptInvoked {
    pub prompt_kind: PromptKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}
impl Event for PromptInvoked {
    fn name(&self) -> Cow<'_, str> {
        "prompt_invoked".into()
    }
}

/// `tome.catalog_action` — catalog add/remove/update.
#[derive(Debug, Clone, Serialize)]
pub struct CatalogActionEvent {
    pub action: CatalogAction,
    pub source_type: SourceType,
}
impl Event for CatalogActionEvent {
    fn name(&self) -> Cow<'_, str> {
        "catalog_action".into()
    }
}

/// `tome.plugin_action` — plugin enable/disable.
#[derive(Debug, Clone, Serialize)]
pub struct PluginActionEvent {
    pub action: PluginAction,
}
impl Event for PluginActionEvent {
    fn name(&self) -> Cow<'_, str> {
        "plugin_action".into()
    }
}

/// `tome.workspace_action` — workspace lifecycle verb.
#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceActionEvent {
    pub action: WorkspaceAction,
}
impl Event for WorkspaceActionEvent {
    fn name(&self) -> Cow<'_, str> {
        "workspace_action".into()
    }
}

/// `tome.harness_action` — harness use/sync/remove.
#[derive(Debug, Clone, Serialize)]
pub struct HarnessActionEvent {
    pub action: HarnessAction,
    pub harness: Harness,
}
impl Event for HarnessActionEvent {
    fn name(&self) -> Cow<'_, str> {
        "harness_action".into()
    }
}

/// `tome.authoring_action` — a create/convert/lint run.
#[derive(Debug, Clone, Serialize)]
pub struct AuthoringActionEvent {
    pub verb: AuthoringVerb,
    pub artifact: Artifact,
    pub source_format: SourceFormat,
    pub outcome: AuthoringOutcome,
}
impl Event for AuthoringActionEvent {
    fn name(&self) -> Cow<'_, str> {
        "authoring_action".into()
    }
}

/// `tome.meta_action` — a meta-skill add/remove/fix.
#[derive(Debug, Clone, Serialize)]
pub struct MetaActionEvent {
    pub action: MetaAction,
    pub outcome: Outcome,
}
impl Event for MetaActionEvent {
    fn name(&self) -> Cow<'_, str> {
        "meta_action".into()
    }
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
impl Event for ModelDownload {
    fn name(&self) -> Cow<'_, str> {
        "model_download".into()
    }
}

/// `tome.cold_start` — process-start embedder/index readiness timings (ms).
#[derive(Debug, Clone, Serialize)]
pub struct ColdStart {
    pub embedder_load_ms: u32,
    pub index_ready_ms: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedder_model_id: Option<&'static str>,
}
impl Event for ColdStart {
    fn name(&self) -> Cow<'_, str> {
        "cold_start".into()
    }
}

/// `tome.doctor_run` — a `tome doctor` invocation.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorRun {
    pub fix: bool,
    pub findings: u32,
}
impl Event for DoctorRun {
    fn name(&self) -> Cow<'_, str> {
        "doctor_run".into()
    }
}

/// `tome.reindex` — a `tome reindex` invocation.
#[derive(Debug, Clone, Serialize)]
pub struct Reindex {
    pub scope: ReindexScope,
    pub forced: bool,
    pub outcome: Outcome,
}
impl Event for Reindex {
    fn name(&self) -> Cow<'_, str> {
        "reindex".into()
    }
}

/// `tome.summary` — a workspace summary was (re)generated. Records WHICH provider
/// kind served the summariser (Phase 12) plus the outcome. NO provider name /
/// model id / `base_url` is recorded — only the closed [`ProviderKind`].
#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub summariser_provider_kind: ProviderKind,
    pub outcome: Outcome,
}
impl Event for Summary {
    fn name(&self) -> Cow<'_, str> {
        "summary".into()
    }
}

/// `tome.error` — a classified error surfaced to a user-facing command.
#[derive(Debug, Clone, Serialize)]
pub struct ErrorEvent {
    pub error_class: crate::error::ErrorCategory,
    pub surface: Surface,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}
impl Event for ErrorEvent {
    fn name(&self) -> Cow<'_, str> {
        "error".into()
    }
}

// ---------------------------------------------------------------------------
// Catalog-attributed events (`tome.catalog_*`) — the ONLY bounded-`String`
// carve-out (FR-059)
// ---------------------------------------------------------------------------
//
// THE SINGLE EXCEPTION to the "no free-form `String` in an event field" rule:
// the structs below carry PUBLISHED artefact names + versions (`plugin_name`,
// `entry_name`, `plugin_version`, …) as bounded `String`s. These are not user
// secrets and not a fingerprint — they are the names a maintainer already
// published to a public catalog, and they are emitted ONLY for an allowlisted
// catalog (see `allowlist.rs`). The `catalog` field is the resolved allowlist
// short id (was `catalog_id` pre-kernel — now serialized as the `catalog`
// attribute). Every other event type in this module is raw ints / closed enums.
// If you are adding a `String` field ANYWHERE ELSE in this file, stop: it almost
// certainly belongs in a closed enum or a bucket instead.

/// `tome.catalog_plugin_enabled` — an allowlisted-catalog plugin was enabled.
#[derive(Debug, Clone, Serialize)]
pub struct PluginEnabled {
    pub catalog: &'static str,
    pub plugin_name: String,
    pub plugin_version: String,
}
impl Event for PluginEnabled {
    fn name(&self) -> Cow<'_, str> {
        "catalog_plugin_enabled".into()
    }
}

/// `tome.catalog_plugin_disabled` — an allowlisted-catalog plugin was disabled.
#[derive(Debug, Clone, Serialize)]
pub struct PluginDisabled {
    pub catalog: &'static str,
    pub plugin_name: String,
    pub plugin_version: String,
}
impl Event for PluginDisabled {
    fn name(&self) -> Cow<'_, str> {
        "catalog_plugin_disabled".into()
    }
}

/// `tome.catalog_plugin_updated` — per plugin whose version changed during a
/// `catalog update` of an allowlisted catalog.
#[derive(Debug, Clone, Serialize)]
pub struct PluginUpdated {
    pub catalog: &'static str,
    pub plugin_name: String,
    pub from_version: String,
    pub to_version: String,
}
impl Event for PluginUpdated {
    fn name(&self) -> Cow<'_, str> {
        "catalog_plugin_updated".into()
    }
}

/// `tome.catalog_entry_invoked` — an allowlisted-catalog entry body was invoked.
#[derive(Debug, Clone, Serialize)]
pub struct AttributedEntryInvoked {
    pub catalog: &'static str,
    pub entry_name: String,
    pub entry_kind: EntryKind,
    pub plugin_name: String,
    pub plugin_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}
impl Event for AttributedEntryInvoked {
    fn name(&self) -> Cow<'_, str> {
        "catalog_entry_invoked".into()
    }
}

/// `tome.catalog_search_result` — fires once per allowlisted-catalog entry that
/// appears in a result. `rank` is EXACT (FR-057), not bucketed: selection
/// attribution is a server-side join against the later `entry_invoked`.
#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub catalog: &'static str,
    pub entry_name: String,
    pub entry_kind: EntryKind,
    pub plugin_name: String,
    pub rank: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub calling_harness: Option<Harness>,
}
impl Event for SearchResult {
    fn name(&self) -> Cow<'_, str> {
        "catalog_search_result".into()
    }
}

/// `tome.catalog_error` — a classified error involving an allowlisted-catalog
/// plugin. `entry_name` is optional (some errors are plugin-level, not entry).
#[derive(Debug, Clone, Serialize)]
pub struct AttributedError {
    pub catalog: &'static str,
    pub plugin_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_name: Option<String>,
    pub error_class: crate::error::ErrorCategory,
    pub plugin_version: String,
}
impl Event for AttributedError {
    fn name(&self) -> Cow<'_, str> {
        "catalog_error".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gauge_telemetry::event::to_attributes;
    use serde_json::Value;

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
    fn harness_wire_token_matches_serialize() {
        // The hand-written `as_wire_token` must agree byte-for-byte with serde
        // (it is the `harnesses_detected` comma-join + sort-key source of truth).
        for h in [
            Harness::ClaudeCode,
            Harness::Cursor,
            Harness::Codex,
            Harness::Opencode,
            Harness::GeminiCli,
            Harness::CopilotCli,
            Harness::Copilot,
            Harness::Devin,
            Harness::Cline,
            Harness::Junie,
            Harness::JetbrainsAi,
            Harness::Antigravity,
            Harness::Pi,
            Harness::Crush,
            Harness::Zed,
            Harness::Kiro,
            Harness::Generic,
            Harness::GenericOp,
            Harness::Goose,
        ] {
            let serde_token = serde_json::to_value(h).unwrap();
            assert_eq!(serde_token, Value::String(h.as_wire_token().to_string()));
        }
    }

    #[test]
    fn search_serializes_to_flat_scalars_with_raw_ints() {
        let e = Search {
            surface: Surface::Cli,
            latency_ms: 142,
            candidates_returned: 7,
            reranker_used: true,
            strict: false,
            corpus_size: 1234,
            embedder_model_id: None,
            embedding_provider_kind: ProviderKind::Bundled,
            reranker_provider_kind: ProviderKind::Bundled,
            calling_harness: None,
        };
        assert_eq!(e.name(), "search");
        let a = to_attributes(&e).unwrap();
        assert_eq!(a["latency_ms"], Value::Number(142u32.into()));
        assert_eq!(a["candidates_returned"], Value::Number(7u32.into()));
        assert_eq!(a["corpus_size"], Value::Number(1234u32.into()));
        assert_eq!(a["surface"], Value::String("cli".into()));
        assert_eq!(
            a["embedding_provider_kind"],
            Value::String("bundled".into())
        );
        // None optionals are omitted (no `null`, which the kernel rejects).
        assert!(!a.contains_key("embedder_model_id"));
        assert!(!a.contains_key("calling_harness"));
    }

    #[test]
    fn attributed_entry_invoked_carries_catalog_attribute() {
        let e = AttributedEntryInvoked {
            catalog: "midnight",
            entry_name: "midnight-compact-debug".into(),
            entry_kind: EntryKind::Skill,
            plugin_name: "midnight-expert".into(),
            plugin_version: "1.2.0".into(),
            calling_harness: None,
        };
        assert_eq!(e.name(), "catalog_entry_invoked");
        let a = to_attributes(&e).unwrap();
        assert_eq!(a["catalog"], Value::String("midnight".into()));
        assert_eq!(a["plugin_name"], Value::String("midnight-expert".into()));
        assert_eq!(a["entry_kind"], Value::String("skill".into()));
        // No `calling_harness` key when `None`.
        assert!(!a.contains_key("calling_harness"));
    }

    #[test]
    fn install_flattens_env_and_keeps_closed_method_token() {
        // A hand-built env snapshot flattens into the event's own attributes
        // (no nested object — `to_attributes` would reject a nested map).
        let env = EnvAttributes {
            cpu_cores: Some(8),
            ram_gb: Some(16),
            ..Default::default()
        };
        let e = Install {
            install_method: InstallMethod::Brew,
            env,
        };
        assert_eq!(e.name(), "install");
        let a = to_attributes(&e).unwrap();
        assert_eq!(a["install_method"], Value::String("brew".into()));
        assert_eq!(a["cpu_cores"], Value::Number(8u32.into()));
        assert_eq!(a["ram_gb"], Value::Number(16u32.into()));
    }

    #[test]
    fn heartbeat_harnesses_detected_is_a_flat_string() {
        let e = Heartbeat {
            skills: 3,
            commands: 1,
            agents: 0,
            workspaces: 2,
            catalogs: 1,
            harnesses_detected: "claude-code,cursor".into(),
            env: EnvAttributes::default(),
        };
        assert_eq!(e.name(), "heartbeat");
        let a = to_attributes(&e).unwrap();
        assert_eq!(a["skills"], Value::Number(3u32.into()));
        // Crucially a STRING, not an array (the kernel rejects arrays).
        assert_eq!(
            a["harnesses_detected"],
            Value::String("claude-code,cursor".into())
        );
    }

    #[test]
    fn provider_kind_serialises_lowercase_closed_tokens() {
        assert_eq!(
            serde_json::to_string(&ProviderKind::Bundled).unwrap(),
            "\"bundled\""
        );
        assert_eq!(
            serde_json::to_string(&ProviderKind::Openai).unwrap(),
            "\"openai\""
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

        let bare = Config::default();
        assert_eq!(ProviderKind::for_reranker(&bare), ProviderKind::Bundled);

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

        let mut dangling = Config::default();
        dangling.reranker.provider = Some("ghost".to_string());
        assert_eq!(ProviderKind::for_reranker(&dangling), ProviderKind::Bundled);
    }

    #[test]
    fn for_embedding_maps_configured_kind_and_defaults_bundled() {
        use crate::config::{Config, ProviderEntry, ProviderKind as Cfg};

        let bare = Config::default();
        assert_eq!(ProviderKind::for_embedding(&bare), ProviderKind::Bundled);

        let mut config = Config::default();
        config.providers.insert(
            "ep".to_string(),
            ProviderEntry {
                kind: Cfg::Openai,
                base_url: None,
                api_key: None,
            },
        );
        config.embedding.provider = Some("ep".to_string());
        config.embedding.model = Some("text-embedding-3-small".to_string());
        assert_eq!(ProviderKind::for_embedding(&config), ProviderKind::Openai);

        let mut dangling = Config::default();
        dangling.embedding.provider = Some("ghost".to_string());
        assert_eq!(
            ProviderKind::for_embedding(&dangling),
            ProviderKind::Bundled
        );
    }

    #[test]
    fn for_summariser_maps_configured_kind_and_defaults_bundled() {
        use crate::config::{Config, ProviderEntry, ProviderKind as Cfg};

        let bare = Config::default();
        assert_eq!(ProviderKind::for_summariser(&bare), ProviderKind::Bundled);

        let mut config = Config::default();
        config.providers.insert(
            "sp".to_string(),
            ProviderEntry {
                kind: Cfg::Anthropic,
                base_url: None,
                api_key: None,
            },
        );
        config.summariser.provider = Some("sp".to_string());
        config.summariser.model = Some("claude-haiku".to_string());
        assert_eq!(
            ProviderKind::for_summariser(&config),
            ProviderKind::Anthropic
        );

        let mut dangling = Config::default();
        dangling.summariser.provider = Some("ghost".to_string());
        assert_eq!(
            ProviderKind::for_summariser(&dangling),
            ProviderKind::Bundled
        );
    }
}

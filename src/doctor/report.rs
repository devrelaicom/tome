//! Serialisable types for `tome doctor`'s report. Data-model §5 / §6 / §15.
//!
//! Emit-only — these types are never deserialised, so no
//! `#[serde(deny_unknown_fields)]`. The wire JSON shape is contract
//! `contracts/doctor.md` + `contracts/doctor-extensions-p4.md`; an integration
//! test pins byte-stability.
//!
//! Phase 4 / US5.a promotes the previously-`String`-typed `subsystem` field
//! on [`SuggestedFix`] to the typed [`Subsystem`] enum (data-model §15).
//! Custom `Serialize` / `Deserialize` impls preserve the Phase 3 wire shape
//! byte-for-byte for every Phase 3 variant; new Phase 4 variants slot in
//! alongside without changing existing keys.

use std::fmt;
use std::path::PathBuf;

use serde::de::{self, Deserializer, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::commands::status::{IndexHealth, ModelHealth};
use crate::index::meta::DriftStatus;
use crate::settings::resolver::EffectiveHarnessList;
use crate::workspace::{WorkspaceInfo, WorkspaceName};

/// Three-state overall classification used by `tome doctor`. Matches the
/// shape of `OverallHealth` from Phase 2 status but lives here so the
/// doctor report's `overall` field is wire-distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DoctorClassification {
    Ok,
    Degraded,
    Unhealthy,
}

/// Per-catalog on-disk cache classification. The `state` field uses
/// snake_case so the JSON wire matches the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogCacheHealth {
    pub name: String,
    pub url: String,
    pub cache_path: PathBuf,
    pub state: CatalogCacheState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogCacheState {
    /// Directory exists, is a git repo, and the catalog manifest parses.
    Ok,
    /// Cache directory not on disk.
    Missing,
    /// Cache directory exists but lacks `.git/`.
    NotARepo,
    /// Cache + `.git/` present but `tome-catalog.toml` is missing or
    /// unparsable.
    ManifestInvalid,
    /// Cache directory exists, is a valid catalog clone, but no
    /// `config.toml` in the resolved scope references its URL. Created
    /// when a `tome catalog remove` left a sibling-scope reference
    /// behind, or when a registry edit dropped the entry without
    /// removing the clone. The orphan record is informational —
    /// `auto_fixable` is `false`; the user removes it by hand once
    /// they've verified nothing else needs the clone. Contract
    /// `catalog-extensions-p3.md` §"Doctor reporting" bullet 4.
    Orphan,
}

impl CatalogCacheState {
    pub fn as_str(self) -> &'static str {
        match self {
            CatalogCacheState::Ok => "ok",
            CatalogCacheState::Missing => "missing",
            CatalogCacheState::NotARepo => "not_a_repo",
            CatalogCacheState::ManifestInvalid => "manifest_invalid",
            CatalogCacheState::Orphan => "orphan",
        }
    }
}

/// Workspace-registry status block. Contract
/// `catalog-extensions-p3.md` §"Doctor reporting" calls for one line
/// summarising the opt-in registry file. `present = false` is the
/// default fresh-install state; `present = true` means the file is
/// opt-in-touched and `tracked` is the count of registered workspaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceRegistryStatus {
    pub present: bool,
    pub tracked: u32,
}

/// One probed agentic-coding harness. The well-known harness names are a
/// fixed list (research §R-7); the value of `present` is what doctor
/// actually checks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HarnessPresence {
    pub name: String,
    pub path: PathBuf,
    pub present: bool,
}

/// A user-actionable repair suggestion. `auto_fixable = true` items are
/// the classes `--fix` handles automatically; everything else is
/// surfaced as a copy-pasteable command.
///
/// The `subsystem` field is the typed [`Subsystem`] enum but serialises to
/// the documented colon-separated wire string so external `--json` consumers
/// see the Phase 3 byte shape for every Phase 3 variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuggestedFix {
    pub subsystem: Subsystem,
    pub diagnosis: String,
    pub command: String,
    pub auto_fixable: bool,
}

/// Per-subsystem health classification used by Phase 4's new doctor
/// surfaces (summariser + harness integration + binding). Mirrors the
/// shape of [`ModelHealth.state`] / [`CatalogCacheState`] but is the
/// single source of truth for the wire vocabulary across the new
/// subsystems. The variants are:
///
/// - `Ok` — subsystem is healthy.
/// - `Drift` — subsystem exists but its content differs from what Tome
///   would produce. Re-runnable by the corresponding `--fix` handler.
/// - `Broken` — subsystem is missing or unreadable. Re-runnable by
///   `--fix` for the auto-fixable variants.
/// - `UserOwned` — only emitted for `HarnessMcp`: the entry under the
///   `tome` key is developer-authored. `--fix` alone refuses to
///   overwrite; `--fix --force` (US5.b) does.
/// - `NotApplicable` — the subsystem isn't applicable in the current
///   context (e.g. harness subsystems when the effective list is empty
///   per FR-561). Does NOT affect overall classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SubsystemHealth {
    Ok,
    Drift,
    Broken,
    UserOwned,
    NotApplicable,
}

impl SubsystemHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            SubsystemHealth::Ok => "ok",
            SubsystemHealth::Drift => "drift",
            SubsystemHealth::Broken => "broken",
            SubsystemHealth::UserOwned => "user_owned",
            SubsystemHealth::NotApplicable => "not_applicable",
        }
    }
}

/// Per-project binding state per data-model §15. Populated by
/// [`crate::doctor::binding::check_binding`] when the resolved scope's
/// source is `ProjectMarker`; `None` otherwise (FR-564).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProjectBindingState {
    pub project_root: PathBuf,
    pub bound_workspace: WorkspaceName,
    pub config_well_formed: bool,
    pub rules_file_drift: RulesCopyState,
}

/// Per-project `.tome/RULES.md` drift classification. Computed by byte
/// comparison against `<root>/workspaces/<name>/RULES.md`.
///
/// US5 reviewer R-M5: `SourceMissing` distinguishes "workspace's
/// canonical RULES.md is absent" (the source-of-truth file at
/// `<root>/workspaces/<name>/RULES.md`) from `Missing` (the project's
/// copy at `<project>/.tome/RULES.md` is absent). The two cases have
/// different remediation paths — `--fix` for `SourceMissing` skips the
/// copy and surfaces "run `tome workspace regen-summary <name>` first"
/// rather than infinite-looping a `cp` of nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RulesCopyState {
    Match,
    Missing,
    Drift,
    SourceMissing,
}

/// Typed subsystem identifier replacing Phase 3's free-form `String`
/// field on [`SuggestedFix`]. The custom `Serialize` / `Deserialize`
/// impls map to the documented colon-separated strings:
///
/// - `Embedder` ↔ `"embedder"`
/// - `Reranker` ↔ `"reranker"`
/// - `Index` ↔ `"index"`
/// - `Drift` ↔ `"drift"`
/// - `Catalog(name)` ↔ `"catalog:<name>"`
/// - `Schema` ↔ `"schema"`
/// - `Summariser` ↔ `"summariser"`
/// - `Binding` ↔ `"binding"`
/// - `BindingRulesCopy` ↔ `"binding-rules-copy"`
/// - `HarnessRules(name)` ↔ `"harness-rules:<name>"`
/// - `HarnessMcp(name)` ↔ `"harness-mcp:<name>"`
///
/// The two Phase 3 drift "subsystems" `embedder_drift`, `reranker_drift`,
/// and the Phase 4 fold-in `summariser_drift` are not part of this enum:
/// they were never first-class subsystems, just descriptive subsystem
/// labels on drift-class suggestions. They land here under
/// `Subsystem::Drift` with the existing drift-specific message attached
/// (the diagnosis text discriminates between embedder/reranker/summariser
/// drift on the wire side).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Subsystem {
    Embedder,
    Reranker,
    Index,
    Drift,
    Catalog(String),
    Schema,
    Summariser,
    Binding,
    BindingRulesCopy,
    HarnessRules(String),
    HarnessMcp(String),
}

impl Subsystem {
    /// Render the wire string (one allocation; only callers that need
    /// the owned string should use this — comparisons use `PartialEq`
    /// against `&str` / `String` directly).
    pub fn to_wire_string(&self) -> String {
        match self {
            Subsystem::Embedder => "embedder".to_owned(),
            Subsystem::Reranker => "reranker".to_owned(),
            Subsystem::Index => "index".to_owned(),
            Subsystem::Drift => "drift".to_owned(),
            Subsystem::Catalog(n) => format!("catalog:{n}"),
            Subsystem::Schema => "schema".to_owned(),
            Subsystem::Summariser => "summariser".to_owned(),
            Subsystem::Binding => "binding".to_owned(),
            Subsystem::BindingRulesCopy => "binding-rules-copy".to_owned(),
            Subsystem::HarnessRules(n) => format!("harness-rules:{n}"),
            Subsystem::HarnessMcp(n) => format!("harness-mcp:{n}"),
        }
    }

    /// Parse a wire string back into a `Subsystem`. Returns `None` for
    /// any string that doesn't match the documented vocabulary.
    pub fn parse_wire(s: &str) -> Option<Self> {
        Some(match s {
            "embedder" => Subsystem::Embedder,
            "reranker" => Subsystem::Reranker,
            "index" => Subsystem::Index,
            "drift" => Subsystem::Drift,
            "schema" => Subsystem::Schema,
            "summariser" => Subsystem::Summariser,
            "binding" => Subsystem::Binding,
            "binding-rules-copy" => Subsystem::BindingRulesCopy,
            other => {
                if let Some(name) = other.strip_prefix("catalog:") {
                    Subsystem::Catalog(name.to_owned())
                } else if let Some(name) = other.strip_prefix("harness-rules:") {
                    Subsystem::HarnessRules(name.to_owned())
                } else if let Some(name) = other.strip_prefix("harness-mcp:") {
                    Subsystem::HarnessMcp(name.to_owned())
                } else {
                    return None;
                }
            }
        })
    }
}

impl fmt::Display for Subsystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_wire_string())
    }
}

/// Comparing a `Subsystem` against a borrowed `&str` matches the wire
/// shape — letting Phase 3 tests like `fix.subsystem == "embedder"` work
/// transparently after the type promotion. Callers that need stricter
/// matching can do `*subsystem == Subsystem::Embedder`.
impl PartialEq<str> for Subsystem {
    fn eq(&self, other: &str) -> bool {
        self.to_wire_string() == other
    }
}

impl PartialEq<&str> for Subsystem {
    fn eq(&self, other: &&str) -> bool {
        self.to_wire_string() == *other
    }
}

impl PartialEq<String> for Subsystem {
    fn eq(&self, other: &String) -> bool {
        self.to_wire_string() == *other
    }
}

impl Serialize for Subsystem {
    fn serialize<S: Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.to_wire_string())
    }
}

impl<'de> Deserialize<'de> for Subsystem {
    fn deserialize<D: Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Subsystem;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a Subsystem wire string per contracts/doctor-extensions-p4.md")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Subsystem, E> {
                Subsystem::parse_wire(v)
                    .ok_or_else(|| E::custom(format!("unknown Subsystem wire string `{v}`")))
            }
        }
        de.deserialize_str(V)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant of [`Subsystem`] survives a serialise → deserialise
    /// round-trip via its documented wire string. The Phase 3 variants
    /// MUST emit the byte-exact string they did before the typed
    /// promotion so external `--json` consumers don't observe a break.
    #[test]
    fn subsystem_round_trip_locks_wire_shape() {
        let cases = [
            (Subsystem::Embedder, "\"embedder\""),
            (Subsystem::Reranker, "\"reranker\""),
            (Subsystem::Index, "\"index\""),
            (Subsystem::Drift, "\"drift\""),
            (Subsystem::Catalog("name".into()), "\"catalog:name\""),
            (Subsystem::Schema, "\"schema\""),
            (Subsystem::Summariser, "\"summariser\""),
            (Subsystem::Binding, "\"binding\""),
            (Subsystem::BindingRulesCopy, "\"binding-rules-copy\""),
            (
                Subsystem::HarnessRules("claude-code".into()),
                "\"harness-rules:claude-code\"",
            ),
            (
                Subsystem::HarnessMcp("codex".into()),
                "\"harness-mcp:codex\"",
            ),
        ];
        for (variant, wire) in cases {
            let serialised = serde_json::to_string(&variant).unwrap();
            assert_eq!(serialised, wire, "wire shape for {variant:?}");
            let parsed: Subsystem = serde_json::from_str(wire).unwrap();
            assert_eq!(parsed, variant, "round-trip for {wire}");
        }
    }

    /// Deserialising an unknown wire string MUST fail rather than silently
    /// coerce to a default variant — typo'd subsystem names are a bug.
    #[test]
    fn subsystem_rejects_unknown_wire_string() {
        let err: Result<Subsystem, _> = serde_json::from_str("\"not-a-subsystem\"");
        assert!(err.is_err());
    }

    /// String-comparison shim: Phase 3 tests + dispatch sites compared
    /// against `&str` literals. The `PartialEq<&str>` / `PartialEq<str>`
    /// impls preserve that ergonomics through the type promotion.
    #[test]
    fn subsystem_compares_against_str_literals() {
        assert!(Subsystem::Embedder == "embedder");
        assert!(Subsystem::Catalog("foo".into()) == "catalog:foo");
        assert!(Subsystem::HarnessRules("cursor".into()) == "harness-rules:cursor");
        assert!(Subsystem::HarnessMcp("gemini".into()) != "harness-mcp:codex");
    }
}

/// Per-harness integration check result. Pair of `(harness_name, health)`
/// — used for both `harness_rules` and `harness_mcp` fields on
/// [`DoctorReport`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HarnessSubsystemReport {
    pub harness: String,
    pub health: SubsystemHealth,
}

// --- Phase 5 doctor extensions (US5.b) -----------------------------------
//
// `PromptsReport`, `OrphanDataDirReport`, and `EntryCountsByKind` are the
// three new sections added by Phase 5. They are all `Option` on
// [`DoctorReport`] so an outside-project / non-workspace doctor pass
// emits `null` for each (Phase 4 convention; preserves the byte-stable
// JSON shape of the existing `doctor_json_shape_is_byte_stable_for_minimal_report`
// pin when these fields are absent).

/// Phase 5 prompts surface — enumeration of every prompt the MCP server
/// would expose for the resolved workspace plus the collisions detected
/// during name resolution. Built via [`crate::mcp::prompts::PromptRegistry::build_for_workspace`]
/// so the doctor view matches what `tome mcp` would surface byte-for-byte.
///
/// `PartialEq` is intentionally omitted: rmcp's `Prompt` is `PartialEq`
/// but [`crate::mcp::prompt_collision::CollisionRecord`] is not, and
/// deriving `PartialEq` here would require a hand-rolled impl with no
/// caller. The serialised JSON shape is what tests pin.
#[derive(Debug, Clone, Serialize)]
pub struct PromptsReport {
    pub prompts: Vec<crate::mcp::prompts::PromptDescriptor>,
    pub collisions: Vec<crate::mcp::prompt_collision::CollisionRecord>,
}

/// Phase 5 orphan persistent-data-dir surface. Both fields are absolute
/// paths discovered on disk that have no matching `(workspace, catalog,
/// plugin)` enrolment. Informational only in Phase 5 (no `--fix` repair
/// handler; deferred to Phase 6+).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OrphanDataDirReport {
    pub plugin_data: Vec<std::path::PathBuf>,
    pub workspace_data: Vec<std::path::PathBuf>,
}

/// Phase 5 per-kind entry counts for the resolved workspace.
/// `pending_re_embedding` is a heuristic: counts enabled entries whose
/// source-file mtime is newer than the stored `indexed_at`. See
/// `contracts/doctor-extensions-p5.md` § `entry_counts` for the
/// false-positive caveats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EntryCountsByKind {
    pub skills: u32,
    pub commands: u32,
    /// Phase 6: agent-kind entries enrolled in the workspace. Always
    /// non-searchable; never embedded (entry-schema-p6.md).
    pub agents: u32,
    pub pending_re_embedding: u32,
}

// --- Phase 6 doctor extensions (US5) -------------------------------------
//
// Five new emit-only `Serialize` records, appended LAST on [`DoctorReport`]
// as `Option<...>` with `skip_serializing_if = "Option::is_none"` so the
// Phase 1-5 byte-stable JSON wire pins are preserved when absent (Phase 4/5
// convention). Each is populated only when the resolved scope is a known
// workspace; `None` under `GlobalFallback` / outside-project modes. The
// persona report is additionally `None` whenever `expose_agents_as_personas`
// resolves false at the doctor scope. Contract:
// `contracts/doctor-extensions-p6.md`.
//
// All five are READ-ONLY surfaces (FR-124): the check functions only
// `fs::read` / `read_dir` / query the index — they never create a directory
// nor invoke the substitution layer.

/// A `<catalog>:<plugin>` provenance pair carried by the guardrails +
/// privilege-escalation reports. The display form is `<catalog>:<plugin>`
/// (the same key the guardrails marker regions use), but the structured
/// fields stay separate on the wire so JSON consumers don't re-split a
/// colon-joined string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogPlugin {
    pub catalog: String,
    pub plugin: String,
}

/// One hook event entry contributed (or expected-but-missing) for a plugin
/// on Claude Code. `event` is the event key (`PreToolUse`, …); `count` is
/// the number of rewritten entries under that event. The full rewritten
/// JSON is intentionally NOT carried — it embeds machine-absolute paths and
/// would bloat the report; the count is the auditable signal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookEventEntry {
    pub event: String,
    pub count: usize,
}

/// Per-plugin hooks contribution + drift. `contributed` are the rewritten
/// hook entries Tome found structurally present in `settings.local.json`;
/// `missing` are plugin-derived entries Tome expected but could not find
/// (drift from a user edit — reported, never auto-fixed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HookPluginEntry {
    pub catalog: String,
    pub plugin: String,
    pub contributed: Vec<HookEventEntry>,
    pub missing: Vec<HookEventEntry>,
}

/// Phase 6 hooks surface (Claude Code only). Per enabled plugin shipping a
/// `hooks/hooks.json`: what Tome contributed to `.claude/settings.local.json`
/// and what drifted. Empty `plugins` when no enabled plugin ships hooks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HooksReport {
    pub plugins: Vec<HookPluginEntry>,
}

/// Per-target-file guardrails region state. `present` are the regions Tome
/// found on disk; `orphaned` are present regions whose plugin is no longer
/// enabled (or whose harness is gone); `suppressed` are plugins whose
/// Claude Code `CLAUDE.md` region is suppressed because the plugin ships
/// real JSON hooks (FR-013).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuardrailsFileEntry {
    pub path: std::path::PathBuf,
    pub present: Vec<CatalogPlugin>,
    pub orphaned: Vec<CatalogPlugin>,
    pub suppressed: Vec<CatalogPlugin>,
}

/// Phase 6 guardrails surface. One entry per harness guardrails target
/// (in-file region or Cursor sibling) that exists on disk or that an
/// enabled plugin would contribute to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GuardrailsReport {
    pub files: Vec<GuardrailsFileEntry>,
}

/// One frontmatter field dropped during agent translation for a harness,
/// recorded informationally (FR-032 / FR-034 / FR-036). `agent` is the
/// `<plugin>__<name>` filename stem; `fields` are the dropped frontmatter
/// keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DroppedFieldEntry {
    pub agent: String,
    pub fields: Vec<String>,
}

/// Per-harness native-agent surface. `present` are the `<plugin>__*` agent
/// files Tome owns on disk; `orphaned` are owned files whose plugin is no
/// longer enabled (removable under `--fix`); `dropped_fields` records
/// per-agent field drops during translation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentHarnessEntry {
    pub harness: String,
    pub present: Vec<String>,
    pub orphaned: Vec<String>,
    pub dropped_fields: Vec<DroppedFieldEntry>,
}

/// Phase 6 agents surface. One entry per native-supporting harness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentsReport {
    pub harnesses: Vec<AgentHarnessEntry>,
}

/// One agent carrying privilege-escalation fields. `name` is the agent's
/// `<name>`; `fields` lists which of `hooks` / `mcpServers` /
/// `permissionMode` the SOURCE agent declares (read regardless of
/// `strip_plugin_agent_privileges`, FR-051).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrivilegeAgentEntry {
    pub name: String,
    pub fields: Vec<String>,
}

/// Per-plugin privilege-escalation grouping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrivilegePluginEntry {
    pub catalog: String,
    pub plugin: String,
    pub agents: Vec<PrivilegeAgentEntry>,
}

/// Phase 6 privilege-escalation surface (FR-051). Installed agents carrying
/// any of `hooks` / `mcpServers` / `permissionMode`, grouped by plugin, so
/// the escalation surface is auditable REGARDLESS of the
/// `strip_plugin_agent_privileges` setting's value (the audit reads the
/// agent SOURCE, never the emission clone). Empty `plugins` when no enabled
/// agent declares a privileged field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrivilegeEscalationReport {
    pub plugins: Vec<PrivilegePluginEntry>,
}

/// One persona prompt the MCP server would expose for the resolved
/// workspace. `resolved_persona_name` is the derived `<name>-persona` slug
/// (or `<plugin>-<name>-persona` for a clashing agent); `clash_prefixed`
/// records whether the plugin-prefixed form was used (FR-061).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonaEntry {
    pub catalog: String,
    pub plugin: String,
    pub agent_name: String,
    pub resolved_persona_name: String,
    pub clash_prefixed: bool,
}

/// Phase 6 personas surface. Populated only when `expose_agents_as_personas`
/// resolves true at the doctor scope (otherwise the whole report is `None`).
/// `drop_persona` is always the reserved `drop-persona` name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PersonaReport {
    pub personas: Vec<PersonaEntry>,
    pub drop_persona: String,
}

/// Full doctor report. Field order matches `contracts/doctor.md` +
/// `contracts/doctor-extensions-p4.md` so the rendered JSON is
/// deterministic.
///
/// Phase 4 adds:
/// - `project_binding` — None when outside any project marker.
/// - `summariser` — mirror of embedder/reranker.
/// - `effective_harness_list` — None when no harness composition resolves
///   (no project + no global harness declarations).
/// - `harness_rules` / `harness_mcp` — per-harness integration state for
///   every harness in `effective_harness_list`.
/// - `detected_uninstalled_harnesses` — FR-560 informational list of
///   supported harnesses present on the machine but not in the effective
///   list. Never affects classification.
///
/// Phase 5 / US5.b adds:
/// - `prompts` — `PromptsReport` for the resolved workspace; `None` when
///   not in a workspace context.
/// - `orphan_data_dirs` — `OrphanDataDirReport`; `None` outside a
///   workspace context.
/// - `entry_counts` — `EntryCountsByKind`; `None` outside a workspace
///   context.
///
/// Phase 5 also drops the `PartialEq` / `Eq` derives from `DoctorReport`
/// because `PromptsReport` carries rmcp's `Prompt` (only `PartialEq`,
/// not `Eq`) and `CollisionRecord` (no equality at all). The JSON wire
/// shape is what tests pin; equality on the whole struct has no
/// production consumer.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub tome_version: String,
    pub workspace: WorkspaceInfo,
    /// FR-564: populated only when the resolved scope's source is
    /// `ProjectMarker`. From outside any project this is `None` and the
    /// harness subsystems use the global effective list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_binding: Option<ProjectBindingState>,
    pub embedder: ModelHealth,
    pub reranker: ModelHealth,
    /// Phase 4 / US5.a: summariser cheap-probe identity + state.
    pub summariser: ModelHealth,
    pub index: IndexHealth,
    pub drift: DriftStatus,
    pub catalogs: Vec<CatalogCacheHealth>,
    /// FR-M-DOC-2 / `catalog-extensions-p3.md` §"Doctor reporting":
    /// status of the opt-in workspace registry file (presence + count).
    pub workspace_registry: WorkspaceRegistryStatus,
    pub harnesses: Vec<HarnessPresence>,
    /// FR-560 / FR-561: snapshot of the resolved effective harness list
    /// (composition output). `None` when no scope declares `harnesses`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_harness_list: Option<EffectiveHarnessList>,
    /// Per-harness rules-file integration state (one entry per harness
    /// in `effective_harness_list`). Empty when the effective list is.
    pub harness_rules: Vec<HarnessSubsystemReport>,
    /// Per-harness MCP-config integration state.
    pub harness_mcp: Vec<HarnessSubsystemReport>,
    /// FR-560 informational list: supported harnesses present on the
    /// local machine (via `HarnessModule::detect`) but NOT in the
    /// effective list. Never affects overall classification.
    pub detected_uninstalled_harnesses: Vec<String>,
    /// Phase 5 / US5.b: prompts surface for the resolved workspace plus
    /// any collision records. `None` outside a workspace context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompts: Option<PromptsReport>,
    /// Phase 5 / US5.b: orphan plugin-data + workspace-data directories.
    /// `None` outside a workspace context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub orphan_data_dirs: Option<OrphanDataDirReport>,
    /// Phase 5 / US5.b: per-kind entry counts for the resolved workspace.
    /// `None` outside a workspace context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_counts: Option<EntryCountsByKind>,
    /// Phase 6 / US5: Claude Code hooks contribution + drift. `None`
    /// outside a workspace context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hooks: Option<HooksReport>,
    /// Phase 6 / US5: guardrails region state per target file. `None`
    /// outside a workspace context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guardrails: Option<GuardrailsReport>,
    /// Phase 6 / US5: native-agent file state per harness. `None` outside
    /// a workspace context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents: Option<AgentsReport>,
    /// Phase 6 / US5: privilege-escalation audit (FR-051). `None` outside
    /// a workspace context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub privilege_escalation: Option<PrivilegeEscalationReport>,
    /// Phase 6 / US5: persona surface. `None` outside a workspace context
    /// OR when `expose_agents_as_personas` resolves false at the scope.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub personas: Option<PersonaReport>,
    pub overall: DoctorClassification,
    pub suggested_fixes: Vec<SuggestedFix>,
}

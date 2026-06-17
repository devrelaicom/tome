//! `WorkspaceInfo` — the serialisable record describing the resolved scope
//! and its on-disk index state. Consumed by `tome workspace info` (the
//! direct surface) and by `tome doctor` (which embeds the same record
//! verbatim under its `workspace` field, per data-model §5).
//!
//! Emit-only — these types never participate in deserialisation, so no
//! `#[serde(deny_unknown_fields)]`. The wire JSON shape is contract; the
//! `tests/workspace_info.rs` `--json` byte-stability test pins it.

use std::path::PathBuf;

use serde::Serialize;

use crate::workspace::scope::ScopeSource;

/// Two-state scope kind as serialised in `--json` output ("global" /
/// "workspace"). Distinct from the in-process `Scope` enum which carries
/// the workspace path inside the variant; the JSON record splits the kind
/// from the path into separate fields per `contracts/workspace-info.md`.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScopeKind {
    Global,
    Workspace,
}

/// Model identity (name + version) as it appears in the JSON record's
/// `embedder` field. Named structurally rather than re-using the
/// `commands::status::ModelHealth` so the wire shape is exactly the two
/// fields the contract pins — no health-state leakage.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelIdentity {
    pub name: String,
    pub version: String,
}

/// One enrolled catalog inside `WorkspaceInfo::enrolled_catalogs`. The
/// triple (name, url, pinned_ref) mirrors the `workspace_catalogs` junction
/// table; the on-disk clone path lives at
/// `paths.cache_dir_for(&url)` and is recomputable by the caller.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorkspaceCatalogEntry {
    pub name: String,
    pub url: String,
    pub pinned_ref: String,
}

/// One enabled plugin inside `WorkspaceInfo::enabled_plugins`. The
/// skill_count is the number of `workspace_skills` rows joined to this
/// `(workspace, catalog, plugin)` triple.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EnabledPluginRecord {
    pub catalog: String,
    pub plugin: String,
    pub skill_count: u32,
}

/// Cached summary length information inside
/// `WorkspaceInfo::summary_cache`. Mirrors the `[summaries]` section of
/// `<root>/workspaces/<name>/settings.toml`. The `chars` fields are
/// character counts; the workspace's `RULES.md` body is the on-disk
/// representation of `long`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SummaryCacheState {
    pub short_chars: usize,
    pub long_chars: usize,
    /// RFC 3339 timestamp string. Distinct from a raw
    /// `time::OffsetDateTime` so the JSON wire-shape stays stringy
    /// regardless of TOML's native datetime support.
    pub generated_at: String,
}

/// One entry inside a `--details` plugin breakdown. `tier` is `None` for agents.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DetailEntry {
    pub name: String,
    pub kind: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tier: Option<u8>,
}

/// A `--details` per-plugin breakdown.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PluginDetail {
    pub catalog: String,
    pub plugin: String,
    pub skills: Vec<DetailEntry>,
    pub commands: Vec<DetailEntry>,
    pub agents: Vec<DetailEntry>,
}

/// The wire record for `tome workspace info`. Field order matches
/// `contracts/workspace-commands.md` § `tome workspace info` so the
/// human-readable `serde_json::to_string` rendering is deterministic.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorkspaceInfo {
    pub scope: ScopeKind,
    pub path: Option<PathBuf>,
    pub source: ScopeSource,
    pub catalogs: u32,
    pub plugins_total: u32,
    pub plugins_enabled: u32,
    pub skills_indexed: u32,
    pub schema_version: Option<u32>,
    pub embedder: Option<ModelIdentity>,
    /// Phase 4 / US2.a — every catalog enrolled in this workspace via
    /// the `workspace_catalogs` junction table. Ordered by catalog
    /// name.
    #[serde(default)]
    pub enrolled_catalogs: Vec<WorkspaceCatalogEntry>,
    /// Phase 4 / US2.a — every enabled `(catalog, plugin)` for this
    /// workspace via the `workspace_skills` junction joined to
    /// `skills`. Ordered by `(catalog, plugin)`.
    #[serde(default)]
    pub enabled_plugins: Vec<EnabledPluginRecord>,
    /// Phase 4 / US2.a — every project bound to this workspace via the
    /// `workspace_projects` table. Ordered by `project_path`.
    #[serde(default)]
    pub bound_projects: Vec<PathBuf>,
    /// Phase 4 / US2.a — cached short/long summary lengths + the
    /// generation timestamp. `None` when the workspace has no
    /// `[summaries]` block yet (US2.a-2's `regen-summary` fills it).
    #[serde(default)]
    pub summary_cache: Option<SummaryCacheState>,
    /// Per-plugin entry breakdown with tiers, populated only when `--details`
    /// is passed. Absent from the JSON wire otherwise (default shape unchanged).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_details: Option<Vec<PluginDetail>>,
}

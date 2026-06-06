//! Plugin metadata: identity, third-party manifest / frontmatter parsers, and
//! component enumeration.
//!
//! The submodules are isolated parsers: they do not touch the catalog
//! registry or the index database. Wiring into `tome plugin enable / disable`
//! happens in later slices via `src/plugin/lifecycle.rs`.

pub mod components;
pub mod frontmatter;
pub mod identity;
pub mod lifecycle;
pub mod manifest;

pub use components::ComponentCounts;
pub use frontmatter::{FrontmatterError, ParsedSkill, SkillFrontmatter};
pub use identity::{PluginId, PluginIdParseError, SegmentRejection};
pub use lifecycle::{DisableOutcome, EnableOutcome, LifecycleDeps, disable, enable};
pub use manifest::{
    PluginAuthor, PluginManifest, TomeAuthor, TomePluginManifest, manifest_path_for,
    parse_plugin_manifest, read_plugin_manifest, tome_manifest_path_for,
};

use time::OffsetDateTime;

/// Aggregated view of a plugin returned by `tome plugin list / show`. Built
/// on demand by walking the catalog cache and joining with index state — not
/// persisted in Phase 2.
///
/// Spec: data-model.md §2.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginRecord {
    pub id: PluginId,
    pub version: String,
    pub author: Option<String>,
    pub description: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_upstream_change: Option<OffsetDateTime>,
    pub status: PluginStatus,
    pub component_counts: ComponentCounts,
    #[serde(with = "time::serde::rfc3339::option")]
    pub last_indexed_at: Option<OffsetDateTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginStatus {
    Enabled,
    Disabled,
    Unindexable,
}

//! Tome index database — SQLite + sqlite-vec.
//!
//! Slice 4a (this slice) ships the bootstrap pipeline: schema, forward-only
//! migrations, the static vec-extension registrar, and the [`db::open`] entry
//! point that ties them together. Slice 4b will add the advisory lock, the
//! `meta` accessors, the integrity check, the `skills` CRUD layer, and the
//! KNN query helper.
//!
//! Spec: data-model.md §5–9, contracts/index-schema.sql, research §R3.

pub mod db;
pub mod integrity;
pub mod lock;
pub mod meta;
pub mod migrations;
pub mod query;
pub mod schema;
pub mod skills;
pub mod vec_ext;
pub mod workspace_catalogs;
pub mod workspaces;

pub use db::{OpenOptions, open, open_read_only};
pub use lock::{LockGuard, acquire as acquire_lock};
pub use meta::{
    DriftStatus, MetaKey, ModelIdent, detect_drift, read_embedder_dimension,
    write_embedder_dimension,
};
pub use migrations::{MIGRATIONS, Migration, apply_pending, current_schema_version};
// Phase 5 re-export: the `EntryKind` discriminator threads through every
// `index::skills` entry point. Keep the type accessible from
// `crate::index::EntryKind` to keep call sites brief.
pub use crate::plugin::identity::EntryKind;
pub use query::{Candidate, QueryFilters, knn};
pub use schema::{CREATE_STATEMENTS, GLOBAL_WORKSPACE, MetaSeed, SCHEMA_VERSION, bootstrap};
pub use skills::{
    EnableSummary, PendingSkill, ReindexSummary, SkillRecord, content_hash, delete_by_plugin,
    embedding_text, enable_plugin_atomic, enabled_plugins_for_catalog, find as find_skill,
    list_for_plugin, mark_all_disabled_for_plugin, reindex_plugin_atomic,
};
pub use workspace_catalogs::CatalogEnrolment;

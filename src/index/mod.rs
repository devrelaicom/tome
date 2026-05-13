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

pub use db::{OpenOptions, open};
pub use lock::{LockGuard, acquire as acquire_lock};
pub use meta::{DriftStatus, MetaKey, ModelIdent, detect_drift};
pub use migrations::{MIGRATIONS, Migration, apply_pending, current_schema_version};
pub use query::{Candidate, QueryFilters, knn};
pub use schema::{CREATE_STATEMENTS, MetaSeed, SCHEMA_VERSION, bootstrap};
pub use skills::{
    EnableSummary, PendingSkill, ReindexSummary, SkillRecord, content_hash, delete_by_plugin,
    embedding_text, enable_plugin_atomic, enabled_plugins_for_catalog, find as find_skill,
    list_for_plugin, mark_all_disabled_for_plugin, reindex_plugin_atomic,
};

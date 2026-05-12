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
pub mod migrations;
pub mod schema;
pub mod vec_ext;

pub use db::{OpenOptions, open};
pub use migrations::{MIGRATIONS, Migration, apply_pending, current_schema_version};
pub use schema::{CREATE_STATEMENTS, MetaSeed, SCHEMA_VERSION, bootstrap};

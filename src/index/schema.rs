//! SQLite schema for the Tome index database.
//!
//! Mirror of [`contracts/index-schema.sql`](../../specs/002-phase-2-plugins-index/contracts/index-schema.sql).
//! When this Rust file and the SQL file diverge, the SQL file is canonical and
//! this one must be updated.
//!
//! The bootstrap function applies every statement in [`CREATE_STATEMENTS`]
//! inside a single transaction and seeds the [`meta`] table — see research §R3
//! for the migration policy ("v0 → v1 is bootstrap, not a migration").

use rusqlite::{Connection, params};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::error::TomeError;

/// The schema version Tome's compiled-in code understands. Bumping this
/// requires a matching `Migration` row in [`crate::index::migrations`].
pub const SCHEMA_VERSION: u32 = 1;

/// Embedder + reranker identification stored in `meta` at bootstrap. The
/// caller (typically `db::open`) supplies the configured names so the index
/// can later detect drift against a future-different runtime config.
#[derive(Debug, Clone)]
pub struct MetaSeed {
    pub name: String,
    pub version: String,
}

/// CREATE statements applied in order for a fresh database. Split into one
/// statement per slice so a failure mid-bootstrap surfaces with the exact
/// statement that broke. STRICT typing on `meta` and `skills` is defence in
/// depth against insert paths that bypass the Rust type system.
pub const CREATE_STATEMENTS: &[&str] = &[
    "CREATE TABLE meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) STRICT",
    "CREATE TABLE skills (
        id              INTEGER PRIMARY KEY,
        catalog         TEXT NOT NULL,
        plugin          TEXT NOT NULL,
        name            TEXT NOT NULL,
        description     TEXT NOT NULL,
        plugin_version  TEXT NOT NULL,
        path            TEXT NOT NULL,
        content_hash    TEXT NOT NULL,
        enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
        indexed_at      TEXT NOT NULL,
        UNIQUE (catalog, plugin, name)
    ) STRICT",
    "CREATE INDEX idx_skills_catalog_plugin ON skills(catalog, plugin)",
    "CREATE INDEX idx_skills_enabled        ON skills(enabled)",
    "CREATE INDEX idx_skills_content_hash   ON skills(content_hash)",
    "CREATE VIRTUAL TABLE skill_embeddings USING vec0(
        skill_id   INTEGER PRIMARY KEY,
        embedding  FLOAT[384]
    )",
];

/// Apply every [`CREATE_STATEMENTS`] row and seed the `meta` table. Runs
/// inside a single transaction: a partial bootstrap is never observable on
/// disk thanks to WAL atomicity.
pub fn bootstrap(
    conn: &mut Connection,
    embedder: &MetaSeed,
    reranker: &MetaSeed,
) -> Result<(), TomeError> {
    let tx = conn
        .transaction()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("begin bootstrap tx: {e}")))?;

    for stmt in CREATE_STATEMENTS {
        tx.execute_batch(stmt).map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "bootstrap statement failed ({e}): {stmt}"
            ))
        })?;
    }

    let now = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned());
    let schema_version = SCHEMA_VERSION.to_string();

    let rows: [(&str, &str); 6] = [
        ("schema_version", schema_version.as_str()),
        ("embedder_name", embedder.name.as_str()),
        ("embedder_version", embedder.version.as_str()),
        ("reranker_name", reranker.name.as_str()),
        ("reranker_version", reranker.version.as_str()),
        ("created_at", now.as_str()),
    ];
    for (k, v) in rows {
        tx.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)",
            params![k, v],
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("seed meta `{k}` failed: {e}"))
        })?;
    }

    tx.commit()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("commit bootstrap: {e}")))?;
    Ok(())
}

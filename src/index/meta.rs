//! Typed accessors for the `meta` key/value table.
//!
//! [`MetaKey`] is the closed set of valid keys; values are TEXT in SQLite
//! but typed in Rust. Unknown keys observed on read are returned as
//! [`Option::None`] from [`read`] and reported to the caller; unknown keys
//! are never written by Tome itself (forward-compat with future versions
//! that may seed additional rows).
//!
//! [`detect_drift`] compares the embedder + reranker rows against the
//! caller-supplied configured values and returns the [`DriftStatus`]
//! variant from data-model §11.
//!
//! Spec: data-model.md §8 (MetaKey) and §11 (DriftStatus).

use rusqlite::Connection;
use rusqlite::params;

use crate::error::TomeError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaKey {
    SchemaVersion,
    EmbedderName,
    EmbedderVersion,
    RerankerName,
    RerankerVersion,
    CreatedAt,
    LastWriterPid,
}

impl MetaKey {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SchemaVersion => "schema_version",
            Self::EmbedderName => "embedder_name",
            Self::EmbedderVersion => "embedder_version",
            Self::RerankerName => "reranker_name",
            Self::RerankerVersion => "reranker_version",
            Self::CreatedAt => "created_at",
            Self::LastWriterPid => "last_writer_pid",
        }
    }
}

/// Read a meta row. Returns `Ok(None)` when the key is absent.
pub fn read(conn: &Connection, key: MetaKey) -> Result<Option<String>, TomeError> {
    let result = conn.query_row(
        "SELECT value FROM meta WHERE key = ?1",
        params![key.as_str()],
        |row| row.get::<_, String>(0),
    );
    match result {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(TomeError::IndexIntegrityCheckFailure(format!(
            "read meta `{}`: {e}",
            key.as_str()
        ))),
    }
}

/// Write a meta row. Inserts on first write, replaces on subsequent writes.
pub fn write(conn: &Connection, key: MetaKey, value: &str) -> Result<(), TomeError> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key.as_str(), value],
    )
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("write meta `{}`: {e}", key.as_str()))
    })?;
    Ok(())
}

/// Identification of a model — name + version — used by [`detect_drift`].
#[derive(Debug, Clone)]
pub struct ModelIdent {
    pub name: String,
    pub version: String,
}

/// Drift-detection verdict between the stored meta rows and the caller's
/// configured embedder / reranker. Mirrors the on-the-wire variant from
/// `StatusReport.drift` (data-model §11).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum DriftStatus {
    None,
    EmbedderNameDrift { stored: String, configured: String },
    EmbedderVersionDrift { stored: String, configured: String },
    RerankerDrift { stored: String, configured: String },
}

/// Compare the embedder and reranker rows in `meta` against the configured
/// values. Returns the most-specific drift variant: embedder name drift
/// shadows version drift, and any embedder drift shadows reranker drift
/// (because embedder drift invalidates the stored vectors entirely; reranker
/// drift only degrades query quality, see plan.md §Drift handling).
pub fn detect_drift(
    conn: &Connection,
    embedder: &ModelIdent,
    reranker: &ModelIdent,
) -> Result<DriftStatus, TomeError> {
    let stored_embedder_name = read(conn, MetaKey::EmbedderName)?.unwrap_or_default();
    let stored_embedder_version = read(conn, MetaKey::EmbedderVersion)?.unwrap_or_default();
    let stored_reranker_name = read(conn, MetaKey::RerankerName)?.unwrap_or_default();
    let stored_reranker_version = read(conn, MetaKey::RerankerVersion)?.unwrap_or_default();

    if stored_embedder_name != embedder.name {
        return Ok(DriftStatus::EmbedderNameDrift {
            stored: stored_embedder_name,
            configured: embedder.name.clone(),
        });
    }
    if stored_embedder_version != embedder.version {
        return Ok(DriftStatus::EmbedderVersionDrift {
            stored: stored_embedder_version,
            configured: embedder.version.clone(),
        });
    }
    if stored_reranker_name != reranker.name {
        return Ok(DriftStatus::RerankerDrift {
            stored: stored_reranker_name,
            configured: reranker.name.clone(),
        });
    }
    if stored_reranker_version != reranker.version {
        return Ok(DriftStatus::RerankerDrift {
            stored: stored_reranker_version,
            configured: reranker.version.clone(),
        });
    }
    Ok(DriftStatus::None)
}

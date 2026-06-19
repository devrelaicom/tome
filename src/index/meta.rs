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
    /// Phase 4 / F9: summariser identity row, recorded alongside the
    /// embedder + reranker during bootstrap. Drift detection treats the
    /// placeholder SHA-256 entry (F6 ships with one) as "skip until US4.a".
    SummariserName,
    SummariserVersion,
    CreatedAt,
    /// Phase p11 / model tiering: which profile (small/medium/large) this
    /// index was bootstrapped with. Written at bootstrap and at profile-switch
    /// time. Absent in pre-v6 DBs — `active_profile` defaults to
    /// `Profile::DEFAULT` in that case.
    ModelProfile,
}

impl MetaKey {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SchemaVersion => "schema_version",
            Self::EmbedderName => "embedder_name",
            Self::EmbedderVersion => "embedder_version",
            Self::RerankerName => "reranker_name",
            Self::RerankerVersion => "reranker_version",
            Self::SummariserName => "summariser_name",
            Self::SummariserVersion => "summariser_version",
            Self::CreatedAt => "created_at",
            Self::ModelProfile => "model_profile",
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
/// configured embedder / reranker / summariser. Mirrors the on-the-wire
/// variant from `StatusReport.drift` (data-model §11). Summariser drift
/// is recorded but, when both stored and configured carry the F6
/// placeholder identity (Phase 4 / F9 transient state), drift is
/// suppressed so US4.a's real-model wire-up is the trigger rather than
/// every bootstrap-against-placeholder open.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum DriftStatus {
    None,
    EmbedderNameDrift { stored: String, configured: String },
    EmbedderVersionDrift { stored: String, configured: String },
    RerankerDrift { stored: String, configured: String },
    SummariserDrift { stored: String, configured: String },
}

/// Compare the embedder, reranker, and summariser rows in `meta` against
/// the configured values. Returns the most-specific drift variant:
/// embedder name drift shadows version drift, any embedder drift shadows
/// reranker drift (because embedder drift invalidates the stored vectors
/// entirely; reranker drift only degrades query quality, see plan.md
/// §Drift handling). Summariser drift is the lowest-priority signal —
/// it only affects the cached natural-language summaries, never the
/// retrieval pipeline.
///
/// Phase 4 / F9 caveat: until US4.a flips the summariser registry's
/// SHA-256 to a real value, the bootstrap path writes the placeholder
/// name/version into `meta` and the caller passes the same placeholder
/// values in `summariser`. Drift detection MUST stay silent in that
/// case — every fresh DB would otherwise report `SummariserDrift`
/// against itself.
pub fn detect_drift(
    conn: &Connection,
    embedder: &ModelIdent,
    reranker: &ModelIdent,
    summariser: &ModelIdent,
) -> Result<DriftStatus, TomeError> {
    let stored_embedder_name = read(conn, MetaKey::EmbedderName)?.unwrap_or_default();
    let stored_embedder_version = read(conn, MetaKey::EmbedderVersion)?.unwrap_or_default();
    let stored_reranker_name = read(conn, MetaKey::RerankerName)?.unwrap_or_default();
    let stored_reranker_version = read(conn, MetaKey::RerankerVersion)?.unwrap_or_default();
    let stored_summariser_name = read(conn, MetaKey::SummariserName)?.unwrap_or_default();
    let stored_summariser_version = read(conn, MetaKey::SummariserVersion)?.unwrap_or_default();

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
    if stored_summariser_name != summariser.name {
        return Ok(DriftStatus::SummariserDrift {
            stored: stored_summariser_name,
            configured: summariser.name.clone(),
        });
    }
    if stored_summariser_version != summariser.version {
        return Ok(DriftStatus::SummariserDrift {
            stored: stored_summariser_version,
            configured: summariser.version.clone(),
        });
    }
    Ok(DriftStatus::None)
}

/// The active model profile recorded in `meta`. Absent → Profile::DEFAULT
/// (forward-compat for a DB written before this row existed).
pub fn active_profile(
    conn: &Connection,
) -> Result<crate::embedding::profile::Profile, crate::error::TomeError> {
    use crate::embedding::profile::Profile;
    Ok(read(conn, MetaKey::ModelProfile)?
        .as_deref()
        .and_then(Profile::from_tier_str)
        .unwrap_or(Profile::DEFAULT))
}

/// The embedder registry entry the ACTIVE profile selects (B4). Conn-bearing
/// call sites resolve through this rather than the removed zero-arg
/// `embedder_entry()` so a non-default profile's embedder is honoured
/// everywhere a connection is in hand.
pub fn active_embedder(
    conn: &Connection,
) -> Result<&'static crate::embedding::registry::ModelEntry, TomeError> {
    Ok(crate::embedding::profile::embedder_for(active_profile(
        conn,
    )?))
}

/// The reranker registry entry the ACTIVE profile selects (B4). Companion to
/// [`active_embedder`].
pub fn active_reranker(
    conn: &Connection,
) -> Result<&'static crate::embedding::registry::ModelEntry, TomeError> {
    Ok(crate::embedding::profile::reranker_for(active_profile(
        conn,
    )?))
}

/// B3 / model-tiering drift guard. Refuses any partial-re-embed path
/// (`plugin enable`, `catalog update`) when the configured active-profile
/// embedder no longer matches the embedder identity stamped in `meta`.
///
/// Embedder name OR version drift returns the corresponding
/// [`TomeError::EmbedderNameDrift`] / [`TomeError::EmbedderVersionDrift`]
/// (exit 41 / 42), each of which directs the user at `tome reindex --force`.
/// Reranker / summariser drift do NOT block — only the embedder change
/// invalidates the stored vectors' dimension. `reindex` is the sole resolver
/// and is exempt (it forces the whole-index re-embed itself, B1).
pub fn guard_embedder_drift(
    conn: &Connection,
    configured_embedder: &ModelIdent,
) -> Result<(), TomeError> {
    let stored_embedder_name = read(conn, MetaKey::EmbedderName)?.unwrap_or_default();
    let stored_embedder_version = read(conn, MetaKey::EmbedderVersion)?.unwrap_or_default();

    if stored_embedder_name != configured_embedder.name {
        return Err(TomeError::EmbedderNameDrift {
            stored: stored_embedder_name,
            configured: configured_embedder.name.clone(),
        });
    }
    if stored_embedder_version != configured_embedder.version {
        return Err(TomeError::EmbedderVersionDrift {
            stored: stored_embedder_version,
            configured: configured_embedder.version.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_mem() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;")
            .unwrap();
        conn
    }

    #[test]
    fn active_profile_defaults_to_medium_when_absent() {
        use crate::embedding::profile::Profile;
        let conn = open_mem();
        let p = active_profile(&conn).expect("active_profile should not fail on absent row");
        assert_eq!(p, Profile::DEFAULT);
        assert_eq!(p, Profile::Medium);
    }

    #[test]
    fn active_profile_round_trips_all_tiers() {
        use crate::embedding::profile::Profile;
        let conn = open_mem();
        for p in Profile::ALL {
            write(&conn, MetaKey::ModelProfile, p.as_str()).unwrap();
            let got = active_profile(&conn).expect("round-trip read");
            assert_eq!(got, p, "round-trip failed for {:?}", p);
        }
    }

    #[test]
    fn active_profile_defaults_on_unknown_value() {
        use crate::embedding::profile::Profile;
        let conn = open_mem();
        write(&conn, MetaKey::ModelProfile, "xl").unwrap();
        let p = active_profile(&conn).expect("should not error on unknown tier");
        assert_eq!(
            p,
            Profile::DEFAULT,
            "unknown tier should fall back to DEFAULT"
        );
    }

    #[test]
    fn meta_key_model_profile_str_is_correct() {
        assert_eq!(MetaKey::ModelProfile.as_str(), "model_profile");
    }
}

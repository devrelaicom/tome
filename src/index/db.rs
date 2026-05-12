//! Open / bootstrap / migrate the Tome index database.
//!
//! The single entry point [`open`] wires everything in slice 4a together:
//!
//! 1. Register `sqlite-vec` with SQLite's auto-extension hook (idempotent).
//! 2. Ensure the parent directory exists, then open the SQLite file.
//! 3. Apply the connection-level PRAGMAs (`journal_mode = WAL`,
//!    `synchronous = NORMAL`, `foreign_keys = ON`, `busy_timeout = 5000`).
//! 4. If the schema is absent, run [`schema::bootstrap`]. If older, apply
//!    pending [`migrations`]. If newer, refuse with [`TomeError::SchemaTooNew`].
//! 5. Verify the vec extension is reachable on this connection.
//!
//! Concurrency note: this slice does not acquire the advisory lockfile yet.
//! That arrives in slice 4b — callers that perform writes must wrap them.

use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::error::TomeError;
use crate::index::migrations;
use crate::index::schema::{self, MetaSeed};
use crate::index::vec_ext;

/// Inputs to [`open`] that the caller controls. `embedder` and `reranker`
/// are written into `meta` on bootstrap and used by drift detection later;
/// they are ignored on subsequent opens.
#[derive(Debug, Clone)]
pub struct OpenOptions {
    pub embedder: MetaSeed,
    pub reranker: MetaSeed,
}

/// Open (or bootstrap) the index database at `db_path`. The parent directory
/// is created if missing. Subsequent opens of the same file are no-ops apart
/// from the PRAGMA reapplication.
pub fn open(db_path: &Path, opts: &OpenOptions) -> Result<Connection, TomeError> {
    vec_ext::register_globally()?;

    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    }

    let mut conn = Connection::open(db_path).map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("open {}: {e}", db_path.display()))
    })?;

    apply_connection_pragmas(&conn)?;

    match migrations::current_schema_version(&conn)? {
        None => {
            schema::bootstrap(&mut conn, &opts.embedder, &opts.reranker)?;
        }
        Some(stored) => {
            migrations::apply_pending(&mut conn, stored)?;
        }
    }

    vec_ext::verify(&conn)?;
    Ok(conn)
}

fn apply_connection_pragmas(conn: &Connection) -> Result<(), TomeError> {
    // WAL must be set before any user statements that would acquire a lock.
    // Using `pragma_update`-style execution keeps these PRAGMAs explicit
    // rather than wrapped in `Connection::pragma`.
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("set journal_mode=WAL: {e}")))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("set synchronous=NORMAL: {e}"))
        })?;
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("set foreign_keys=ON: {e}")))?;
    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("set busy_timeout=5000: {e}"))
        })?;
    Ok(())
}

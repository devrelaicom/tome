//! Open / bootstrap / migrate the Tome index database.
//!
//! The single entry point [`open`] wires everything in slice 4a together:
//!
//! 1. Register `sqlite-vec` with SQLite's auto-extension hook (idempotent).
//! 2. Ensure the parent directory exists, then open the SQLite file.
//! 3. Apply the connection-level PRAGMAs (`journal_mode = WAL`,
//!    `synchronous = NORMAL`, `foreign_keys = ON`, `busy_timeout = 5000`).
//! 4. If the schema is absent, run [`schema::bootstrap`]. If older, apply
//!    pending [`migrations`]. If newer, the migration framework refuses
//!    with [`TomeError::SchemaVersionTooNew`] (exit 73). The legacy
//!    [`open_read_only`] gate continues to emit [`TomeError::SchemaTooNew`]
//!    (exit 52) for the read path — see its docstring for the rationale.
//! 5. Verify the vec extension is reachable on this connection.
//!
//! Concurrency note: this slice does not acquire the advisory lockfile yet.
//! That arrives in slice 4b — callers that perform writes must wrap them.

use std::path::Path;
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use crate::error::TomeError;
use crate::index::migrations;
use crate::index::schema::{self, MetaSeed};
use crate::index::vec_ext;

/// Inputs to [`open`] that the caller controls. `embedder`, `reranker`,
/// and `summariser` are written into `meta` on bootstrap and used by
/// drift detection later; they are ignored on subsequent opens.
///
/// `profile` seeds `model_profile` in `meta` on a **fresh** index only;
/// it is ignored on subsequent opens (the DB's own stored profile wins).
/// `None` (the default) falls back to `Profile::DEFAULT`.
#[derive(Debug, Clone)]
pub struct OpenOptions {
    pub embedder: MetaSeed,
    pub reranker: MetaSeed,
    pub summariser: MetaSeed,
    /// Profile to seed on fresh bootstrap. `None` → `Profile::DEFAULT`.
    pub profile: Option<crate::embedding::profile::Profile>,
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
            // Resolve the bootstrap profile: explicit caller value → config file
            // sibling → Profile::DEFAULT. The config file is `<tome-root>/config.toml`
            // where `<tome-root>` is `db_path.parent()` (always `~/.tome/`).
            // This is the ONE chokepoint that governs ALL fresh-index bootstraps
            // regardless of which command first touches the DB, so every command
            // (`catalog add`, `plugin enable`, `reindex`, …) honours the user's
            // `[models] profile` setting without any caller changes.
            //
            // Loading is defensive: any error (missing parent, missing file, parse
            // failure, I/O error) silently falls through to `Profile::DEFAULT` so
            // unit tests that open a bare temp-dir DB without a full `~/.tome/`
            // setup are unaffected, and a malformed `config.toml` never prevents
            // an unrelated command from bootstrapping.
            //
            // Invariant: this path only runs ONCE per DB lifetime (bootstrap fires
            // exactly when the schema is absent). Subsequent opens take the
            // `Some(stored)` branch and never re-read or re-embed the profile.
            let profile = opts.profile.unwrap_or_else(|| {
                db_path
                    .parent()
                    .and_then(|root| {
                        // Load config defensively via the SSOT helper; any error
                        // → default Config → None profile → fallback below.
                        crate::config::load_or_default_from_root(root)
                            .models
                            .profile
                    })
                    .unwrap_or(crate::embedding::profile::Profile::DEFAULT)
            });
            schema::bootstrap(
                &mut conn,
                &opts.embedder,
                &opts.reranker,
                &opts.summariser,
                profile,
            )?;
        }
        Some(stored) => {
            migrations::apply_pending(&mut conn, stored, schema::SCHEMA_VERSION)?;
        }
    }

    vec_ext::verify(&conn)?;
    Ok(conn)
}

/// Open the index database read-only. Skips schema bootstrap, migration,
/// and the WAL/synchronous/foreign_keys PRAGMAs (read-only connections
/// can't write any of them). The caller is responsible for verifying the
/// file exists — `Connection::open_with_flags` with `SQLITE_OPEN_READ_ONLY`
/// errors on a missing file rather than creating it.
///
/// Designed for the read paths: `tome plugin list`, `tome plugin show`,
/// `tome query`, `tome status`, and the future `tome doctor`. They never
/// take the advisory lockfile, so a read-only handle here cannot race
/// with the writer regardless of the WAL state — SQLite's MVCC model
/// gives readers a consistent snapshot.
///
/// Phase 3 contract: this used to be a Phase 10 deferred improvement;
/// MCP's read-side reuses the same surface, so it lands in Foundational.
///
/// `busy_timeout` is still applied so a brief writer hold doesn't make
/// the reader fail immediately. `vec_ext::register_globally()` is called
/// for symmetry with `open` — the query path needs the extension visible
/// to read from `skill_embeddings`. The signature stays parallel to
/// `open`: caller computes the per-scope path and hands it in.
pub fn open_read_only(db_path: &Path) -> Result<Connection, TomeError> {
    vec_ext::register_globally()?;

    let conn = Connection::open_with_flags(
        db_path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("open read-only {}: {e}", db_path.display(),))
    })?;

    conn.busy_timeout(Duration::from_millis(5000))
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("set busy_timeout=5000: {e}"))
        })?;

    vec_ext::verify(&conn)?;

    // Schema-version gate. Writers run this in `apply_pending`; readers
    // don't need migrations but DO need to refuse a future-version DB —
    // it would otherwise read garbage columns or fail with cryptic
    // SQLite errors mid-query. Map the same way `open` does so `tome
    // status` / `tome query` surface exit 52 (`SchemaTooNew`) instead of
    // exit 1 / 51 from a downstream failure.
    if let Some(stored) = migrations::current_schema_version(&conn)?
        && stored > schema::SCHEMA_VERSION
    {
        return Err(TomeError::SchemaTooNew {
            on_disk: stored,
            compiled: schema::SCHEMA_VERSION,
        });
    }

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

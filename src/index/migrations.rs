//! Forward-only schema migrations.
//!
//! Policy (research §R3):
//!
//! * `stored == compiled` — proceed.
//! * `stored  > compiled` — refuse with [`TomeError::SchemaTooNew`] (exit 52).
//! * `stored  < compiled` — apply every migration whose `from < compiled` in
//!   order, inside one transaction each, under the advisory lock acquired by
//!   the caller.
//! * `stored absent` — fresh DB; the caller runs `schema::bootstrap` instead.
//!
//! No down-migrations: older Tome refuses newer DBs. A v2 patch ships as one
//! row appended to [`MIGRATIONS`] plus a SQL file under `migrations/`.

use rusqlite::Connection;

use crate::error::TomeError;
use crate::index::schema::SCHEMA_VERSION;

#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub from: u32,
    pub to: u32,
    pub sql: &'static str,
}

/// All forward migrations Tome knows about, in apply order. Empty in Phase 2:
/// v0 → v1 is the bootstrap path (no migration row), and v2 is unshipped.
pub static MIGRATIONS: &[Migration] = &[];

/// Read `meta.schema_version`. Returns `None` if the `meta` table itself
/// does not exist (fresh DB); returns `Some(version)` otherwise.
pub fn current_schema_version(conn: &Connection) -> Result<Option<u32>, TomeError> {
    let meta_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'meta'",
            [],
            |_| Ok(true),
        )
        .or_else(|err| match err {
            rusqlite::Error::QueryReturnedNoRows => Ok(false),
            other => Err(other),
        })
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("probe meta table: {e}")))?;

    if !meta_exists {
        return Ok(None);
    }

    let raw: Option<String> = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok();

    match raw {
        None => Ok(None),
        Some(s) => s.parse::<u32>().map(Some).map_err(|_| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "meta.schema_version is not an integer: `{s}`"
            ))
        }),
    }
}

/// Apply every pending migration to reach [`SCHEMA_VERSION`]. Refuses with
/// [`TomeError::SchemaTooNew`] when the stored version is newer than this
/// binary understands.
pub fn apply_pending(conn: &mut Connection, stored: u32) -> Result<(), TomeError> {
    if stored == SCHEMA_VERSION {
        return Ok(());
    }
    if stored > SCHEMA_VERSION {
        return Err(TomeError::SchemaTooNew {
            on_disk: stored,
            compiled: SCHEMA_VERSION,
        });
    }

    let mut version = stored;
    while version < SCHEMA_VERSION {
        let step = MIGRATIONS
            .iter()
            .find(|m| m.from == version)
            .ok_or_else(|| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "no migration registered for schema {version} → {}",
                    version + 1
                ))
            })?;

        let tx = conn.transaction().map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "begin migration {} → {} tx: {e}",
                step.from, step.to
            ))
        })?;
        tx.execute_batch(step.sql).map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "apply migration {} → {}: {e}",
                step.from, step.to
            ))
        })?;
        tx.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            rusqlite::params![step.to.to_string()],
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "record migration {} → {}: {e}",
                step.from, step.to
            ))
        })?;
        tx.commit().map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "commit migration {} → {}: {e}",
                step.from, step.to
            ))
        })?;

        version = step.to;
    }
    Ok(())
}

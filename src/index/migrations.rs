//! Forward-only schema migrations.
//!
//! Phase 3 / F7 lands the migration framework per
//! `specs/003-phase-3-mcp-workspaces/contracts/schema-migration.md`. The
//! registry is empty: every Tome shipped to date bootstraps directly to
//! `SCHEMA_VERSION`. Phase 4+ adds the first real `Migration` row plus an
//! exercising test under `tests/schema_migration_e2e.rs`; that test relies
//! on [`MIGRATIONS_OVERRIDE`] to inject a synthetic table without polluting
//! production state.
//!
//! Policy (research §R3, refined in P3 §R-14):
//!
//! * `current == target` — proceed.
//! * `current  > target` — refuse with [`TomeError::SchemaVersionTooNew`]
//!   (exit 73, Phase 3 dedicated refusal).
//! * `current  < target` — apply every registered step in order, each in
//!   its own transaction, under the advisory lock acquired by the caller.
//! * `meta.schema_version` absent — fresh DB; the caller runs
//!   `schema::bootstrap` instead.
//!
//! No down-migrations: older Tome refuses newer DBs. A v2 patch ships as
//! one row appended to [`MIGRATIONS`] plus a Phase 4+ entry in this
//! module's history.

use std::cell::RefCell;
use std::time::Instant;

use rusqlite::{Connection, Transaction};
use tracing::{error, info};

use crate::error::TomeError;

/// A single forward migration step. Compared to the Phase 2 shape, `apply`
/// is a function pointer (rather than `&'static str` SQL) so a migration
/// can carry post-DDL fixups inside the same transaction.
pub struct Migration {
    pub from: u32,
    pub to: u32,
    pub name: &'static str,
    pub apply: fn(&Transaction) -> Result<(), TomeError>,
}

/// Compile-time list of every migration. PHASE 3 SHIPS WITH ZERO MIGRATIONS.
pub const MIGRATIONS: &[Migration] = &[];

thread_local! {
    /// Test-only injection point. Phase 7's `tests/schema_migration_e2e.rs`
    /// registers a synthetic migration table for a single scenario, then
    /// clears the slot. Production [`apply_pending`] reads through
    /// [`active_migrations`] which falls back to [`MIGRATIONS`].
    ///
    /// Public surface intentionally — integration tests live outside the
    /// crate and `#[cfg(test)]` items aren't visible there. Doc-hidden to
    /// keep it out of the published API; the only legitimate caller is a
    /// test.
    #[doc(hidden)]
    pub static MIGRATIONS_OVERRIDE: RefCell<Option<&'static [Migration]>> =
        const { RefCell::new(None) };
}

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

/// Walk every registered migration step from `current` up to `target`. Each
/// step runs in its own transaction; on failure the transaction is dropped
/// (rolling back) and the on-disk `schema_version` row reflects the last
/// successfully-committed step. Returns the new on-disk version.
///
/// Refuses newer-on-disk with [`TomeError::SchemaVersionTooNew`] (exit 73,
/// the Phase 3 migration-framework refusal). Note: the legacy read-only
/// open path in `index::db::open_read_only` continues to surface the
/// Phase 2 `SchemaTooNew` (exit 52) for backward compatibility — both
/// gates name the same condition but route through different exit codes
/// historically.
pub fn apply_pending(conn: &mut Connection, current: u32, target: u32) -> Result<u32, TomeError> {
    if current == target {
        return Ok(current);
    }
    if current > target {
        return Err(TomeError::SchemaVersionTooNew {
            on_disk: current,
            expected: target,
        });
    }

    let migrations = active_migrations();

    let mut version = current;
    while version < target {
        let step = migrations
            .iter()
            .find(|m| m.from == version)
            .ok_or_else(|| {
                // Defensive guard against an unknown forward gap. Surfaces
                // as IndexIntegrityCheckFailure (exit 51) — distinct from
                // the dedicated "schema too new" / "migration failed"
                // variants because the DB is in a state we cannot
                // understand at all (likely a downgraded binary).
                TomeError::IndexIntegrityCheckFailure(format!(
                    "no migration registered for schema {version} → {}",
                    version + 1
                ))
            })?;

        let started = Instant::now();
        info!(
            target: "tome::index::migrations",
            from = step.from,
            to = step.to,
            name = step.name,
            "migrating",
        );

        let tx = conn
            .transaction()
            .map_err(|e| TomeError::SchemaMigrationFailed {
                from: step.from,
                to: step.to,
                source: anyhow::anyhow!("begin tx: {e}"),
            })?;

        if let Err(err) = (step.apply)(&tx) {
            error!(
                target: "tome::index::migrations",
                from = step.from,
                to = step.to,
                name = step.name,
                error = %err,
                "migration failed",
            );
            return Err(TomeError::SchemaMigrationFailed {
                from: step.from,
                to: step.to,
                source: anyhow::anyhow!(err),
            });
        }

        if let Err(e) = tx.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            rusqlite::params![step.to.to_string()],
        ) {
            error!(
                target: "tome::index::migrations",
                from = step.from,
                to = step.to,
                name = step.name,
                error = %e,
                "migration failed (record schema_version)",
            );
            return Err(TomeError::SchemaMigrationFailed {
                from: step.from,
                to: step.to,
                source: anyhow::anyhow!("record schema_version: {e}"),
            });
        }

        if let Err(e) = tx.commit() {
            error!(
                target: "tome::index::migrations",
                from = step.from,
                to = step.to,
                name = step.name,
                error = %e,
                "migration failed (commit)",
            );
            return Err(TomeError::SchemaMigrationFailed {
                from: step.from,
                to: step.to,
                source: anyhow::anyhow!("commit: {e}"),
            });
        }

        info!(
            target: "tome::index::migrations",
            from = step.from,
            to = step.to,
            name = step.name,
            elapsed_ms = started.elapsed().as_millis() as u64,
            "migration committed",
        );

        version = step.to;
    }

    Ok(version)
}

/// Resolve the registered migrations table, honouring the test override if
/// set. Production callers see [`MIGRATIONS`].
fn active_migrations() -> &'static [Migration] {
    MIGRATIONS_OVERRIDE
        .with(|slot| *slot.borrow())
        .unwrap_or(MIGRATIONS)
}

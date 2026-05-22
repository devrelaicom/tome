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

use rusqlite::{Connection, Transaction, params};
use time::OffsetDateTime;
use tracing::{error, info};

use crate::error::TomeError;
use crate::index::schema::GLOBAL_WORKSPACE;

/// A single forward migration step. Compared to the Phase 2 shape, `apply`
/// is a function pointer (rather than `&'static str` SQL) so a migration
/// can carry post-DDL fixups inside the same transaction.
pub struct Migration {
    pub from: u32,
    pub to: u32,
    pub name: &'static str,
    pub apply: fn(&Transaction) -> Result<(), TomeError>,
}

/// Compile-time list of every migration. Phase 4 / F9 registers the
/// first production migration: `phase_4_v1_to_v2` rebuilds the `skills`
/// table to drop the `enabled` column, creates the four new workspace
/// tables + indices, and seeds the privileged `global` workspace row.
/// See [`phase_4_v1_to_v2`] for the body and the M-MIG-2 audit trail.
pub const MIGRATIONS: &[Migration] = &[Migration {
    from: 1,
    to: 2,
    name: "phase-4-central-db-refactor",
    apply: phase_4_v1_to_v2,
}];

/// The schema v1 → v2 migration body. Implements the SQLite "12-step"
/// table-rebuild pattern for dropping the `skills.enabled` column (which
/// has both an index and an implicit CHECK constraint blocking
/// `ALTER TABLE ... DROP COLUMN`), creates the four new workspace
/// tables + indices, and seeds the privileged `global` workspace row.
///
/// **M-MIG-2 audit** (per reviewer fold-in): the Phase 2/3 `skills` table
/// carried three indices: `idx_skills_catalog_plugin`,
/// `idx_skills_enabled`, and `idx_skills_content_hash`. The migration
/// preserves the catalog/plugin and content_hash indices; it drops
/// `idx_skills_enabled` (the column it indexes no longer exists). No
/// triggers or views were registered on `skills` in Phase 2/3.
///
/// `apply_pending` updates `meta.schema_version` to `2` after this `fn`
/// returns `Ok` — the migration body does NOT touch the version row.
fn phase_4_v1_to_v2(tx: &Transaction) -> Result<(), TomeError> {
    // Create the new workspace tables + indices first so the `skills`
    // rebuild that follows can run with foreign-key enforcement disabled
    // (the new `workspace_skills.skill_id` references `skills(id)` and
    // would cascade on DROP TABLE; FK is OFF for the rebuild block).
    tx.execute_batch(
        "CREATE TABLE workspaces (
            id            INTEGER PRIMARY KEY AUTOINCREMENT,
            name          TEXT UNIQUE NOT NULL,
            created_at    INTEGER NOT NULL,
            last_used_at  INTEGER NOT NULL
         );

         CREATE TABLE workspace_skills (
            workspace_id  INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            skill_id      INTEGER NOT NULL REFERENCES skills(id)     ON DELETE CASCADE,
            enabled_at    INTEGER NOT NULL,
            PRIMARY KEY (workspace_id, skill_id)
         );

         CREATE TABLE workspace_catalogs (
            workspace_id  INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            catalog_name  TEXT NOT NULL,
            url           TEXT NOT NULL,
            pinned_ref    TEXT NOT NULL,
            PRIMARY KEY (workspace_id, catalog_name)
         );

         CREATE TABLE workspace_projects (
            project_path  TEXT PRIMARY KEY NOT NULL,
            workspace_id  INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
            bound_at      INTEGER NOT NULL
         );

         CREATE INDEX idx_workspace_projects_workspace ON workspace_projects(workspace_id);
         CREATE INDEX idx_workspace_skills_skill       ON workspace_skills(skill_id);
         CREATE INDEX idx_workspace_catalogs_url       ON workspace_catalogs(url);",
    )
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!(
            "phase_4_v1_to_v2: create workspace tables: {e}"
        ))
    })?;

    // Seed the privileged `global` workspace row (FR-323). Phase 3 wipe
    // (FR-304) guarantees no developer-meaningful timestamp to inherit —
    // we use the migration time.
    let now_unix = OffsetDateTime::now_utc().unix_timestamp();
    tx.execute(
        "INSERT INTO workspaces (name, created_at, last_used_at) VALUES (?1, ?2, ?2)",
        params![GLOBAL_WORKSPACE, now_unix],
    )
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!(
            "phase_4_v1_to_v2: seed global workspace: {e}"
        ))
    })?;

    // SQLite "12-step" rebuild of the `skills` table to drop the
    // `enabled` column. FK enforcement is off for the rebuild block
    // because `DROP TABLE skills` would otherwise cascade through any
    // FK references; `workspace_skills` was created in the previous
    // step but is empty at this point so the cascade would be a no-op,
    // but disabling FK is still the documented SQLite recipe.
    tx.execute_batch(
        "PRAGMA foreign_keys = OFF;

         DROP INDEX IF EXISTS idx_skills_enabled;

         CREATE TABLE skills_new (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            catalog         TEXT NOT NULL,
            plugin          TEXT NOT NULL,
            name            TEXT NOT NULL,
            description     TEXT NOT NULL,
            plugin_version  TEXT NOT NULL,
            path            TEXT NOT NULL,
            content_hash    TEXT NOT NULL,
            indexed_at      INTEGER NOT NULL,
            UNIQUE (catalog, plugin, name)
         );

         INSERT INTO skills_new (id, catalog, plugin, name, description,
                                 plugin_version, path, content_hash, indexed_at)
         SELECT id, catalog, plugin, name, description,
                plugin_version, path, content_hash, indexed_at
         FROM skills;

         DROP TABLE skills;
         ALTER TABLE skills_new RENAME TO skills;

         -- Recreate every non-`enabled` index that existed on Phase 2/3
         -- `skills` (audit per the M-MIG-2 trail above). `IF NOT EXISTS`
         -- handles the case where the test harness downgrade-stamps a
         -- v2-bootstrapped DB back to v1 (Phase 4's doctor-fix e2e):
         -- the v2 bootstrap already created these indices, and the
         -- DROP TABLE + ALTER TABLE rename block above drops only the
         -- old `skills` table — its indices are removed by the DROP,
         -- but in the downgrade-then-rerun case the v2 bootstrap's
         -- pre-existing indices may resurface on the renamed table.
         CREATE INDEX IF NOT EXISTS idx_skills_catalog_plugin ON skills(catalog, plugin);
         CREATE INDEX IF NOT EXISTS idx_skills_content_hash   ON skills(content_hash);

         PRAGMA foreign_keys = ON;",
    )
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("phase_4_v1_to_v2: rebuild skills: {e}"))
    })?;

    Ok(())
}

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

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

/// Compile-time list of every migration. Phase 4 / F9 registered the first
/// production migration: `phase_4_v1_to_v2` rebuilds the `skills` table
/// to drop the `enabled` column, creates the four new workspace tables +
/// indices, and seeds the privileged `global` workspace row. Phase 5 /
/// US1.a registers the second: `phase_5_v2_to_v3` widens the `skills`
/// identity tuple with a `kind` discriminator and adds the new
/// `searchable`, `user_invocable`, and `when_to_use` columns. Phase 6
/// registers the third: `phase_6_v3_to_v4`, a marker-only no-op that
/// advances the version because the free-text `kind` column already admits
/// the new `'agent'` value without DDL (entry-schema-p6.md). Phase 11
/// registers the fourth: `phase_11_v4_to_v5`, adding `workspace_skills.tier`
/// for tiered skill routing (every pre-existing enrolment defaults to tier 3).
pub const MIGRATIONS: &[Migration] = &[
    Migration {
        from: 1,
        to: 2,
        name: "phase-4-central-db-refactor",
        apply: phase_4_v1_to_v2,
    },
    Migration {
        from: 2,
        to: 3,
        name: "phase5_entry_kind_unification",
        apply: phase_5_v2_to_v3,
    },
    Migration {
        from: 3,
        to: 4,
        name: "phase6_kind_domain_agent_marker",
        apply: phase_6_v3_to_v4,
    },
    Migration {
        from: 4,
        to: 5,
        name: "phase11_workspace_skills_tier",
        apply: phase_11_v4_to_v5,
    },
];

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

/// The schema v2 → v3 migration body (Phase 5 / US1.a). Unifies the
/// `skills` table into a kind-discriminated entry store per
/// `contracts/schema-migration-p5.md` and `contracts/entry-schema-p5.md`.
///
/// The contract's authoritative DDL is:
///
/// ```sql
/// ALTER TABLE skills ADD COLUMN kind TEXT NOT NULL DEFAULT 'skill';
/// ALTER TABLE skills ADD COLUMN searchable INTEGER NOT NULL DEFAULT 1;
/// ALTER TABLE skills ADD COLUMN user_invocable INTEGER NOT NULL DEFAULT 0;
/// ALTER TABLE skills ADD COLUMN when_to_use TEXT;
/// DROP INDEX IF EXISTS skills_unique;
/// CREATE UNIQUE INDEX skills_unique ON skills (catalog, plugin, kind, name);
/// ```
///
/// **Contract amendment (US1.a discovery):** the schema-v2 `skills` table
/// expresses uniqueness via an inline `UNIQUE (catalog, plugin, name)`
/// table constraint, which SQLite materialises as an auto-named
/// `sqlite_autoindex_skills_*` rather than a developer-named index. The
/// contract's `DROP INDEX IF EXISTS skills_unique` therefore no-ops on a
/// real v2 DB, leaving the old narrow constraint in force — defeating
/// the kind-discriminated identity model. The migration body below
/// follows the SQLite "12-step" table-rebuild pattern (same approach
/// Phase 4's `phase_4_v1_to_v2` used to drop the `enabled` column) so
/// the resulting `skills` table has the widened identity tuple as the
/// only unique constraint. The contract's intent is preserved
/// byte-for-byte; only the mechanism changes.
fn phase_5_v2_to_v3(tx: &Transaction) -> Result<(), TomeError> {
    tx.execute_batch(
        "PRAGMA foreign_keys = OFF;

         CREATE TABLE skills_new (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            catalog         TEXT NOT NULL,
            plugin          TEXT NOT NULL,
            name            TEXT NOT NULL,
            kind            TEXT NOT NULL DEFAULT 'skill',
            description     TEXT NOT NULL,
            plugin_version  TEXT NOT NULL,
            path            TEXT NOT NULL,
            content_hash    TEXT NOT NULL,
            searchable      INTEGER NOT NULL DEFAULT 1,
            user_invocable  INTEGER NOT NULL DEFAULT 0,
            when_to_use     TEXT,
            indexed_at      INTEGER NOT NULL
         );

         -- Backfill per `contracts/schema-migration-p5.md` § Backfill:
         -- every pre-existing row keeps its identity, gains
         -- `kind = 'skill'` + default `searchable = 1` +
         -- `user_invocable = 0` + `when_to_use = NULL`.
         INSERT INTO skills_new
            (id, catalog, plugin, name, kind, description,
             plugin_version, path, content_hash,
             searchable, user_invocable, when_to_use, indexed_at)
         SELECT
             id, catalog, plugin, name, 'skill', description,
             plugin_version, path, content_hash,
             1, 0, NULL, indexed_at
         FROM skills;

         DROP TABLE skills;
         ALTER TABLE skills_new RENAME TO skills;

         -- Recreate every non-uniqueness index that existed on the v2
         -- `skills` table. `IF NOT EXISTS` mirrors the v1→v2 migration's
         -- defensive shape (handles the downgrade-stamp test path).
         CREATE INDEX IF NOT EXISTS idx_skills_catalog_plugin ON skills(catalog, plugin);
         CREATE INDEX IF NOT EXISTS idx_skills_content_hash   ON skills(content_hash);

         -- Widened identity tuple as the sole unique constraint. Same
         -- index name a fresh v3 bootstrap uses (see
         -- `schema::CREATE_STATEMENTS`).
         CREATE UNIQUE INDEX skills_unique ON skills (catalog, plugin, kind, name);

         PRAGMA foreign_keys = ON;",
    )
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("phase_5_v2_to_v3: rebuild skills: {e}"))
    })?;

    Ok(())
}

/// The schema v3 → v4 migration body (Phase 6). A documented **no-op
/// marker**: the `skills.kind` column is free-text TEXT, so admitting the
/// new `'agent'` domain value needs no DDL and no data backfill
/// (entry-schema-p6.md § "Schema migration"). The migration exists only to
/// advance the version monotonically so doctor's schema check and the
/// migration registry agree the `kind` domain widened. `apply_pending`
/// records `meta.schema_version = 4` after this returns `Ok`; the body
/// itself touches nothing.
fn phase_6_v3_to_v4(_tx: &Transaction) -> Result<(), TomeError> {
    Ok(())
}

/// The schema v4 → v5 migration body (Phase 11 / tiered skill routing). A
/// purely additive `ALTER TABLE ... ADD COLUMN`: `workspace_skills` carries no
/// index or CHECK constraint that blocks ADD COLUMN (unlike the `skills`
/// rebuilds above), so no 12-step rebuild is needed. Every pre-existing
/// enrolment row gains `tier = 3` (the default routing tier). `apply_pending`
/// records `meta.schema_version = 5` after this returns `Ok`.
fn phase_11_v4_to_v5(tx: &Transaction) -> Result<(), TomeError> {
    tx.execute_batch("ALTER TABLE workspace_skills ADD COLUMN tier INTEGER NOT NULL DEFAULT 3;")
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "phase_11_v4_to_v5: add workspace_skills.tier: {e}"
            ))
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

/// Read `meta.schema_version`. Returns `None` only when the `meta` table
/// itself does not exist (a genuinely fresh DB → the caller bootstraps).
/// Returns `Some(version)` when the row is present and parses.
///
/// FR-015 (F-BOOT-META-DIAG): a `meta` table that EXISTS but is **missing**
/// its `schema_version` row is corruption, NOT a fresh DB — distinguished
/// explicitly here rather than collapsed to `None` by a blanket `.ok()`.
/// The old behaviour mis-routed a half-written DB into the bootstrap path,
/// which then failed with a misleading "table meta already exists". Both the
/// missing-row and unparsable-value cases reuse the existing
/// [`TomeError::IndexIntegrityCheckFailure`] (exit 51); no new variant/code.
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
        // Genuinely fresh DB: no `meta` table at all. The caller bootstraps.
        return Ok(None);
    }

    // The `meta` table exists, so a missing `schema_version` row is corruption
    // (a half-bootstrapped or tampered DB), explicitly distinct from the
    // fresh-DB case above. A query error other than "no rows" is likewise a
    // real failure to surface — neither is swallowed into `Ok(None)`.
    let raw: String = match conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |row| row.get::<_, String>(0),
    ) {
        Ok(value) => value,
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            return Err(TomeError::IndexIntegrityCheckFailure(
                "meta table exists but the `schema_version` row is missing \
                 (database is corrupt, not fresh)"
                    .to_string(),
            ));
        }
        Err(e) => {
            return Err(TomeError::IndexIntegrityCheckFailure(format!(
                "read meta.schema_version: {e}"
            )));
        }
    };

    raw.parse::<u32>().map(Some).map_err(|_| {
        TomeError::IndexIntegrityCheckFailure(format!(
            "meta.schema_version is not an integer: `{raw}`"
        ))
    })
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

        // Disable FK enforcement around the per-step transaction. Two of
        // the registered migrations (phase_4_v1_to_v2 and
        // phase_5_v2_to_v3) use the SQLite "12-step" table-rebuild
        // pattern that DROPs the `skills` table and recreates it; with
        // FKs enabled the DROP cascades through `workspace_skills` via
        // ON DELETE CASCADE, wiping the very rows the migration is
        // trying to preserve. `PRAGMA foreign_keys` cannot be set
        // INSIDE a transaction (SQLite silently ignores), so the
        // migration-body's PRAGMA statements are belt-and-braces only;
        // the authoritative toggle is here. Restored to ON on the
        // success path so post-migration writes still get FK
        // enforcement. The setting is per-connection — Phase-3 contract
        // notes the migration framework owns connection PRAGMAs around
        // each step.
        let prior_fk = read_fk_pragma(conn).unwrap_or(1);
        if let Err(e) = conn.pragma_update(None, "foreign_keys", "OFF") {
            return Err(TomeError::SchemaMigrationFailed {
                from: step.from,
                to: step.to,
                source: anyhow::anyhow!("disable foreign_keys: {e}"),
            });
        }

        // Run the per-step work + commit inside a closure so the FK
        // PRAGMA can be restored on every exit path (success or failure)
        // before we return from `apply_pending`. Restoring FK on the
        // failure path matters because the migration framework's caller
        // (typically `index::db::open`) keeps the connection open for
        // subsequent reads/writes; leaving FKs OFF would silently relax
        // the runtime invariant the rest of the binary relies on.
        let step_result: Result<(), TomeError> = (|| {
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
            Ok(())
        })();

        // Restore the prior FK state on every exit path. Best-effort —
        // a failure here is itself surfaced as a migration error so the
        // caller doesn't carry a half-PRAGMA'd connection forward.
        let restore = conn.pragma_update(
            None,
            "foreign_keys",
            if prior_fk != 0 { "ON" } else { "OFF" },
        );

        step_result?;
        if let Err(e) = restore {
            return Err(TomeError::SchemaMigrationFailed {
                from: step.from,
                to: step.to,
                source: anyhow::anyhow!("restore foreign_keys: {e}"),
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

/// Read `PRAGMA foreign_keys` (returns `0` or `1`). Used by
/// [`apply_pending`] to capture the prior FK state before a step's
/// transaction so it can be restored on commit/rollback. Errors are
/// folded by callers (default = ON = 1).
fn read_fk_pragma(conn: &Connection) -> Result<i64, rusqlite::Error> {
    conn.query_row("PRAGMA foreign_keys", [], |row| row.get(0))
}

#[cfg(test)]
mod tier_migration_tests {
    use rusqlite::Connection;

    #[test]
    fn v4_to_v5_adds_tier_default_three() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
             INSERT INTO meta (key, value) VALUES ('schema_version', '4');
             CREATE TABLE workspaces (
                id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT UNIQUE NOT NULL,
                created_at INTEGER NOT NULL, last_used_at INTEGER NOT NULL);
             INSERT INTO workspaces (name, created_at, last_used_at) VALUES ('global', 0, 0);
             CREATE TABLE skills (
                id INTEGER PRIMARY KEY AUTOINCREMENT, catalog TEXT NOT NULL,
                plugin TEXT NOT NULL, name TEXT NOT NULL, kind TEXT NOT NULL DEFAULT 'skill',
                description TEXT NOT NULL, plugin_version TEXT NOT NULL, path TEXT NOT NULL,
                content_hash TEXT NOT NULL, searchable INTEGER NOT NULL DEFAULT 1,
                user_invocable INTEGER NOT NULL DEFAULT 0, when_to_use TEXT,
                indexed_at INTEGER NOT NULL);
             INSERT INTO skills (catalog, plugin, name, description, plugin_version, path, content_hash, indexed_at)
                VALUES ('cat', 'plug', 'sk', 'd', '1.0.0', 'skills/sk/SKILL.md', 'h', 0);
             CREATE TABLE workspace_skills (
                workspace_id INTEGER NOT NULL, skill_id INTEGER NOT NULL,
                enabled_at INTEGER NOT NULL, PRIMARY KEY (workspace_id, skill_id));
             INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (1, 1, 0);",
        )
        .unwrap();

        let new_version = super::apply_pending(&mut conn, 4, 5).unwrap();
        assert_eq!(new_version, 5);

        let tier: i64 = conn
            .query_row("SELECT tier FROM workspace_skills WHERE skill_id = 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tier, 3, "pre-existing rows default to Tier 3");
    }
}

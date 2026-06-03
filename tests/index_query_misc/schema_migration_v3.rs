//! Phase 5 / US1.a — schema v2 → v3 migration end-to-end coverage.
//!
//! Exercises the production [`tome::index::migrations::MIGRATIONS`] entry
//! `phase5_entry_kind_unification` against a synthetic v2 database
//! produced by the Phase-3 helper [`write_index_db_with_schema_version`].
//! All assertions follow `specs/005-phase-5-commands-prompts/contracts/
//! schema-migration-p5.md` § Testing.
//!
//! Companion file `tests/schema_migration_e2e.rs` (Phase 3 / US5) covers
//! the framework itself via `MIGRATIONS_OVERRIDE`-injected synthetic
//! migrations; this file covers the REAL Phase 5 migration end-to-end.

use crate::common::write_index_db_with_schema_version;
use rusqlite::{Connection, Transaction, params};
use tempfile::TempDir;
use tome::error::TomeError;
use tome::index::Migration;
use tome::index::migrations::{MIGRATIONS, MIGRATIONS_OVERRIDE, apply_pending};

/// Same RAII guard the companion file uses. Repeated here so this file is
/// self-contained; lives at the top of every migration test file as a
/// convention.
struct MigrationsGuard;

impl MigrationsGuard {
    fn install(migrations: &'static [Migration]) -> Self {
        MIGRATIONS_OVERRIDE.with(|slot| *slot.borrow_mut() = Some(migrations));
        Self
    }
}

impl Drop for MigrationsGuard {
    fn drop(&mut self) {
        MIGRATIONS_OVERRIDE.with(|slot| *slot.borrow_mut() = None);
    }
}

// ---------------------------------------------------------------------------
// Helpers.
// ---------------------------------------------------------------------------

/// Bootstrap a synthetic v2 `skills` table that matches Phase 4's
/// post-migration shape: every v3 column is absent, the `UNIQUE (catalog,
/// plugin, name)` table constraint is present (which SQLite materialises
/// as `sqlite_autoindex_skills_*`, NOT a developer-named
/// `skills_unique` — see the contract amendment in
/// `src/index/migrations.rs::phase_5_v2_to_v3`).
fn bootstrap_v2_skills_table(conn: &Connection) {
    conn.execute_batch(
        "CREATE TABLE skills (
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
        CREATE INDEX idx_skills_catalog_plugin ON skills(catalog, plugin);
        CREATE INDEX idx_skills_content_hash   ON skills(content_hash);",
    )
    .expect("bootstrap synthetic v2 skills table");
}

/// Insert a known skill row by way of the v2 column list.
fn insert_v2_skill(conn: &Connection, catalog: &str, plugin: &str, name: &str) {
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, description, plugin_version, path,
             content_hash, indexed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            catalog,
            plugin,
            name,
            "first skill",
            "1.0.0",
            "/skills/first/SKILL.md",
            "abc",
            "2026-05-26T00:00:00Z",
        ],
    )
    .expect("insert synthetic v2 row");
}

fn schema_version(conn: &Connection) -> String {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |row| row.get::<_, String>(0),
    )
    .expect("read schema_version")
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn v2_to_v3_happy_path_bumps_schema_version() {
    let _guard = MigrationsGuard::install(MIGRATIONS);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    write_index_db_with_schema_version(&path, 2);
    {
        let conn = Connection::open(&path).expect("open synthetic v2");
        bootstrap_v2_skills_table(&conn);
        insert_v2_skill(&conn, "c1", "p1", "s1");
    }

    let mut conn = Connection::open(&path).expect("re-open");
    let new = apply_pending(&mut conn, 2, 3).expect("apply v2→v3");
    assert_eq!(new, 3, "apply_pending must return the new version");
    assert_eq!(schema_version(&conn), "3");
}

#[test]
fn backfill_defaults_apply_to_pre_existing_rows() {
    let _guard = MigrationsGuard::install(MIGRATIONS);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    write_index_db_with_schema_version(&path, 2);
    {
        let conn = Connection::open(&path).expect("open");
        bootstrap_v2_skills_table(&conn);
        insert_v2_skill(&conn, "c1", "p1", "s1");
    }

    let mut conn = Connection::open(&path).expect("re-open");
    apply_pending(&mut conn, 2, 3).expect("apply v2→v3");

    let (kind, searchable, user_invocable, when_to_use): (String, i64, i64, Option<String>) = conn
        .query_row(
            "SELECT kind, searchable, user_invocable, when_to_use FROM skills WHERE name = ?1",
            params!["s1"],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("read backfilled row");

    assert_eq!(kind, "skill", "pre-existing rows default to kind='skill'");
    assert_eq!(searchable, 1, "pre-existing rows default to searchable=1");
    assert_eq!(
        user_invocable, 0,
        "pre-existing rows default to user_invocable=0"
    );
    assert!(
        when_to_use.is_none(),
        "pre-existing rows have when_to_use=NULL"
    );
}

#[test]
fn identity_preservation_keeps_catalog_plugin_name_tuple_intact() {
    let _guard = MigrationsGuard::install(MIGRATIONS);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    write_index_db_with_schema_version(&path, 2);
    {
        let conn = Connection::open(&path).expect("open");
        bootstrap_v2_skills_table(&conn);
        insert_v2_skill(&conn, "c1", "p1", "s1");
    }

    let mut conn = Connection::open(&path).expect("re-open");
    apply_pending(&mut conn, 2, 3).expect("apply v2→v3");

    let (catalog, plugin, name): (String, String, String) = conn
        .query_row("SELECT catalog, plugin, name FROM skills", [], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("read post-migration row");
    assert_eq!(catalog, "c1");
    assert_eq!(plugin, "p1");
    assert_eq!(name, "s1");

    // Post-migration unique constraint widens to `(catalog, plugin, kind,
    // name)` — same-name-different-kind insert must succeed without
    // violating the unique index.
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, indexed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            "c1",
            "p1",
            "s1",
            "command",
            "twin command",
            "1.0.0",
            "/commands/s1.md",
            "def",
            "2026-05-26T00:00:00Z",
        ],
    )
    .expect("same-name-different-kind must not violate widened uniqueness");

    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM skills", [], |row| row.get(0))
        .expect("count");
    assert_eq!(total, 2, "after the second insert there must be two rows");

    // The narrow (catalog, plugin, name) tuple WITHOUT kind must still
    // refuse a duplicate skill — i.e. the widened unique index correctly
    // refuses a second 'skill'-kind row with the same name.
    let dup_err = conn
        .execute(
            "INSERT INTO skills
                (catalog, plugin, name, kind, description, plugin_version,
                 path, content_hash, indexed_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                "c1",
                "p1",
                "s1",
                "skill",
                "duplicate skill",
                "1.0.0",
                "/skills/dup/SKILL.md",
                "ghi",
                "2026-05-26T00:00:00Z",
            ],
        )
        .expect_err("duplicate (c1,p1,'skill',s1) must violate skills_unique");
    let msg = dup_err.to_string();
    assert!(
        msg.contains("UNIQUE") || msg.contains("constraint"),
        "expected unique-constraint violation, got: {msg}",
    );
}

#[test]
fn fk_references_from_workspace_skills_survive_migration() {
    // Build a v2 DB with the full Phase-4 workspace tables + a
    // `workspace_skills` row referencing the seed skill. After
    // migrating to v3 the FK must still resolve (the skill row keeps its
    // `id`, the migration rebuilds the table but preserves identifiers).
    let _guard = MigrationsGuard::install(MIGRATIONS);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    write_index_db_with_schema_version(&path, 2);
    {
        let conn = Connection::open(&path).expect("open");
        bootstrap_v2_skills_table(&conn);
        conn.execute_batch(
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
            );",
        )
        .expect("v2 workspace tables");
        conn.execute(
            "INSERT INTO workspaces (name, created_at, last_used_at) VALUES ('global', 0, 0)",
            [],
        )
        .expect("seed global");
        insert_v2_skill(&conn, "c1", "p1", "s1");
        // The seeded skill row has `id = 1` (AUTOINCREMENT on a fresh
        // table). Reference it from `workspace_skills`.
        conn.execute(
            "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at)
             VALUES ((SELECT id FROM workspaces WHERE name='global'), 1, 0)",
            [],
        )
        .expect("seed workspace_skills");
    }

    let mut conn = Connection::open(&path).expect("re-open");
    apply_pending(&mut conn, 2, 3).expect("apply v2→v3");

    // The FK target row must still be resolvable via the original `id`.
    let join_count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM workspace_skills ws
             JOIN skills s ON s.id = ws.skill_id",
            [],
            |row| row.get(0),
        )
        .expect("join through FK");
    assert_eq!(
        join_count, 1,
        "the workspace_skills row's FK must still resolve to the migrated skill row",
    );
}

#[test]
fn fresh_v2_db_with_no_skills_rows_still_migrates_cleanly() {
    let _guard = MigrationsGuard::install(MIGRATIONS);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    write_index_db_with_schema_version(&path, 2);
    {
        let conn = Connection::open(&path).expect("open");
        bootstrap_v2_skills_table(&conn);
    }

    let mut conn = Connection::open(&path).expect("re-open");
    apply_pending(&mut conn, 2, 3).expect("apply v2→v3 on empty table");
    assert_eq!(schema_version(&conn), "3");

    // Insert via the new column list — kind discriminator + defaults.
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, indexed_at)
         VALUES ('c','p','s','skill','d','1','/p','h',0)",
        [],
    )
    .expect("post-migration insert with kind column");
}

#[test]
fn mid_transaction_failure_rolls_back_v2_state_unchanged() {
    // Synthetic single-step migration that always fails. The framework
    // must drop the partial transaction; the on-disk DB remains at v2
    // with the seeded rows visible.
    static FAILING: &[Migration] = &[Migration {
        from: 2,
        to: 3,
        name: "test_v2_to_v3_always_fails",
        apply: migrate_v2_to_v3_always_fails,
    }];
    let _guard = MigrationsGuard::install(FAILING);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    write_index_db_with_schema_version(&path, 2);
    {
        let conn = Connection::open(&path).expect("open");
        bootstrap_v2_skills_table(&conn);
        insert_v2_skill(&conn, "c1", "p1", "s1");
    }

    let mut conn = Connection::open(&path).expect("re-open");
    let err = apply_pending(&mut conn, 2, 3).expect_err("failing migration must propagate");
    assert!(matches!(
        err,
        TomeError::SchemaMigrationFailed { from: 2, to: 3, .. }
    ));

    // schema_version remains 2.
    assert_eq!(schema_version(&conn), "2");

    // The scratch table from inside the failing migration must NOT
    // exist on disk (its CREATE TABLE landed inside the rolled-back tx).
    let scratch_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='v3_scratch_should_not_exist'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(
        !scratch_exists,
        "mid-tx failure must drop the partial migration's writes",
    );
}

fn migrate_v2_to_v3_always_fails(tx: &Transaction) -> Result<(), TomeError> {
    tx.execute_batch("CREATE TABLE v3_scratch_should_not_exist (id INTEGER PRIMARY KEY) STRICT")
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("scratch: {e}")))?;
    Err(TomeError::IndexIntegrityCheckFailure(
        "deliberate test failure".into(),
    ))
}

# Schema Migration v1 → v2 — Contract

**Spec source**: [spec.md FR-580 through FR-581 + FR-327](../spec.md)
**Research**: [research.md R-7](../research.md)

The Phase 3 forward-migration framework debuts its first registered production migration in Phase 4.

## Framework recap (unchanged from Phase 3)

```rust
pub struct Migration {
    pub from: u32,
    pub to: u32,
    pub name: &'static str,
    pub apply: fn(&Transaction) -> Result<(), TomeError>,
}

pub const MIGRATIONS: &[Migration] = &[
    Migration {
        from: 1, to: 2,
        name: "phase-4-central-db-refactor",
        apply: phase_4_v1_to_v2,
    },
];

pub fn apply_pending(conn: &mut Connection, current: u32, target: u32)
    -> Result<u32, TomeError>;
```

Behaviour: `current == target` → no-op. `current > target` → exit 73 (`SchemaVersionTooNew`). `current < target` → walk `MIGRATIONS` filtered by `from >= current && to <= target`, apply each inside its own `Transaction`; failure rolls that transaction back and returns the last-good intermediate version via `SchemaMigrationFailed`.

## Migration as a named `fn`

Per R-7 + P7 retro recommendation (named `fn`, not closure):

```rust
fn phase_4_v1_to_v2(tx: &Transaction) -> Result<(), TomeError> {
    // FK constraints are off for the duration of this migration so the
    // skills-table rebuild (below) can DROP+RENAME without tripping the new
    // workspace_skills.skill_id FK. WAL transactions honour PRAGMA toggled
    // inside them; the re-enable is the last statement.
    tx.execute_batch(r#"
        PRAGMA foreign_keys = OFF;

        -- Phase 4 new tables
        CREATE TABLE workspaces (
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
        CREATE INDEX idx_workspace_skills_skill      ON workspace_skills(skill_id);
        CREATE INDEX idx_workspace_catalogs_url      ON workspace_catalogs(url);

        -- Skills table rebuild (SQLite "12-step" pattern):
        -- ALTER TABLE skills DROP COLUMN enabled fails because the column is
        -- indexed (idx_skills_enabled) AND constrained (CHECK enabled IN (0,1)).
        -- We rebuild the table without the column instead.

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

        -- Recreate every non-enabled Phase 2/3 index that lived on skills.
        -- (See src/index/schema.rs for the canonical list; any change to
        -- Phase 2/3 indexes requires updating this migration in lockstep.)
        CREATE INDEX idx_skills_catalog_plugin ON skills(catalog, plugin);

        PRAGMA foreign_key_check;
        PRAGMA foreign_keys = ON;
    "#)?;

    // Seed the privileged global workspace row.
    let now = unix_seconds_now();
    tx.execute(
        "INSERT INTO workspaces (name, created_at, last_used_at) VALUES (?, ?, ?)",
        params!["global", now, now],
    )?;

    // Update schema_version row (handled by apply_pending after this fn returns Ok)
    Ok(())
}
```

The implementer MUST audit `src/index/schema.rs` for every non-`enabled` index/trigger on `skills` and add `CREATE` statements for each in the rebuild section above. The `idx_skills_catalog_plugin` line is illustrative; the actual list comes from Phase 3's schema.

`apply_pending` updates `meta.schema_version` to `2` after the migration's transaction commits — outside this `fn`.

## Structural-only contract (FR-327)

The migration is **structural only**. It MUST NOT attempt to copy data from any Phase 3-shaped state (e.g. per-workspace `index.db` files at `<workspace>/.tome/index.db`). The Phase 3 wipe (FR-304) is the architectural contract that makes this safe: no Phase 3 user database with developer data is ever opened by a Phase 4 binary.

Concrete consequences:

- `skills.enabled = 1` rows from Phase 3 (if any test fixture has them) are **not** translated to `workspace_skills` rows. The column is dropped.
- Phase 3 `catalogs.toml` from `<workspace>/.tome/` is not read or migrated.
- The seeded `global` workspace row is inserted with `created_at = now`, `last_used_at = now` — there is no historical timestamp to inherit.

## Bootstrap path

A fresh install (no DB on disk) goes through `index::schema::bootstrap` directly, which emits the v2 schema in one transaction. The migration framework is not invoked for bootstrap; only for opening an existing DB whose recorded version is older.

## SQLite version requirement

The migration does NOT use `ALTER TABLE ... DROP COLUMN` (see the table-rebuild rationale above — the `enabled` column has both an index and a CHECK constraint, which block direct `DROP COLUMN`). The 12-step rebuild pattern works on every SQLite version Tome's `rusqlite` bundles. No version-pin assertion is required.

## Synthetic-fixture e2e testing

Phase 3's `tests/schema_migration_e2e.rs` continues to exercise the framework via `MIGRATIONS_OVERRIDE` thread-local injection. Phase 4 adds:

- `tests/migration_v1_to_v2.rs`: real production migration tested against a synthetic v1 fixture DB. Asserts: `workspaces` / `workspace_skills` / `workspace_catalogs` / `workspace_projects` tables exist; `skills.enabled` column is absent; `meta.schema_version = "2"`; the seeded `global` workspace row is present; rollback on injected SQL failure leaves schema at 1 with none of the new tables present.

## Removal of Phase 3 synthetic injection

Phase 3 shipped `tests/doctor.rs::fix_runs_forward_schema_migration_end_to_end` with a synthetic `SuggestedFix { subsystem: "schema", auto_fixable: true }` injection because no production trigger existed. Phase 4's first registered migration provides the natural trigger via `doctor::build_suggested_fixes` (which now emits a `Subsystem::Schema` suggested fix when `index.schema_version < SCHEMA_VERSION`). The synthetic injection is removed in Phase 4; the test now relies on the real production path.

## Refusal of newer-on-disk

Unchanged from Phase 3 FR-182: a DB recording schema version 3+ opened by a Phase 4 binary refuses with code 73. The framework MUST NOT attempt backward migration.

## Concurrency

Migration runs under the central DB's advisory lockfile. Readers opened against schema v1 mid-migration follow SQLite WAL semantics (FR-325 carryover): they either complete against pre-migration schema OR fail with `SchemaVersionTooNew` once the migration commits; they MUST NEVER observe a half-migrated schema.

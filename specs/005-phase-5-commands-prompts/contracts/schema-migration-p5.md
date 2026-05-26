# Phase 5 — Schema migration (v2 → v3)

Authoritative DDL and backfill rules for the Phase 5 schema change. Lands as the second registered migration in the Phase 3 forward-only migration framework.

## Migration record

```rust
Migration {
    from: 2,
    to: 3,
    name: "phase5_entry_kind_unification",
    apply: phase5_v3_apply,
}
```

Registered alongside Phase 4's v1→v2 in `src/index/migrations.rs::MIGRATIONS`.

## DDL (run inside a single transaction)

```sql
ALTER TABLE skills ADD COLUMN kind TEXT NOT NULL DEFAULT 'skill';
ALTER TABLE skills ADD COLUMN searchable INTEGER NOT NULL DEFAULT 1;
ALTER TABLE skills ADD COLUMN user_invocable INTEGER NOT NULL DEFAULT 0;
ALTER TABLE skills ADD COLUMN when_to_use TEXT;
DROP INDEX IF EXISTS skills_unique;
CREATE UNIQUE INDEX skills_unique ON skills (catalog, plugin, kind, name);
```

## Backfill semantics (FR-111a)

After the ALTER TABLE statements:
- Every pre-existing row has `kind = 'skill'` (the column default).
- Every pre-existing row has `searchable = 1` (the column default).
- Every pre-existing row has `user_invocable = 0` (the column default).
- Every pre-existing row has `when_to_use = NULL` (column added without default).

No re-embedding is triggered by the migration. The next reindex against any plugin will:
1. Recompute `embedding_text` per the new composer (Phase 5 §R-12); rows whose `when_to_use` newly appears will have a different `embedding_text` from the indexed version.
2. Re-embed only rows whose `content_hash` changed.

## Identity preservation

Pre-migration unique constraint: `(catalog, plugin, name)`.
Post-migration unique constraint: `(catalog, plugin, kind, name)`.

For any pre-existing row `(C, P, N)`, the post-migration identity is `(C, P, 'skill', N)`. Existing FK references (e.g. from `workspace_skills`) continue to resolve because the FK uses the `(catalog, plugin, name)` triple AND the migrated row is the only row matching that triple with the default kind. The unique constraint widening does not create ambiguity for pre-existing FK references.

## Schema version meta

The `meta` table's `schema_version` row is updated from `2` to `3` inside the same transaction by the migration framework (existing mechanism per Phase 3 / US5).

## Read-side compatibility

Read-only opens of a v3 DB from a v3-aware binary succeed. Read-only opens against a v3 DB from a v2-aware binary fail with `SchemaTooNew` (exit code 52 carryover from Phase 2). The Phase 5 binary must always see v3.

## Backwards-incompatibility note

Phase 5's migration is forward-only (per the migration framework). Downgrading from a v3 DB to a v2 binary requires manually dropping the new columns; not supported by Tome tooling.

## Failure modes

| Failure | Exit code | Handling |
|---|---|---|
| `ALTER TABLE` fails (DB corruption, disk full) | 74 (`SchemaMigrationFailed`) | Transaction rolls back; `schema_version` stays at 2; subsequent open will retry. |
| `DROP INDEX` fails | 74 | Same as above. |
| Transaction commit fails | 74 | Migration framework treats as failed; subsequent open will retry. |
| Migration framework finds gap (no migration from current to target) | 51 (`IndexIntegrityCheckFailure`) | Existing behaviour. |

## Testing

End-to-end test pattern (extending the Phase 3 / US5 `tests/schema_migration_e2e.rs`):

1. Bootstrap a v2 DB using existing helper `write_index_db_with_schema_version(path, 2)`.
2. Insert known rows via raw SQL (catalog `c1`, plugin `p1`, name `s1`).
3. Run `apply_pending(conn, 2, 3)`.
4. Assert:
   - `schema_version` row now reads 3.
   - Original rows' `kind` is `'skill'`, `searchable` is 1, `user_invocable` is 0, `when_to_use` is NULL.
   - The unique index `skills_unique` now spans `(catalog, plugin, kind, name)`.
   - Inserting a row with `kind='command'` and same `(catalog, plugin, name)` succeeds without violating the constraint.

Mid-tx failure injection (existing pattern): `MIGRATIONS_OVERRIDE` injects a synthetic migration that returns `Err` partway; assert that the original v2 state is preserved (no partial column adds).

## Doctor surface

`tome doctor` reports schema version in its existing surface; if version reads 2 in a v3-built binary, it surfaces a `Schema` subsystem `SuggestedFix` with `auto_fixable: true` (existing pattern from Phase 3 / US4). `--fix` runs `apply_pending(conn, 2, 3)`.

## What does NOT change

- The `skill_embeddings` virtual table (sqlite-vec) is unchanged.
- The `workspace_skills` junction table is unchanged structurally.
- The `workspace_catalogs` junction table is unchanged.
- The `workspaces` table is unchanged.
- The `meta` table's columns are unchanged.

# Schema Migration — Contract

How Tome migrates an index database's schema forward, refuses unknown-future schemas, and surfaces failure. Phase 3 lands the framework; **zero migrations are registered**. The synthetic-fixture test (`tests/schema_migration_e2e.rs`) exercises the framework against a crafted older-version SQLite file so Phase 4+ migrations land on tested rails.

## Algorithm

`index::migrations::apply_pending(conn: &mut Connection, current: u32, target: u32) -> Result<u32, TomeError>`:

```
if current == target:
    return Ok(current)                                # no-op
if current > target:
    return Err(SchemaVersionTooNew { on_disk: current, expected: target })
# current < target
for migration in MIGRATIONS where migration.from >= current && migration.to <= target:
    tx = conn.transaction()
    migration.apply(&tx)?                              # closure returns Err on failure
    write_meta(&tx, schema_version = migration.to)?
    tx.commit()?
return Ok(target)
```

## Registration

```rust
pub struct Migration {
    pub from: u32,
    pub to: u32,
    pub name: &'static str,                            // for logging
    pub apply: fn(&Transaction) -> Result<(), TomeError>,
}

/// Compile-time list of every migration. PHASE 3 SHIPS WITH ZERO MIGRATIONS.
pub const MIGRATIONS: &[Migration] = &[];

/// Test-only injection point.
#[cfg(test)]
thread_local! { pub static MIGRATIONS_OVERRIDE: RefCell<Option<&'static [Migration]>> = RefCell::new(None); }
```

The test injection point lets `tests/schema_migration_e2e.rs` register a synthetic v0→v1 migration without polluting production code.

## Atomicity

Each migration runs in its own `Transaction`. On failure, the transaction is dropped (rolling back); the schema version row is unchanged; the function returns `Err(SchemaMigrationFailed { from, to, source })`. The `current` schema on disk equals the highest `to` of any successfully-committed migration.

**Worked example** — DB is at v3, Tome expects v5, `MIGRATIONS = [v3→v4, v4→v5]`:

- Apply v3→v4. Commit. DB is now at v4.
- Apply v4→v5. Fail.
- Return `Err(SchemaMigrationFailed { from: 4, to: 5, … })`. DB stays at v4. Next Tome invocation sees v4, tries v4→v5 again.

This per-step atomicity is required because some migrations may be slow (schema rewrites against large indexes); rolling all the way back on a late failure throws away progress and forces re-running every prior migration.

## Concurrency

`apply_pending` acquires the workspace's advisory lockfile before opening the connection (or the global lockfile, depending on `Scope`). Two Tome processes attempting migration at the same time → one wins the lock, the other contends past `busy_timeout` and exits 50 (`IndexBusy`). The contending process retries naturally on next invocation.

## Refusal of newer-on-disk

`SchemaVersionTooNew { on_disk, expected }` is the dedicated refusal. Exit code 73. No code path attempts a backward migration under any circumstance — FR-182.

The error message names what the developer should do:

```
error[73]: schema version too new: on-disk schema is v3, this Tome supports up to v2
  hint: upgrade Tome to a version that supports schema v3 (or newer)
  index: /home/user/.local/share/tome/index.db
```

`tome doctor` surfaces this without `--fix` as `auto_fixable: false`. `--fix` skips the migration class entirely for this case.

## Failure isolation across scopes

Global DB and every workspace DB are migrated independently. A migration failure on workspace A's DB has no effect on workspace B or the global DB. The next Tome command against workspace A re-attempts the failed migration; commands against other scopes proceed normally.

## Logging

Each migration boundary emits a `tracing::info` line:

```
INFO tome::index::migrations migrating from=3 to=4 name="add_skill_tags_index" scope=workspace path=/abs/path
INFO tome::index::migrations migration committed from=3 to=4 elapsed_ms=42
```

On failure:

```
ERROR tome::index::migrations migration failed from=4 to=5 name="rebuild_vec_index" error="constraint violation: …" scope=workspace path=/abs/path
```

The scrubber runs against the error string before logging.

## Testing strategy (synthetic fixture)

`tests/schema_migration_e2e.rs` covers four cases against three fixture DBs:

1. **Forward migration succeeds.** `tests/fixtures/older-schema.db` records `meta.schema_version = 0`; the test injects `MIGRATIONS = &[Migration { from: 0, to: 1, apply: |tx| Ok(()) }]`; runs `apply_pending(conn, 0, 1)`; asserts the returned value is `1` and `meta.schema_version` on disk is `1`.
2. **Multi-step migration succeeds.** Same fixture; inject `&[m0→1, m1→2]`; assert the returned value is `2` and each step's commit was visible to subsequent steps.
3. **Mid-sequence failure leaves last-good intermediate.** Inject `&[m0→1, m1→2 (always Err)]`; assert `Err(SchemaMigrationFailed { from: 1, to: 2, … })` and `meta.schema_version == 1`.
4. **Newer-on-disk refused.** `tests/fixtures/newer-schema.db` records `meta.schema_version = 99`; test asserts `Err(SchemaVersionTooNew { on_disk: 99, expected: 1 })` and `meta.schema_version` still `99` on disk.

The injection point uses `MIGRATIONS_OVERRIDE` (a `thread_local!` over an `Option<&'static [Migration]>`); production `apply_pending` reads `MIGRATIONS` directly.

## Migration design constraints (Phase 4+ guidance)

Future migrations must:

- Be **deterministic**. The same migration applied to the same input DB yields the same output DB.
- Be **idempotent under failure-retry**. If the transaction rolls back, the next attempt sees the original state.
- Operate **only via SQL** (no out-of-band file mutations). The transaction must contain every change.
- Touch **only the index schema and its rows**. Migrations must not modify config files, model artefacts, catalog clones, or any other on-disk artefact.
- Be **paired with a downgrade plan documented in the spec**. Tome refuses backward migration in code; the spec documents how a downgrade-by-rebuild would work (delete the DB, re-enable every plugin).

## Exit codes

| Code | Meaning |
|---|---|
| 0 | Migration succeeded (or no migration needed). |
| 50 | Index busy — another process held the lock past timeout. |
| 73 | `SchemaVersionTooNew` — on-disk version exceeds what this Tome supports. |
| 74 | `SchemaMigrationFailed` — a registered migration's apply function returned an error. |
| 7 | `Io` — failed to open the DB or write to it. |

## Doctor interaction

`tome doctor` reports the schema state per opened DB:

- `current_version == expected` → "Schema version: v1" (no action needed).
- `current_version < expected` → "Schema needs forward migration from v0 to v1." Suggested fix: `tome doctor --fix` (auto_fixable: true).
- `current_version > expected` → "Schema version too new (v99 > v1). Upgrade Tome." Suggested fix: external (auto_fixable: false).

With `--fix`, doctor calls `apply_pending` for each DB that needs forward migration (the resolved scope's DB, by default; with multi-DB awareness in a future phase the doctor may walk every known DB — out of scope for Phase 3, which migrates only the resolved scope's DB).

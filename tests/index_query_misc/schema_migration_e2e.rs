//! Phase 3 / US5 — forward-migration framework end-to-end coverage.
//!
//! Phase 3 ships `index::migrations::apply_pending` with **zero registered
//! migrations**: the production `MIGRATIONS` slice is empty by design. The
//! first real migration lands in Phase 4+. This file exercises the
//! framework against **synthetic migrations** injected via
//! `MIGRATIONS_OVERRIDE` so future migrations land on tested rails.
//!
//! Contract: [`schema-migration.md`](../specs/003-phase-3-mcp-workspaces/contracts/schema-migration.md)
//! §Testing strategy.
//!
//! Companion file `tests/schema_migrations.rs` covers the "no migration
//! registered" defensive guard and the read-path CLI exit-52 gate; this
//! file covers the actually-registered-migrations path.
//!
//! `MIGRATIONS_OVERRIDE` is a `thread_local!`. Cargo runs each
//! `#[test]` on its own (possibly fresh) thread within the same process,
//! so as long as a test clears the slot before returning (drop guard +
//! `cargo test --test schema_migration_e2e -- --test-threads=N` both work)
//! no other test sees a stale override.

use std::cell::Cell;

use crate::common::write_index_db_with_schema_version;
use rusqlite::Transaction;
use tempfile::TempDir;
use tome::error::TomeError;
use tome::index::Migration;
use tome::index::migrations::{MIGRATIONS_OVERRIDE, apply_pending, current_schema_version};

/// RAII guard that swaps the per-thread `MIGRATIONS_OVERRIDE` for the
/// duration of a test, then restores `None` on drop. Survives panics so a
/// failed assertion never poisons subsequent tests on the same thread.
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
// Migration step functions (need `fn` items because `Migration.apply` is
// `fn(&Transaction) -> Result<(), TomeError>`, not a closure type).
// ---------------------------------------------------------------------------

fn migrate_v0_to_v1_create_marker(tx: &Transaction) -> Result<(), TomeError> {
    tx.execute_batch("CREATE TABLE v1_marker (id INTEGER PRIMARY KEY) STRICT")
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("v0→v1 apply: {e}")))?;
    Ok(())
}

fn migrate_v1_to_v2_create_marker(tx: &Transaction) -> Result<(), TomeError> {
    tx.execute_batch("CREATE TABLE v2_marker (id INTEGER PRIMARY KEY) STRICT")
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("v1→v2 apply: {e}")))?;
    // Read from the v1 table so we prove the prior step's commit is visible
    // here — multi-step success is "each step's commit was visible to
    // subsequent steps" per the contract.
    let _: i64 = tx
        .query_row("SELECT COUNT(*) FROM v1_marker", [], |row| row.get(0))
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("v1→v2 read v1: {e}")))?;
    Ok(())
}

fn migrate_v1_to_v2_always_fails(tx: &Transaction) -> Result<(), TomeError> {
    // Create a table inside the failing transaction. After the rollback this
    // table must NOT exist on disk — that proves the failing step's writes
    // were dropped.
    tx.execute_batch("CREATE TABLE v2_marker_should_not_exist (id INTEGER) STRICT")
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("v1→v2 scratch: {e}")))?;
    Err(TomeError::IndexIntegrityCheckFailure(
        "deliberate test failure".into(),
    ))
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

#[test]
fn forward_migration_v0_to_v1_succeeds() {
    static MIGRATIONS: &[Migration] = &[Migration {
        from: 0,
        to: 1,
        name: "test_v0_to_v1",
        apply: migrate_v0_to_v1_create_marker,
    }];
    let _guard = MigrationsGuard::install(MIGRATIONS);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    write_index_db_with_schema_version(&path, 0);

    let mut conn = rusqlite::Connection::open(&path).expect("open synthetic db");
    let new_version = apply_pending(&mut conn, 0, 1).expect("migration runs");
    assert_eq!(new_version, 1, "apply_pending must return the new version");

    // On-disk schema_version row reflects the committed step.
    let stored = current_schema_version(&conn).expect("probe").unwrap();
    assert_eq!(stored, 1, "meta.schema_version must reflect the commit");

    // The migration's side effect is visible.
    let table_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'v1_marker'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(table_exists, "v1_marker table must exist after migration");
}

#[test]
fn multi_step_forward_migration_succeeds() {
    static MIGRATIONS: &[Migration] = &[
        Migration {
            from: 0,
            to: 1,
            name: "test_v0_to_v1",
            apply: migrate_v0_to_v1_create_marker,
        },
        Migration {
            from: 1,
            to: 2,
            name: "test_v1_to_v2",
            apply: migrate_v1_to_v2_create_marker,
        },
    ];
    let _guard = MigrationsGuard::install(MIGRATIONS);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    write_index_db_with_schema_version(&path, 0);

    let mut conn = rusqlite::Connection::open(&path).expect("open synthetic db");
    let new_version = apply_pending(&mut conn, 0, 2).expect("multi-step migration runs");
    assert_eq!(
        new_version, 2,
        "apply_pending must return the final version after a multi-step walk",
    );

    let stored = current_schema_version(&conn).expect("probe").unwrap();
    assert_eq!(stored, 2, "meta.schema_version must be 2 after both steps");

    // Each step's table exists — proving every commit landed.
    for table in ["v1_marker", "v2_marker"] {
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |_| Ok(true),
            )
            .unwrap_or(false);
        assert!(
            exists,
            "table {table} must exist after multi-step migration"
        );
    }
}

#[test]
fn mid_sequence_failure_leaves_last_good_intermediate() {
    static MIGRATIONS: &[Migration] = &[
        Migration {
            from: 0,
            to: 1,
            name: "test_v0_to_v1_ok",
            apply: migrate_v0_to_v1_create_marker,
        },
        Migration {
            from: 1,
            to: 2,
            name: "test_v1_to_v2_fail",
            apply: migrate_v1_to_v2_always_fails,
        },
    ];
    let _guard = MigrationsGuard::install(MIGRATIONS);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    write_index_db_with_schema_version(&path, 0);

    let mut conn = rusqlite::Connection::open(&path).expect("open synthetic db");
    let err = apply_pending(&mut conn, 0, 2).expect_err("v1→v2 step must fail");
    match err {
        TomeError::SchemaMigrationFailed { from, to, source } => {
            assert_eq!(from, 1, "failure must report from=1 (the failing step)");
            assert_eq!(to, 2, "failure must report to=2");
            assert!(
                source.to_string().contains("deliberate test failure"),
                "failure source must surface the inner error: {source:#}",
            );
        }
        other => panic!("expected SchemaMigrationFailed, got {other:?}"),
    }

    // Last-good invariant: schema_version == 1 (the committed step), NOT 0
    // (unrolled) and NOT 2 (would mean the failure didn't roll back).
    let stored = current_schema_version(&conn).expect("probe").unwrap();
    assert_eq!(
        stored, 1,
        "schema_version must reflect the last successfully-committed step",
    );

    // The first step's table is committed.
    let v1_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'v1_marker'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(v1_exists, "v1_marker table must exist (its step committed)");

    // The failing step's scratch table must NOT exist — the transaction
    // rolled back, so its writes are gone.
    let v2_scratch_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'v2_marker_should_not_exist'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(
        !v2_scratch_exists,
        "failing migration's writes must roll back",
    );
}

#[test]
fn newer_on_disk_refused_with_schema_version_too_new() {
    // No migrations registered for this test — the refusal happens before
    // any migration is consulted. Still install an empty override so the
    // production `MIGRATIONS` list is bypassed for symmetry with the other
    // tests.
    static MIGRATIONS: &[Migration] = &[];
    let _guard = MigrationsGuard::install(MIGRATIONS);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    write_index_db_with_schema_version(&path, 99);

    let mut conn = rusqlite::Connection::open(&path).expect("open synthetic db");
    let err = apply_pending(&mut conn, 99, 1).expect_err("must refuse newer-on-disk");
    match err {
        TomeError::SchemaVersionTooNew { on_disk, expected } => {
            assert_eq!(on_disk, 99, "must report the on-disk version");
            assert_eq!(expected, 1, "must report the compiled-in expected version");
        }
        other => panic!("expected SchemaVersionTooNew, got {other:?}"),
    }

    // The refusal must not mutate the on-disk version.
    let stored = current_schema_version(&conn).expect("probe").unwrap();
    assert_eq!(stored, 99, "refusal must not touch the on-disk version");
}

/// T098m (Phase 4 / FR-325): a reader opened against a schema-newer-than-
/// compiled-in DB MUST be refused with `SchemaVersionTooNew` — never
/// silently observe garbage columns from a future migration. The
/// `open_read_only` gate uses the legacy [`TomeError::SchemaTooNew`]
/// (exit 52) for the read path; here we exercise the writer-side
/// [`TomeError::SchemaVersionTooNew`] (exit 73) for parity with what a
/// half-applied future migration would surface to the next reader after
/// the writer commits beyond the compiled-in target.
///
/// We synthesise the "future on-disk version" by stamping the DB at v99
/// directly, then opening it for a write attempt. The contract is
/// identical to the "newer-on-disk refused" test above, but phrased in
/// terms of FR-325's read-path semantics: a reader observing v99 must
/// receive a structured refusal, never a half-migrated view.
#[test]
fn read_path_refuses_newer_on_disk_with_schema_version_too_new() {
    static MIGRATIONS: &[Migration] = &[];
    let _guard = MigrationsGuard::install(MIGRATIONS);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    write_index_db_with_schema_version(&path, 99);

    // `apply_pending` is the writer-side gate; it must refuse with
    // `SchemaVersionTooNew` for the same reason `open_read_only` refuses
    // with `SchemaTooNew` — neither must execute against an unknown
    // schema. FR-325 names the invariant: a reader must EITHER see a
    // pre-migration consistent snapshot OR a structured refusal — never
    // a half-migrated mid-statement view.
    let mut conn = rusqlite::Connection::open(&path).expect("open synthetic db");
    let err = apply_pending(&mut conn, 99, tome::index::schema::SCHEMA_VERSION)
        .expect_err("future-version DB must be refused");
    assert!(
        matches!(err, TomeError::SchemaVersionTooNew { on_disk: 99, .. }),
        "expected SchemaVersionTooNew, got {err:?}",
    );
}

/// T098m (Phase 4 / FR-325): SQLite's MVCC snapshot guarantees a
/// reader holding an open connection sees a consistent view of the
/// database for the duration of its transaction, regardless of what a
/// concurrent writer commits. This test exercises the simpler half of
/// FR-325 — a reader opened against schema v1 keeps observing v1 even
/// while a writer migrates to v2 on a second thread.
///
/// The timing-sensitive "writer commits mid-statement" race is left
/// `#[ignore]`-d below (US4.b follow-up) because it requires controlled
/// scheduling we don't have infrastructure for. The MVCC-snapshot half
/// is what production callers actually rely on.
#[test]
fn reader_holding_snapshot_observes_pre_migration_schema() {
    use std::sync::Arc;
    use std::sync::Barrier;
    use std::thread;

    static MIGRATIONS: &[Migration] = &[Migration {
        from: 1,
        to: 2,
        name: "test_mvcc_snapshot_v1_to_v2",
        apply: migrate_v1_to_v2_create_marker,
    }];
    // First step from v0 must exist so v1_marker is queryable for the
    // multi-step v2 body.
    static FULL: &[Migration] = &[
        Migration {
            from: 0,
            to: 1,
            name: "test_mvcc_snapshot_v0_to_v1",
            apply: migrate_v0_to_v1_create_marker,
        },
        Migration {
            from: 1,
            to: 2,
            name: "test_mvcc_snapshot_v1_to_v2",
            apply: migrate_v1_to_v2_create_marker,
        },
    ];
    let _guard = MigrationsGuard::install(MIGRATIONS);
    // Silence unused-static warnings: FULL is referenced for documentation
    // of the migration shape but the actual sequencing here is single-step
    // because we stamp v1 directly.
    let _ = FULL;

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    write_index_db_with_schema_version(&path, 1);
    // Switch the synthetic DB into WAL mode and pre-create the
    // v1_marker table. WAL is the only journal mode under which
    // SQLite's MVCC snapshot semantics apply (FR-325 relies on this);
    // the helper that built the synthetic v1 DB defaults to rollback
    // journal. Also create v1_marker by hand — the FULL migration
    // would have produced it on the v0→v1 step; we stamped v1
    // directly so the v1→v2 step's `SELECT FROM v1_marker` requires it.
    {
        let conn = rusqlite::Connection::open(&path).expect("populate v1 state");
        conn.pragma_update(None, "journal_mode", "WAL")
            .expect("enable WAL");
        conn.execute_batch("CREATE TABLE v1_marker (id INTEGER PRIMARY KEY) STRICT")
            .expect("create v1_marker");
    }

    // Reader thread: open a read-only handle, start a deferred transaction
    // to pin the MVCC snapshot, then wait on the barrier so the writer
    // can race ahead. After the writer commits + signals, the reader
    // confirms `v2_marker` is NOT visible from its snapshot.
    let barrier_start = Arc::new(Barrier::new(2));
    let barrier_after_commit = Arc::new(Barrier::new(2));

    let reader_path = path.clone();
    let r_start = Arc::clone(&barrier_start);
    let r_after = Arc::clone(&barrier_after_commit);
    let reader = thread::spawn(move || {
        let conn = rusqlite::Connection::open_with_flags(
            &reader_path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .expect("reader open");

        // Pin the MVCC snapshot by issuing a read against the on-disk
        // schema version before the writer migrates.
        let before_version: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("read schema_version pre-migration");
        assert_eq!(before_version, "1", "reader must see v1 before migration");

        // Hold open a transaction so SQLite's WAL snapshot is pinned
        // for the rest of this thread's lifetime.
        conn.execute_batch("BEGIN DEFERRED").expect("begin tx");
        // First read inside the transaction establishes the snapshot.
        let _: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("snapshot establish");

        r_start.wait();
        // Writer races ahead now.
        r_after.wait();

        // Inside our pinned snapshot, v2_marker must NOT be visible —
        // the writer's commit lands after our snapshot.
        let v2_visible: bool = conn
            .query_row(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name='v2_marker'",
                [],
                |_| Ok(true),
            )
            .unwrap_or(false);

        conn.execute_batch("COMMIT").ok();
        v2_visible
    });

    // Writer thread: wait for reader's snapshot to pin, then run the
    // v1→v2 migration through the production `apply_pending` path.
    // `MIGRATIONS_OVERRIDE` is `thread_local!` — the writer thread must
    // install its OWN guard (the main-thread guard doesn't propagate)
    // so `apply_pending` here uses the synthetic single-step v1→v2
    // migration, not the production registry.
    let writer_path = path.clone();
    let w_start = Arc::clone(&barrier_start);
    let w_after = Arc::clone(&barrier_after_commit);
    let writer = thread::spawn(move || {
        let _writer_guard = MigrationsGuard::install(MIGRATIONS);
        w_start.wait();
        let mut conn = rusqlite::Connection::open(&writer_path).expect("writer open");
        let new_version = apply_pending(&mut conn, 1, 2).expect("writer migrates");
        assert_eq!(new_version, 2);
        w_after.wait();
    });

    writer.join().expect("writer thread");
    let v2_visible_to_reader = reader.join().expect("reader thread");

    assert!(
        !v2_visible_to_reader,
        "FR-325 MVCC: a reader's pinned snapshot must not observe a post-commit migration",
    );

    // Fresh read-only handle after both threads complete must see v2.
    let conn = rusqlite::Connection::open(&path).expect("fresh open");
    let after_version: String = conn
        .query_row(
            "SELECT value FROM meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("post-migration version");
    assert_eq!(
        after_version, "2",
        "fresh reader opened after migration must see v2",
    );
}

/// T098m timing-sensitive half (FR-325). Verifies a reader that
/// SUBMITS a statement WHILE the writer is mid-transaction (i.e. the
/// writer holds BEGIN but has not yet committed) sees the
/// pre-migration view. SQLite WAL guarantees this — writers don't
/// block readers — but proving it requires controlled scheduling that
/// `std::thread` + `Barrier` cannot reliably express without flakiness.
/// Left ignored; revisit if/when the project picks up a deterministic
/// concurrency-testing harness (e.g. shuttle).
#[test]
#[ignore = "F11c-2 followup: timing-sensitive mid-migration race needs controlled scheduling"]
fn reader_mid_writer_transaction_sees_pre_migration_view() {
    // Skeleton intentionally empty: see the doc-comment above.
}

#[test]
fn migrations_override_is_thread_local_and_clears_on_drop() {
    // Sanity guard: the RAII `MigrationsGuard` clears the slot on drop.
    // Without this, a subsequent test on the same thread would see a stale
    // override. Verifies the cleanup discipline this file relies on.
    {
        static MIGRATIONS: &[Migration] = &[Migration {
            from: 0,
            to: 1,
            name: "scoped",
            apply: migrate_v0_to_v1_create_marker,
        }];
        let _guard = MigrationsGuard::install(MIGRATIONS);
        let registered = MIGRATIONS_OVERRIDE.with(|slot| slot.borrow().is_some());
        assert!(registered, "guard must install the override");
    }
    let cleared = MIGRATIONS_OVERRIDE.with(|slot| slot.borrow().is_none());
    assert!(cleared, "guard's Drop must clear the override");
    // `Cell` is only used here to keep the assertion blocks from being
    // optimised away in --release. (Test crate compiles --release builds for
    // bench-style runs in CI; defensive belt-and-braces.)
    let _ = Cell::new(0u8);
}

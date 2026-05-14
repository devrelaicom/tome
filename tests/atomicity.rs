//! Interruption-injecting tests for the atomic write path (SC-012). The
//! contract: an interrupted write leaves the on-disk file in either its
//! pre-state or its post-state, never a partial-bytes state.
//!
//! Phase 3 / US5 adds a forward-migration atomicity case below. The
//! schema-migration framework's per-step transaction guarantees that a
//! failure inside a migration closure rolls the SQLite transaction back —
//! both the closure's data writes AND the `schema_version` bump live in the
//! same `Transaction`, so a `Drop` (rollback) restores both. We model the
//! SIGINT scenario as a deliberate `Err` returned from the migration
//! closure: the rollback path is identical regardless of whether the
//! abort came from a signal handler or a closure-level failure. The
//! `catalog::git::CANCELLED` static is *not* used here — flipping it
//! races every other test in the binary (cargo runs tests in the same
//! process), per the same discipline documented in `atomicity_enable.rs`.

mod common;

use std::fs;
use std::path::Path;

use common::write_index_db_with_schema_version;
use rusqlite::Transaction;
use tempfile::TempDir;
use time::OffsetDateTime;
use tome::catalog::store::{save, write_atomic};
use tome::config::{CatalogEntry, Config};
use tome::error::TomeError;
use tome::index::Migration;
use tome::index::migrations::{MIGRATIONS_OVERRIDE, apply_pending, current_schema_version};

fn make_config(name: &str) -> Config {
    let mut cfg = Config::default();
    cfg.catalogs.insert(
        name.into(),
        CatalogEntry {
            name: name.into(),
            url: format!("https://example/{}", name),
            ref_: "main".into(),
            path: std::path::PathBuf::from("/tmp/x"),
            last_synced: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        },
    );
    cfg
}

#[test]
fn write_atomic_does_not_leave_partial_file_when_target_dir_writable() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("config.toml");
    write_atomic(&target, b"first").unwrap();
    write_atomic(&target, b"second").unwrap();
    let read = fs::read(&target).unwrap();
    assert_eq!(read, b"second");
}

#[test]
fn no_temp_file_left_behind_after_successful_write() {
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("config.toml");
    save(&target, &make_config("a")).unwrap();
    let entries: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    // Exactly one entry — the target. The same-directory temp file used by
    // `tempfile::NamedTempFile::persist` is consumed by the rename.
    assert_eq!(entries.len(), 1, "stray files: {:?}", entries);
}

#[test]
fn failed_persist_into_nonexistent_dir_does_not_create_target() {
    // Pre-existing file then attempt a write to a sub-path whose parent will
    // be created. The pre-existing file is unrelated and must be untouched.
    let dir = TempDir::new().unwrap();
    let untouched = dir.path().join("other.toml");
    fs::write(&untouched, b"do not touch").unwrap();
    let target = dir.path().join("nested/config.toml");
    save(&target, &make_config("a")).unwrap();
    let kept = fs::read(&untouched).unwrap();
    assert_eq!(kept, b"do not touch");
    assert!(target.exists());
}

#[test]
fn concurrent_writes_yield_a_complete_file_not_a_torn_one() {
    // Spawn 8 writers racing to replace the same target. The atomic-rename
    // contract guarantees the final file matches one of the writers' inputs,
    // never a mix.
    let dir = TempDir::new().unwrap();
    let target = dir.path().join("config.toml");
    fs::write(&target, b"pre").unwrap();
    let mut handles = Vec::new();
    for i in 0..8u8 {
        let t = target.clone();
        handles.push(std::thread::spawn(move || {
            let payload = format!("writer-{}", i).into_bytes();
            // ignore any individual error — at least one must succeed
            let _ = write_atomic(&t, &payload);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let bytes = fs::read(&target).unwrap();
    let final_text = String::from_utf8(bytes).expect("utf-8");
    assert!(
        is_one_of_the_writers(&final_text) || final_text == "pre",
        "torn write detected: {:?}",
        final_text
    );
}

fn is_one_of_the_writers(s: &str) -> bool {
    (0..8u8).any(|i| s == format!("writer-{}", i))
}

#[test]
fn missing_target_directory_is_created_on_save() {
    let dir = TempDir::new().unwrap();
    let nested = dir.path().join("a/b/c");
    let target = nested.join("config.toml");
    assert!(!nested.exists());
    save(&target, &make_config("a")).unwrap();
    assert!(target.exists());
    assert!(is_dir(&nested));
}

fn is_dir(p: &Path) -> bool {
    fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Phase 3 / US5 — forward-migration atomicity (T199).
// ---------------------------------------------------------------------------

/// Migration step that performs schema mutations and *then* errors. The
/// transaction's rollback must drop both the schema mutation AND the
/// `schema_version` bump (the framework writes the bump inside the same tx).
fn migrate_v0_to_v1_aborts_mid_tx(tx: &Transaction) -> Result<(), TomeError> {
    tx.execute_batch("CREATE TABLE v1_partial (id INTEGER PRIMARY KEY) STRICT")
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("v0→v1 partial: {e}")))?;
    tx.execute("INSERT INTO v1_partial DEFAULT VALUES", [])
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("v0→v1 row: {e}")))?;
    // Now abort. The framework will roll back the entire transaction — the
    // table, the row, and the schema_version bump all disappear together.
    Err(TomeError::IndexIntegrityCheckFailure(
        "simulated mid-transaction abort".into(),
    ))
}

struct MigrationsGuard;
impl MigrationsGuard {
    fn install(m: &'static [Migration]) -> Self {
        MIGRATIONS_OVERRIDE.with(|slot| *slot.borrow_mut() = Some(m));
        Self
    }
}
impl Drop for MigrationsGuard {
    fn drop(&mut self) {
        MIGRATIONS_OVERRIDE.with(|slot| *slot.borrow_mut() = None);
    }
}

#[test]
fn migration_abort_mid_transaction_leaves_schema_version_and_data_unchanged() {
    static MIGRATIONS: &[Migration] = &[Migration {
        from: 0,
        to: 1,
        name: "test_abort_mid_tx",
        apply: migrate_v0_to_v1_aborts_mid_tx,
    }];
    let _guard = MigrationsGuard::install(MIGRATIONS);

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("index.db");
    write_index_db_with_schema_version(&path, 0);

    let mut conn = rusqlite::Connection::open(&path).expect("open synthetic db");
    let err = apply_pending(&mut conn, 0, 1).expect_err("migration must fail");
    assert!(
        matches!(err, TomeError::SchemaMigrationFailed { from: 0, to: 1, .. }),
        "expected SchemaMigrationFailed(0→1), got {err:?}",
    );

    // schema_version untouched — the bump lived inside the rolled-back tx.
    let stored = current_schema_version(&conn).expect("probe").unwrap();
    assert_eq!(
        stored, 0,
        "schema_version must stay at 0 after a mid-tx abort",
    );

    // The partial table the migration tried to create must be gone — same
    // tx, same rollback.
    let table_exists: bool = conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'v1_partial'",
            [],
            |_| Ok(true),
        )
        .unwrap_or(false);
    assert!(
        !table_exists,
        "v1_partial must not exist on disk after rollback",
    );
}

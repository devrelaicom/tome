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

use std::fs;
use std::path::Path;

use crate::common::write_index_db_with_schema_version;
use rusqlite::Transaction;
use tempfile::TempDir;
use tome::catalog::store::write_atomic;
use tome::error::TomeError;
use tome::index::Migration;
use tome::index::migrations::{MIGRATIONS_OVERRIDE, apply_pending, current_schema_version};

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
    write_atomic(&target, b"placeholder").unwrap();
    let entries: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .collect();
    // Exactly one entry — the target. The same-directory temp file used by
    // `tempfile::NamedTempFile::persist` is consumed by the rename.
    assert_eq!(entries.len(), 1, "stray files: {:?}", entries);
}

#[test]
fn write_to_nested_path_creates_target_and_leaves_sibling_untouched() {
    // A write to a sub-path whose parent does not yet exist: the target is
    // created (atomic write creates missing parent dirs), and a pre-existing
    // sibling file in the root dir must be byte-for-byte untouched.
    let dir = TempDir::new().unwrap();
    let untouched = dir.path().join("other.toml");
    fs::write(&untouched, b"do not touch").unwrap();
    let target = dir.path().join("nested/config.toml");
    write_atomic(&target, b"placeholder").unwrap();
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
    write_atomic(&target, b"placeholder").unwrap();
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

// ---------------------------------------------------------------------------
// Phase 6 / US3 — guardrails atomicity (T3-2).
// ---------------------------------------------------------------------------

/// A mid-write failure on an in-file guardrails target that already holds a
/// region surfaces exit 46 AND leaves the file byte-for-byte unchanged (the
/// old region intact, no partial region between markers).
///
/// Injection: a target that already holds a region, with its PARENT directory
/// made read-only so the atomic write (sibling tempfile creation in the parent)
/// fails after the desired body has been computed. The symlink-refusal path is
/// not involved — this exercises the actual atomic-write path.
#[test]
#[cfg(unix)]
fn guardrails_in_file_write_failure_leaves_target_byte_unchanged() {
    use std::collections::BTreeMap;
    use std::os::unix::fs::PermissionsExt;
    use tome::harness::guardrails;

    let dir = TempDir::new().unwrap();
    let target = dir.path().join("CLAUDE.md");

    // Seed an existing region via a successful first reconcile.
    let mut desired = BTreeMap::new();
    desired.insert("cat:plug".to_string(), "original guardrails\n".to_string());
    guardrails::reconcile_in_file_region(&target, &desired).expect("seed region");
    let before = fs::read(&target).expect("read seeded target");

    // Make the parent directory read-only so the sibling tempfile cannot be
    // created → the atomic write fails mid-reconcile.
    let parent = target.parent().unwrap();
    let original_mode = fs::metadata(parent).unwrap().permissions().mode();
    fs::set_permissions(parent, fs::Permissions::from_mode(0o500)).expect("chmod parent ro");

    // Reconcile with a CHANGED body so a write is actually attempted.
    let mut changed = BTreeMap::new();
    changed.insert("cat:plug".to_string(), "updated guardrails\n".to_string());
    let result = guardrails::reconcile_in_file_region(&target, &changed);

    // Restore permissions before any assertion can early-return (so the
    // TempDir can clean up).
    fs::set_permissions(parent, fs::Permissions::from_mode(original_mode)).expect("restore perms");

    let err = result.expect_err("a write into a read-only parent must fail");
    assert_eq!(
        err.exit_code(),
        46,
        "a guardrails write failure → exit 46; got {err:?}"
    );

    // The file is byte-for-byte unchanged — old region intact, no partial
    // region between markers.
    let after = fs::read(&target).expect("read target after failed write");
    assert_eq!(
        before, after,
        "target must be byte-for-byte unchanged after a failed write"
    );
    let after_text = String::from_utf8(after).expect("utf-8");
    assert!(
        after_text.contains("original guardrails"),
        "the old region body must survive:\n{after_text}"
    );
    assert!(
        !after_text.contains("updated guardrails"),
        "no partial new region must be written:\n{after_text}"
    );
    assert_eq!(
        after_text
            .matches("<!-- START GUARDRAILS: cat:plug -->")
            .count(),
        1,
        "exactly one START marker — no torn duplicate:\n{after_text}"
    );
}

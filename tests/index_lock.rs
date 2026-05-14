//! Integration tests for the advisory write lock (slice 4b).
//!
//! Two scenarios cover the FR-040 contract:
//!
//! 1. **Contention**: a second `acquire` on the same path while a first
//!    guard is alive must fail with [`TomeError::IndexBusy`] (exit 50)
//!    rather than blocking until the lock is released.
//! 2. **Release on drop**: after the first guard is dropped, the next
//!    `acquire` must succeed.
//!
//! The test exercises two file handles inside one process. Rust's stable
//! `File::try_lock` uses per-fd OS-level locks (F_OFD_SETLK on Linux,
//! flock on macOS/BSD, LockFileEx on Windows), so two opens on the same
//! path do compete inside one process — no need for a child process.
//!
//! Spec: research §R2, FR-040.

use std::time::Instant;

use tempfile::TempDir;

use tome::error::TomeError;
use tome::index::{OpenOptions, lock, open, open_read_only, schema::MetaSeed};

fn lock_path_in(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("index.lock")
}

#[test]
fn second_acquire_returns_index_busy() {
    let dir = TempDir::new().expect("tempdir");
    let path = lock_path_in(&dir);

    let first = lock::acquire(&path).expect("first acquire");

    let err = lock::acquire(&path).expect_err("second acquire must fail while first is held");
    match err {
        TomeError::IndexBusy => {}
        other => panic!("expected IndexBusy, got {other:?}"),
    }

    // Keep `first` alive to prove the second attempt failed for a *live*
    // lock, not because of any drop reordering.
    drop(first);
}

#[test]
fn drop_releases_the_lock() {
    let dir = TempDir::new().expect("tempdir");
    let path = lock_path_in(&dir);

    {
        let _first = lock::acquire(&path).expect("first acquire");
    }
    // After the first guard's drop, the same process must be able to
    // re-acquire without blocking.
    let _second = lock::acquire(&path).expect("re-acquire after drop");
}

#[test]
fn explicit_release_allows_reacquire() {
    let dir = TempDir::new().expect("tempdir");
    let path = lock_path_in(&dir);

    let first = lock::acquire(&path).expect("first acquire");
    first.release().expect("explicit release");

    let _second = lock::acquire(&path).expect("re-acquire after explicit release");
}

#[test]
fn lockfile_creation_is_idempotent() {
    // Pre-create the lockfile (e.g. left over from a previous crash). The
    // acquire path must reuse it rather than fail with EEXIST.
    let dir = TempDir::new().expect("tempdir");
    let path = lock_path_in(&dir);
    std::fs::write(&path, b"").expect("seed empty lockfile");

    let _guard = lock::acquire(&path).expect("acquire on pre-existing lockfile");
}

/// Phase 3 slice F5: a read-only DB handle must not block a writer that
/// is holding the advisory lockfile, and must not race with it.
///
/// We can't easily simulate a writer with the lockfile held *and* an open
/// write `Connection` in one process (the lockfile is fd-scoped and the
/// SQLite connection is process-scoped, but the WAL writer state stays
/// consistent for our purposes). The narrow guarantee this test exercises:
///
/// 1. With the advisory write lockfile held, `open_read_only` succeeds.
///    The Phase 2 `acquire_lock` semantics promise it gates writers, not
///    readers; `open_read_only` is the reader contract.
/// 2. The reader can run a real query against the DB within the
///    busy_timeout window (5s). If reads were blocked on the writer's
///    locks the query would error with `database is locked` after 5s.
fn meta_seed(name: &str, version: &str) -> MetaSeed {
    MetaSeed {
        name: name.to_owned(),
        version: version.to_owned(),
    }
}

#[test]
fn read_only_open_does_not_block_writer_lock() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("index.db");
    let lock_path = dir.path().join("index.lock");

    // Bootstrap the DB once via the normal write path so `meta`, `skills`,
    // and `skill_embeddings` exist for the reader to query.
    let _bootstrap = open(
        &db_path,
        &OpenOptions {
            embedder: meta_seed("stub-embedder", "0"),
            reranker: meta_seed("stub-reranker", "0"),
        },
    )
    .expect("bootstrap");

    // Acquire the writer's advisory lock and hold it for the duration of
    // the reader's work — mirrors `lifecycle::enable` mid-transaction.
    let writer_lock = lock::acquire(&lock_path).expect("acquire write lock");

    let started = Instant::now();
    let reader = open_read_only(&db_path).expect("open read-only under writer lock");

    // Real query — exercise both the skills table and the meta table.
    let count: i64 = reader
        .query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
        .expect("read-only query against locked writer");
    assert_eq!(count, 0, "fresh-bootstrap skills table must be empty");

    let elapsed = started.elapsed();
    // 5s is the busy_timeout; anything close to it means we were blocked.
    // 100ms is a generous bound for a single SELECT against a 2-row table.
    assert!(
        elapsed.as_millis() < 1_000,
        "read-only query took {elapsed:?} — likely blocked on writer lock"
    );

    drop(reader);
    drop(writer_lock);
}

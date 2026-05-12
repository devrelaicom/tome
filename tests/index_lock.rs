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

use tempfile::TempDir;

use tome::error::TomeError;
use tome::index::lock;

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

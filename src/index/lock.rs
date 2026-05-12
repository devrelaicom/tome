//! Advisory write lock for the index database.
//!
//! Mutating commands (`plugin enable`, `plugin disable`, `reindex`, schema
//! migrations) acquire this lockfile before opening their SQLite transaction.
//! Read-only commands (`query`, `plugin list`, `status`) deliberately do not
//! touch it. See research §R2 for the rationale and the user-visible
//! behaviour table.
//!
//! Implementation: `std::fs::File::try_lock` — per-fd OS-level advisory lock
//! (flock on macOS/BSD, F_OFD_SETLK on Linux, LockFileEx on Windows) stable
//! since Rust 1.89. Contention is mapped to [`TomeError::IndexBusy`]
//! (exit 50) so the user sees a dedicated error within milliseconds rather
//! than waiting out the 5 s SQLite `busy_timeout`.
//!
//! Crash safety: the OS releases the lock when the holding process dies,
//! so there are no orphaned locks to clean up. Dropping the [`LockGuard`]
//! returned by [`acquire`] releases the lock via `File::unlock` at the
//! `drop` site.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use crate::error::TomeError;

/// Holder of the advisory write lock. The lock is released either explicitly
/// via [`LockGuard::release`] or implicitly when the guard is dropped.
#[derive(Debug)]
pub struct LockGuard {
    file: Option<File>,
    path: PathBuf,
}

impl LockGuard {
    /// Release the lock and consume the guard. Equivalent to letting it drop
    /// at the end of scope, but lets callers surface unlock errors instead
    /// of swallowing them in the destructor.
    pub fn release(mut self) -> Result<(), TomeError> {
        if let Some(file) = self.file.take() {
            file.unlock().map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "release index lock at {}: {e}",
                    self.path.display()
                ))
            })?;
        }
        Ok(())
    }
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            // Best effort — the OS will also release on process exit.
            let _ = file.unlock();
        }
    }
}

/// Try to acquire the advisory write lock at `lock_path`. Returns the guard
/// on success; on contention returns [`TomeError::IndexBusy`].
///
/// The parent directory must exist (the caller is expected to have already
/// opened the index DB, which creates the directory).
pub fn acquire(lock_path: &Path) -> Result<LockGuard, TomeError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(TomeError::Io)?;

    match file.try_lock() {
        Ok(()) => Ok(LockGuard {
            file: Some(file),
            path: lock_path.to_path_buf(),
        }),
        Err(TryLockError::WouldBlock) => Err(TomeError::IndexBusy),
        Err(TryLockError::Error(e)) => Err(TomeError::Io(e)),
    }
}

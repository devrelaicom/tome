//! The telemetry flush lock (`telemetry/flush.lock`).
//!
//! This is a SEPARATE advisory lock from the index lock (`index.lock`, NFR-009):
//! telemetry must never contend with — or be blocked by — index work, and vice
//! versa. The two locks live on different files and never nest, so there is no
//! cross-subsystem deadlock to reason about.
//!
//! It serialises the three operations that read+rewrite telemetry state: the
//! delivery flush (US3), and the explicit `reset`/`purge` commands in
//! [`super::identity`]. Only one process at a time may drain the queue or
//! rewrite the install id.
//!
//! Implementation mirrors `src/index/lock.rs`: a per-fd OS advisory lock via
//! `std::fs::File::try_lock` with a `Drop` guard that releases on scope exit.
//! The OS releases the lock on process death, so there are no orphaned locks to
//! clean up. The foreground `reset`/`purge` commands use a BOUNDED retry over
//! `try_acquire` ([`acquire_bounded`], FR-021a) so a hung flusher can never
//! block them indefinitely; the background flusher (US3) uses the non-blocking
//! [`try_acquire`] directly.

use std::fs::{File, OpenOptions, TryLockError};
use std::time::{Duration, Instant};

use crate::error::TomeError;
use crate::paths::Paths;

/// Poll interval between bounded-acquire retries. Small enough that the wait is
/// responsive, large enough that a brief flush is waited out without a busy spin.
const ACQUIRE_RETRY_INTERVAL: Duration = Duration::from_millis(25);

/// RAII holder of the telemetry flush lock. The lock is released when the guard
/// is dropped (or the process exits).
#[derive(Debug)]
pub struct FlushLock {
    // `Option` so `Drop` can take the file and unlock it exactly once.
    file: Option<File>,
}

impl Drop for FlushLock {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            // Best effort — the OS also releases on process exit. A failed
            // unlock here is not actionable (the fd is about to close), so we
            // deliberately swallow it.
            let _ = file.unlock();
        }
    }
}

/// Open (creating if absent) the `telemetry/flush.lock` file with a `0600`
/// mode, creating the `telemetry/` parent dir first.
///
/// The lock file is opened, never written to — it is a pure lock token, so its
/// contents are irrelevant; we only need a stable fd to lock.
fn open_lock_file(paths: &Paths) -> Result<File, TomeError> {
    // Land `telemetry/` if it does not exist yet (mode 0700, matching the id
    // dir). Idempotent; a pre-existing dir is fine.
    let dir = paths.telemetry_dir();
    std::fs::create_dir_all(&dir).map_err(TomeError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // Best-effort tighten to 0700; ignore the error on a platform/FS that
        // rejects it (the lock semantics do not depend on the mode).
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }

    let path = paths.telemetry_flush_lock();
    let mut opts = OpenOptions::new();
    opts.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // 0600: only the owner may take the lock token. Matches the id/config
        // file modes so the whole telemetry tree is owner-only.
        opts.mode(0o600);
    }
    opts.open(&path).map_err(TomeError::Io)
}

/// Acquire the flush lock with a BOUNDED wait (FR-021a).
///
/// Used by `reset`/`purge`: explicit, foreground user commands whose lock acquire
/// MUST be bounded — never an indefinite wait — so a hung flusher cannot block
/// the user's command forever. We poll [`try_acquire`] every
/// [`ACQUIRE_RETRY_INTERVAL`] until the lock is taken or `max_wait` elapses; on
/// timeout we return a `WouldBlock` [`TomeError::Io`] ("telemetry flush in
/// progress; retry shortly").
///
/// `Instant` is the correct clock here: this is a single-process, in-foreground
/// wall-clock budget (not a cross-process timestamp), so monotonic elapsed time
/// is exactly what we want — and it is immune to wall-clock adjustments.
pub fn acquire_bounded(paths: &Paths, max_wait: Duration) -> Result<FlushLock, TomeError> {
    let deadline = Instant::now() + max_wait;
    loop {
        if let Some(guard) = try_acquire(paths)? {
            return Ok(guard);
        }
        // Contended. Give up once the budget is spent; otherwise back off briefly
        // and retry. Clamp the sleep to whatever remains so we never overshoot.
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(TomeError::Io(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "telemetry flush in progress; retry shortly",
            )));
        }
        std::thread::sleep(ACQUIRE_RETRY_INTERVAL.min(remaining));
    }
}

/// Try to acquire the flush lock WITHOUT blocking.
///
/// Returns `Ok(None)` on contention (`WouldBlock`) — the US3 background flusher
/// uses this to silently no-op when another flush already holds the lock (only
/// one delivery at a time, never a queued wait). `Ok(Some(_))` on success, and
/// `Err(Io)` on a real lock/open error.
pub fn try_acquire(paths: &Paths) -> Result<Option<FlushLock>, TomeError> {
    let file = open_lock_file(paths)?;
    match file.try_lock() {
        Ok(()) => Ok(Some(FlushLock { file: Some(file) })),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(e)) => Err(TomeError::Io(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn paths_in(dir: &TempDir) -> Paths {
        Paths::from_root(dir.path().to_path_buf())
    }

    #[test]
    fn acquire_bounded_succeeds_on_fresh_lock() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let guard = acquire_bounded(&paths, Duration::from_secs(1)).unwrap();
        // The lock file was created.
        assert!(paths.telemetry_flush_lock().exists());
        drop(guard);
    }

    #[test]
    fn try_acquire_returns_none_while_held() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);

        // Hold the lock on one fd...
        let held = acquire_bounded(&paths, Duration::from_secs(1)).unwrap();
        // ...a second fd in the SAME process must see contention. (OFD/flock
        // locks are per-open-file-description, so two opens of the same path
        // contend even within one process.)
        assert!(try_acquire(&paths).unwrap().is_none());

        // After releasing, a fresh try succeeds.
        drop(held);
        let again = try_acquire(&paths).unwrap();
        assert!(again.is_some());
    }

    #[test]
    fn acquire_bounded_times_out_while_held() {
        // While the lock is held on a second in-process fd, a bounded acquire
        // must give up within ~the bound and return a `WouldBlock` Io error —
        // never wait indefinitely (FR-021a).
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);

        let _held = acquire_bounded(&paths, Duration::from_secs(1)).unwrap();

        let start = Instant::now();
        let err = acquire_bounded(&paths, Duration::from_millis(100)).unwrap_err();
        let elapsed = start.elapsed();

        match err {
            TomeError::Io(e) => assert_eq!(
                e.kind(),
                std::io::ErrorKind::WouldBlock,
                "bounded acquire timeout must be a WouldBlock Io error"
            ),
            other => panic!("expected Io(WouldBlock), got {other:?}"),
        }
        // It honoured the bound and did not block forever (generous ceiling to
        // absorb a slow/contended CI scheduler).
        assert!(
            elapsed < Duration::from_secs(2),
            "bounded acquire must return within ~the bound, took {elapsed:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn lock_file_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let _guard = acquire_bounded(&paths, Duration::from_secs(1)).unwrap();
        let mode = std::fs::metadata(paths.telemetry_flush_lock())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

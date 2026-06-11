//! Local-only telemetry identity: the install UUID, the per-process session
//! UUID, upgrade detection, and the explicit `reset`/`purge` operations.
//!
//! The install id is THE funnel join key (the anonymous + catalog-attributed
//! streams share it). It is minted once, stored at `telemetry/id` (mode `0600`),
//! and never network-derived. The session id is minted once per process and
//! never persisted — it links the events of a single run without ever touching
//! disk.

use std::io::{ErrorKind, Write};
use std::sync::OnceLock;

use time::OffsetDateTime;

use crate::error::TomeError;
use crate::paths::Paths;
use crate::telemetry::event::{Uuid, VersionStr};
use crate::telemetry::lock;

/// How many times the `AlreadyExists` loser re-reads a still-EMPTY id file
/// before giving up and treating it as corrupt. Each retry sleeps a growing,
/// capped wall-clock interval (see the loop below), so this count bounds a real
/// time budget rather than a scheduling-dependent spin count.
///
/// R-L2: the budget is deliberately small. The legitimate winner-mid-write
/// window (two adjacent `write_all`s, no I/O between) is sub-millisecond, so the
/// early exponential sleeps already cover it with wide margin; the count + cap
/// below bound the worst-case FOREGROUND block (a rare crashed-mint empty file)
/// to ~6 ms, not the prior ~83 ms.
const RACE_READ_RETRIES: usize = 12;

/// Re-assert `0600` on the id file after an atomic *replace*.
///
/// `write_atomic` PRESERVES the existing target's mode (it copies onto the prior
/// file's permissions), so a re-mint over a loosened id would inherit the loose
/// mode. The fresh-mint path uses `OpenOptions::mode(0o600)` and is already
/// tight; this helper covers every atomic-replace re-mint (corrupt / empty /
/// reset) so a widened mode can never persist. Best-effort + Unix-only — a
/// platform/FS that rejects the chmod leaves the (already-written) id intact.
#[cfg(unix)]
fn reassert_id_0600(paths: &Paths) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(paths.telemetry_id(), std::fs::Permissions::from_mode(0o600));
}

/// No-op on non-Unix (the mode model does not apply).
#[cfg(not(unix))]
fn reassert_id_0600(_paths: &Paths) {}

/// Ensure the `telemetry/` directory exists with a `0600`-friendly (`0700`)
/// mode. Idempotent. Mirrors `lock::open_lock_file`'s dir landing.
fn ensure_dir(paths: &Paths) -> Result<(), TomeError> {
    let dir = paths.telemetry_dir();
    std::fs::create_dir_all(&dir).map_err(TomeError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 0700: the whole telemetry tree is owner-only. Best-effort — the lock
        // semantics and the per-file 0600 modes do not depend on the dir mode.
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Race-safe mint-or-read of the install UUID.
///
/// Returns `(uuid, just_minted)`: `just_minted` is `true` exactly when THIS call
/// created (or re-minted over a corrupt) id file — the caller uses it to print
/// the first-run notice exactly once (the O_EXCL create is the once-guarantee,
/// not a separate marker file).
///
/// Concurrency: two racing processes both attempt `create_new` (O_CREAT|O_EXCL,
/// atomic). Exactly one wins and writes the fresh id; the loser hits
/// `AlreadyExists` and re-reads the winner's id, returning `just_minted = false`.
///
/// Corruption: if a present id file does not parse as a v4 UUID it is treated as
/// unrecoverable (a corrupt join key cannot be repaired — there is no "correct"
/// value to restore) and RE-MINTED via an atomic replace, returning
/// `just_minted = true`.
///
/// Foreground-block bound (R-L2, qualifying NFR-001's "no wait"): the only path
/// that sleeps is the rare EMPTY-id race-retry (a winner caught mid-write, or a
/// crashed mint that left a zero-byte file). The happy path (mint, or read a
/// valid id) does NOT sleep. When it does, the bounded retry budget caps the
/// total foreground block at ~6 ms before the file is treated as corrupt and
/// re-minted — so even the pathological case stays well under a perceptible wait.
pub fn ensure_install_id(paths: &Paths) -> Result<(Uuid, bool), TomeError> {
    ensure_dir(paths)?;
    let path = paths.telemetry_id();

    // Atomic create-or-fail. The O_EXCL guarantees exactly one racer creates.
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }

    match opts.open(&path) {
        Ok(mut file) => {
            // We won the race (or it is a fresh install): write a new id.
            let uuid = Uuid::mint();
            file.write_all(uuid.as_str().as_bytes())
                .map_err(TomeError::Io)?;
            file.write_all(b"\n").map_err(TomeError::Io)?;
            // fsync is best-effort durability; a lost id just re-mints next run.
            let _ = file.sync_all();
            Ok((uuid, true))
        }
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {
            // Someone else holds the id (a prior run, or the race winner). Read
            // and validate it.
            //
            // Read/write containment parity: the write side (`write_atomic`)
            // refuses a symlinked component; the read side must too, or a hostile
            // `telemetry/id` symlink/FIFO could redirect/block this read. Refuse
            // up front and fail closed (the guard returns `Ok(())` for a normal
            // regular file, so this is inert on the happy path).
            crate::util::refuse_symlinked_component(&path).map_err(TomeError::Io)?;
            //
            // Race subtlety: `create_new` (O_EXCL) is atomic, but the winner's
            // *write* is not — between its create and its first `write_all` the
            // file exists but is EMPTY. A loser that reads in that window would
            // see "" and, if we treated empty as corrupt, wrongly re-mint and
            // clobber the winner's id (two minters). So we distinguish:
            //   - an EMPTY id ⇒ the winner is mid-write ⇒ briefly retry the read;
            //   - a NON-empty but unparsable id ⇒ genuine corruption ⇒ re-mint.
            // The retry budget is tiny: an id write is two adjacent `write_all`s
            // with no I/O in between, so the empty window is sub-millisecond.
            for attempt in 0..RACE_READ_RETRIES {
                let contents =
                    crate::util::bounded_read_to_string(&path, crate::util::TOME_CONFIG_MAX)?;
                let first = contents.lines().next().unwrap_or("").trim();

                if let Some(uuid) = Uuid::parse(first) {
                    return Ok((uuid, false));
                }
                if !first.is_empty() {
                    // Non-empty garbage: unrecoverable corruption — re-mint via
                    // an atomic replace (a corrupt join key cannot be repaired).
                    let uuid = Uuid::mint();
                    let mut line = uuid.as_str().to_string();
                    line.push('\n');
                    crate::catalog::store::write_atomic(&path, line.as_bytes())?;
                    // write_atomic preserves the prior file's mode; re-tighten.
                    reassert_id_0600(paths);
                    return Ok((uuid, true));
                }
                // Empty: the winner is mid-write. Sleep a growing, capped
                // interval and retry. A real wall-clock sleep (not `yield_now`)
                // is deliberate: under slow-FS / heavy-load a busy-spin can burn
                // all retries *inside* the winner's sub-millisecond empty window
                // and wrongly re-mint over it (two minters). The growing sleep
                // (~25µs doubling, capped at attempt 5 ≈ 800µs) gives the winner
                // a guaranteed wall-clock budget regardless of scheduling, while
                // bounding the total worst-case foreground block to ~6 ms (R-L2).
                if attempt + 1 < RACE_READ_RETRIES {
                    std::thread::sleep(std::time::Duration::from_micros(25u64 << attempt.min(5)));
                }
            }

            // Exhausted retries on a persistently-empty file: treat as corrupt
            // (a stale zero-byte id from a crashed mint) and re-mint.
            let uuid = Uuid::mint();
            let mut line = uuid.as_str().to_string();
            line.push('\n');
            crate::catalog::store::write_atomic(&path, line.as_bytes())?;
            // write_atomic preserves the prior file's mode; re-tighten.
            reassert_id_0600(paths);
            Ok((uuid, true))
        }
        Err(e) => Err(TomeError::Io(e)),
    }
}

/// The install-id mint time, sourced from the `telemetry/id` file's mtime.
///
/// WHY the mtime (data-model §8): the grace period ([`super::clock::grace_period_active`])
/// needs the moment the id was minted, but the id file holds only the UUID, not
/// a timestamp. The file is written exactly once (at mint) and then only ever
/// atomic-*replaced* (reset/re-mint), so its mtime IS the mint time. Returns
/// `None` if the file is absent or its mtime is unreadable — the caller treats a
/// missing mint time as "hold delivery" (fail-safe).
pub fn install_mint_time(paths: &Paths) -> Option<OffsetDateTime> {
    let meta = std::fs::metadata(paths.telemetry_id()).ok()?;
    let modified = meta.modified().ok()?;
    // `OffsetDateTime: From<SystemTime>` (time `std` feature) — UTC instant.
    Some(OffsetDateTime::from(modified))
}

/// The per-process session UUID: minted once on first call, cached, never
/// persisted. Subsequent calls return a clone of the same value.
pub fn session_id() -> Uuid {
    static SESSION: OnceLock<Uuid> = OnceLock::new();
    SESSION.get_or_init(Uuid::mint).clone()
}

/// Detect (and record) a version change since the last run.
///
/// Reads `telemetry/last-version` and compares it to the running binary's
/// `CARGO_PKG_VERSION`:
/// - file ABSENT ⇒ first run (an install, NOT an upgrade): stamp `current`,
///   return `Ok(None)`.
/// - file == current ⇒ no change: return `Ok(None)`.
/// - file != current ⇒ an upgrade: stamp `current`, return `Ok(Some(prior))`
///   so the caller can emit `tome.upgrade { from_version: prior }`.
///
/// The stamp write is atomic + `0600` (via `write_atomic`).
pub fn detect_and_record_version(paths: &Paths) -> Result<Option<VersionStr>, TomeError> {
    ensure_dir(paths)?;
    let path = paths.telemetry_last_version();
    let current = env!("CARGO_PKG_VERSION");

    let prior = match crate::util::bounded_read_to_string(&path, crate::util::TOME_CONFIG_MAX) {
        Ok(s) => Some(s.lines().next().unwrap_or("").trim().to_string()),
        Err(TomeError::Io(e)) if e.kind() == ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };

    match prior {
        // First run: stamp current, NOT an upgrade.
        None => {
            stamp_version(&path, current)?;
            Ok(None)
        }
        // Unchanged (also treats an empty/blank stamp as "no detectable prior").
        Some(p) if p == current || p.is_empty() => Ok(None),
        // Changed: record current, report the prior version.
        Some(p) => {
            stamp_version(&path, current)?;
            Ok(Some(VersionStr::from_last_version(p)))
        }
    }
}

/// Atomic-write `version\n` to the `last-version` stamp (0600 via `write_atomic`).
fn stamp_version(path: &std::path::Path, version: &str) -> Result<(), TomeError> {
    let mut body = version.to_string();
    body.push('\n');
    crate::catalog::store::write_atomic(path, body.as_bytes())
}

/// Reset the install identity: mint a FRESH install UUID and clear the queue,
/// severing all continuity with the prior id (FR-021a).
///
/// Serialises on the flush lock FIRST (held for the whole operation) so a
/// concurrent flush cannot read a half-rewritten id or drain the queue we are
/// clearing. The lock releases when the returned guard drops at end of scope.
/// Returns the new install UUID.
pub fn reset(paths: &Paths) -> Result<Uuid, TomeError> {
    // Hold the lock for the full reset; `_guard` drops (unlocks) on return.
    // BOUNDED acquire (FR-021a): a hung flusher must never block reset forever.
    let _guard = lock::acquire_bounded(paths, std::time::Duration::from_secs(3))?;

    ensure_dir(paths)?;
    let uuid = Uuid::mint();
    let mut line = uuid.as_str().to_string();
    line.push('\n');
    // Atomic replace of the id — symlink-safe. write_atomic preserves the prior
    // file's mode, so re-tighten to 0600 (a loosened mode must not persist).
    crate::catalog::store::write_atomic(&paths.telemetry_id(), line.as_bytes())?;
    reassert_id_0600(paths);

    clear_queue(paths)?;
    Ok(uuid)
}

/// Purge ALL telemetry state and switch telemetry OFF until explicitly re-enabled.
///
/// Serialises on the flush lock first (same rationale as [`reset`]). Deletes the
/// install id (ignoring a missing file), clears the queue, and writes
/// `enabled = false`. Telemetry then stays off until `tome telemetry on`.
pub fn purge(paths: &Paths) -> Result<(), TomeError> {
    // BOUNDED acquire (FR-021a): never wait indefinitely on a hung flusher.
    let _guard = lock::acquire_bounded(paths, std::time::Duration::from_secs(3))?;

    // Delete the id; a missing file is fine (nothing to purge there).
    match std::fs::remove_file(paths.telemetry_id()) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(TomeError::Io(e)),
    }

    clear_queue(paths)?;

    // Opt OUT: stays off until the user re-enables.
    crate::telemetry::config::set_enabled(paths, false)
}

/// Clear the JSONL queue: remove it if present, ignore a missing file. (Removal
/// rather than truncation keeps the next append's `create` path identical to a
/// fresh install.)
fn clear_queue(paths: &Paths) -> Result<(), TomeError> {
    match std::fs::remove_file(paths.telemetry_queue()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(TomeError::Io(e)),
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
    fn mint_creates_single_line_valid_v4_id() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let (uuid, just_minted) = ensure_install_id(&paths).unwrap();
        assert!(just_minted, "first call must mint");

        // The on-disk file is exactly one trimmed line that re-parses to the id.
        let body = std::fs::read_to_string(paths.telemetry_id()).unwrap();
        assert_eq!(body.lines().count(), 1, "id file is one line: {body:?}");
        let stored = Uuid::parse(body.trim()).expect("stored id parses as v4");
        assert_eq!(stored.as_str(), uuid.as_str());
    }

    #[cfg(unix)]
    #[test]
    fn mint_writes_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        ensure_install_id(&paths).unwrap();
        let mode = std::fs::metadata(paths.telemetry_id())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn second_call_reads_back_same_id_not_minted() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let (first, minted1) = ensure_install_id(&paths).unwrap();
        let (second, minted2) = ensure_install_id(&paths).unwrap();
        assert!(minted1 && !minted2, "only the first call mints");
        assert_eq!(first.as_str(), second.as_str());
    }

    #[test]
    fn concurrent_racers_agree_on_one_id_with_one_minter() {
        // Two threads both call `ensure_install_id` on the SAME fresh dir. The
        // O_EXCL create guarantees exactly one wins (just_minted=true); the
        // loser re-reads the winner's id (just_minted=false). They must agree.
        use std::sync::Arc;
        use std::sync::Barrier;

        let dir = TempDir::new().unwrap();
        let root = dir.path().to_path_buf();
        let barrier = Arc::new(Barrier::new(2));

        let handles: Vec<_> = (0..2)
            .map(|_| {
                let root = root.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let paths = Paths::from_root(root);
                    barrier.wait();
                    ensure_install_id(&paths).unwrap()
                })
            })
            .collect();

        let results: Vec<(Uuid, bool)> = handles.into_iter().map(|h| h.join().unwrap()).collect();

        // Exactly one minter.
        let minters = results.iter().filter(|(_, m)| *m).count();
        assert_eq!(minters, 1, "exactly one racer mints");
        // Both observed the same id.
        assert_eq!(results[0].0.as_str(), results[1].0.as_str());
    }

    #[test]
    fn corrupt_id_is_re_minted() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        ensure_dir(&paths).unwrap();
        // Plant garbage that is not a valid v4 UUID.
        std::fs::write(paths.telemetry_id(), b"not-a-uuid\n").unwrap();

        let (uuid, just_minted) = ensure_install_id(&paths).unwrap();
        assert!(just_minted, "a corrupt id re-mints");
        // The file now holds a valid id matching the returned one.
        let body = std::fs::read_to_string(paths.telemetry_id()).unwrap();
        assert_eq!(Uuid::parse(body.trim()).unwrap().as_str(), uuid.as_str());
    }

    #[test]
    fn mint_time_reads_the_id_mtime() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // Absent before mint.
        assert!(install_mint_time(&paths).is_none());
        ensure_install_id(&paths).unwrap();
        // Present and recent after mint (sanity: within a wide window of now).
        let mint = install_mint_time(&paths).expect("mint time after mint");
        let delta = (OffsetDateTime::now_utc() - mint).abs();
        assert!(
            delta < time::Duration::minutes(5),
            "mtime is recent: {delta}"
        );
    }

    #[test]
    fn session_id_is_stable_within_process() {
        let a = session_id();
        let b = session_id();
        assert_eq!(a.as_str(), b.as_str());
        // And it is a valid v4 id.
        assert!(Uuid::parse(a.as_str()).is_some());
    }

    #[test]
    fn detect_version_none_on_first_run_then_persists_current() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // First run: no stamp ⇒ install, not upgrade.
        assert!(detect_and_record_version(&paths).unwrap().is_none());
        // Current is now persisted.
        let stamped = std::fs::read_to_string(paths.telemetry_last_version()).unwrap();
        assert_eq!(stamped.trim(), env!("CARGO_PKG_VERSION"));
        // Same version on the next run ⇒ still None.
        assert!(detect_and_record_version(&paths).unwrap().is_none());
    }

    #[test]
    fn detect_version_some_old_on_change() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        ensure_dir(&paths).unwrap();
        // Plant a different prior version.
        std::fs::write(paths.telemetry_last_version(), b"0.0.1\n").unwrap();

        let prior = detect_and_record_version(&paths).unwrap();
        assert_eq!(
            prior.map(|v| v.as_str().to_string()),
            Some("0.0.1".to_string())
        );
        // Current is now stamped.
        let stamped = std::fs::read_to_string(paths.telemetry_last_version()).unwrap();
        assert_eq!(stamped.trim(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn reset_changes_uuid_and_clears_queue() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let (original, _) = ensure_install_id(&paths).unwrap();
        // Seed a queue file.
        std::fs::write(paths.telemetry_queue(), b"{}\n").unwrap();

        let fresh = reset(&paths).unwrap();
        assert_ne!(original.as_str(), fresh.as_str(), "reset mints a new id");
        // Queue cleared.
        assert!(!paths.telemetry_queue().exists());
        // On-disk id matches the returned fresh id.
        let body = std::fs::read_to_string(paths.telemetry_id()).unwrap();
        assert_eq!(Uuid::parse(body.trim()).unwrap().as_str(), fresh.as_str());
    }

    #[cfg(unix)]
    #[test]
    fn reset_re_asserts_0600_even_over_a_loosened_id() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        ensure_install_id(&paths).unwrap();
        // Loosen the existing id's mode; `write_atomic` would otherwise preserve
        // it across the reset re-mint.
        std::fs::set_permissions(paths.telemetry_id(), std::fs::Permissions::from_mode(0o644))
            .unwrap();

        reset(&paths).unwrap();

        let mode = std::fs::metadata(paths.telemetry_id())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "reset must re-tighten the id to 0600");
    }

    #[cfg(unix)]
    #[test]
    fn corrupt_re_mint_re_asserts_0600_even_over_a_loosened_id() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        ensure_dir(&paths).unwrap();
        // Plant loosened garbage so the AlreadyExists re-mint path runs.
        std::fs::write(paths.telemetry_id(), b"not-a-uuid\n").unwrap();
        std::fs::set_permissions(paths.telemetry_id(), std::fs::Permissions::from_mode(0o644))
            .unwrap();

        let (_uuid, just_minted) = ensure_install_id(&paths).unwrap();
        assert!(just_minted, "corrupt id re-mints");
        let mode = std::fs::metadata(paths.telemetry_id())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "corrupt re-mint must re-tighten to 0600"
        );
    }

    #[test]
    fn empty_id_is_re_minted_after_retry_window() {
        // T5 (deterministic empty-window fall-through): a pre-existing EMPTY id
        // file (a crashed/mid-write mint with NO live writer) must, after the
        // bounded retry exhausts, be re-minted to a valid v4 id (just_minted).
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        ensure_dir(&paths).unwrap();
        // Zero-byte id — the create_new hits AlreadyExists, every read sees "".
        std::fs::write(paths.telemetry_id(), b"").unwrap();

        let (uuid, just_minted) = ensure_install_id(&paths).unwrap();
        assert!(
            just_minted,
            "a persistently-empty id (no live winner) re-mints after the retry budget"
        );
        assert!(
            Uuid::parse(uuid.as_str()).is_some(),
            "re-minted id is a valid v4 uuid"
        );
        // On-disk file now holds the returned id, one line.
        let body = std::fs::read_to_string(paths.telemetry_id()).unwrap();
        assert_eq!(body.lines().count(), 1);
        assert_eq!(Uuid::parse(body.trim()).unwrap().as_str(), uuid.as_str());
    }

    #[test]
    fn purge_removes_id_clears_queue_and_disables() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        ensure_install_id(&paths).unwrap();
        std::fs::write(paths.telemetry_queue(), b"{}\n").unwrap();

        purge(&paths).unwrap();
        assert!(!paths.telemetry_id().exists(), "id removed");
        assert!(!paths.telemetry_queue().exists(), "queue cleared");
        // Telemetry now reads disabled from the config file.
        assert!(!crate::telemetry::config::load(&paths).unwrap().enabled);
    }
}

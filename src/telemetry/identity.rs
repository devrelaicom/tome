//! Tome-owned telemetry identity helpers that the kernel does NOT cover.
//!
//! The kernel (`gauge-telemetry`) owns the install UUID (the funnel join key),
//! the per-process session UUID, and `reset`. The ONE identity concern that stays
//! Tome's is **upgrade detection** — comparing the running binary's version to a
//! Tome-written `last-version` stamp — because it is a product signal specific to
//! Tome's release cadence, not a kernel concern.

use std::io::ErrorKind;

use crate::error::TomeError;
use crate::paths::Paths;

/// Ensure the `telemetry/` directory exists with an owner-only (`0700`) mode.
/// Idempotent. The kernel creates the dir lazily on its first emit, but the
/// upgrade-stamp write below can run before any emit, so we land it here too.
fn ensure_dir(paths: &Paths) -> Result<(), TomeError> {
    let dir = paths.telemetry_dir();
    std::fs::create_dir_all(&dir).map_err(TomeError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // 0700: the whole telemetry tree is owner-only. Best-effort.
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

/// Detect (and record) a version change since the last run.
///
/// Reads `telemetry/last-version` and compares it to the running binary's
/// `CARGO_PKG_VERSION`:
/// - file ABSENT ⇒ first run (an install, NOT an upgrade): stamp `current`,
///   return `Ok(None)`.
/// - file == current ⇒ no change: return `Ok(None)`.
/// - file != current ⇒ an upgrade: stamp `current`, return `Ok(Some(prior))` so
///   the caller can emit `tome.upgrade { from_version: prior }`. `prior` is
///   Tome's OWN prior version (it was written by a previous Tome run into a `0600`
///   stamp file), never user input.
///
/// The stamp write is atomic + `0600` (via `write_atomic`).
pub fn detect_and_record_version(paths: &Paths) -> Result<Option<String>, TomeError> {
    ensure_dir(paths)?;
    let path = paths.telemetry_last_version();
    let current = env!("CARGO_PKG_VERSION");

    // Read/write containment parity — `stamp_version` writes via `write_atomic`
    // (symlink-safe); the read must refuse a symlinked component too. A hostile
    // `last-version` symlink is treated as ABSENT (degrade the refusal to the
    // first-run branch), never propagated/blocked.
    let prior = match crate::util::refuse_symlinked_component(&path) {
        Err(_) => None,
        Ok(()) => match crate::util::bounded_read_to_string(&path, crate::util::TOME_CONFIG_MAX) {
            Ok(s) => Some(s.lines().next().unwrap_or("").trim().to_string()),
            Err(TomeError::Io(e)) if e.kind() == ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        },
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
            Ok(Some(p))
        }
    }
}

/// Atomic-write `version\n` to the `last-version` stamp (0600 via `write_atomic`).
fn stamp_version(path: &std::path::Path, version: &str) -> Result<(), TomeError> {
    let mut body = version.to_string();
    body.push('\n');
    crate::catalog::store::write_atomic(path, body.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn paths_in(dir: &TempDir) -> Paths {
        Paths::from_root(dir.path().to_path_buf())
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
        assert_eq!(prior, Some("0.0.1".to_string()));
        // Current is now stamped.
        let stamped = std::fs::read_to_string(paths.telemetry_last_version()).unwrap();
        assert_eq!(stamped.trim(), env!("CARGO_PKG_VERSION"));
    }

    #[cfg(unix)]
    #[test]
    fn stamp_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        detect_and_record_version(&paths).unwrap();
        let mode = std::fs::metadata(paths.telemetry_last_version())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}

//! Sweep stale `.tome.tmp.*` staging directories left behind by
//! crashed / SIGINT-interrupted atomic-directory landings (FR-410).
//!
//! The atomic-rename pattern in [`crate::util::atomic_dir`] creates a
//! sibling staging dir named `.tome.tmp.<random>` next to its target,
//! populates it, then `rename(2)`s it into place. A crash between
//! `TempDir::keep()` and the final rename leaves the staging dir on
//! disk as an orphan. `doctor --fix` finds and removes those orphans.
//!
//! ## Where we sweep
//!
//! Two parent directories can legitimately contain staging dirs:
//!
//! 1. `<root>/workspaces/` — orphans from `tome workspace init`,
//!    `tome workspace rename`, `tome workspace regen-summary` (the
//!    workspace dir itself is built atomically).
//! 2. Every bound project's parent directory — orphans from
//!    `tome workspace use` (the `<project>/.tome/` marker dir lands
//!    atomically; the staging sibling is `<project>/.tome.tmp.<rand>`).
//!    "Parent of the marker" is the project root itself.
//!
//! Other Tome paths (catalog clones, model dirs, the central DB) are
//! NOT atomic-rename landed, so they don't produce staging orphans.
//!
//! ## Age gate
//!
//! Per FR-410, only staging dirs older than 1 hour are removed. The
//! gate exists to avoid TOCTOU with a concurrently-running atomic
//! landing that legitimately staged a directory and is racing toward
//! the rename. One hour is far longer than any plausible atomic
//! population step (the slowest case — writing RULES.md after a
//! summariser pass — is single-digit seconds).
//!
//! ## Read-only when no orphans are present
//!
//! The sweep is a `read_dir` walk + `metadata` per entry. No mutation
//! happens unless an orphan crosses the age gate. A clean install
//! sees zero entries that match the prefix and the function returns
//! `Ok(0)` after one `read_dir` syscall per swept root.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tracing::{debug, warn};

use crate::error::TomeError;
use crate::index;
use crate::paths::Paths;
use crate::util::atomic_dir::STAGING_PREFIX;

/// How old a staging directory must be before doctor will sweep it.
/// Conservatively higher than any plausible legitimate atomic-landing
/// duration, to keep the race-window with active writers wide.
pub const STAGING_AGE_GATE: Duration = Duration::from_secs(60 * 60);

/// Sweep every `.tome.tmp.*` directory under the known atomic-landing
/// parents, removing those older than [`STAGING_AGE_GATE`]. Returns
/// the count of directories removed.
///
/// Errors at the filesystem layer are downgraded to `warn!` per entry —
/// doctor's auto-fix discipline (FR-M-DOC-4) says "continue on per-fix
/// failure". A returned `Err` means we couldn't even enumerate the
/// sweep roots, which is itself a bigger failure than any single
/// dangling orphan.
pub fn cleanup_stale_staging_dirs(paths: &Paths) -> Result<usize, TomeError> {
    let mut removed = 0usize;

    // 1. Central workspaces root.
    removed += sweep_one(&paths.workspaces_dir);

    // 2. Every bound project's parent directory.
    //    "Parent of the marker dir" = the project root.
    for project_root in bound_project_roots(paths)? {
        removed += sweep_one(&project_root);
    }

    Ok(removed)
}

/// Read the central DB for every row in `workspace_projects` and
/// return the canonical-ish project root paths. Bootstrap-not-yet (no
/// DB on disk) yields the empty vector — no projects to sweep.
///
/// DB read failures are not fatal: we return whatever we managed to
/// collect and the caller's outer sweep continues against the
/// workspaces-dir.
fn bound_project_roots(paths: &Paths) -> Result<Vec<PathBuf>, TomeError> {
    if !paths.index_db.is_file() {
        return Ok(Vec::new());
    }
    let conn = match index::open_read_only(&paths.index_db) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "doctor orphan-cleanup: index open_read_only failed; skipping project sweep");
            return Ok(Vec::new());
        }
    };
    let mut stmt = match conn.prepare("SELECT project_path FROM workspace_projects") {
        Ok(s) => s,
        Err(e) => {
            warn!(error = %e, "doctor orphan-cleanup: prepare workspace_projects failed; skipping project sweep");
            return Ok(Vec::new());
        }
    };
    let mut out = Vec::new();
    let rows = match stmt.query_map([], |row| {
        let p: String = row.get(0)?;
        Ok(p)
    }) {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "doctor orphan-cleanup: query workspace_projects failed; skipping project sweep");
            return Ok(Vec::new());
        }
    };
    for r in rows.flatten() {
        out.push(PathBuf::from(r));
    }
    Ok(out)
}

/// Sweep one parent directory. Per-entry errors are warn'd and
/// skipped so a single un-readable entry doesn't break the rest.
fn sweep_one(parent: &Path) -> usize {
    let read_dir = match std::fs::read_dir(parent) {
        Ok(r) => r,
        // NotFound is the common case (workspaces dir absent on fresh
        // install; project dir gone). Silent skip.
        Err(_) => return 0,
    };
    let mut removed = 0;
    for entry in read_dir.flatten() {
        let path = entry.path();
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue, // non-UTF8 name; can't be a tome staging
        };
        if !name.starts_with(STAGING_PREFIX) {
            continue;
        }
        // S-M6: refuse symlinks BEFORE `metadata` (which follows
        // symlinks). A hostile symlink at `<project>/.tome.tmp.evil`
        // pointing at a sensitive directory must be skipped, not
        // followed by `remove_dir_all`. The discipline matches
        // `mcp/tools/get_skill.rs::walk_dir`.
        let symlink_meta = match entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                debug!(path = %path.display(), error = %e, "orphan-cleanup: file_type failed");
                continue;
            }
        };
        if symlink_meta.is_symlink() {
            debug!(
                path = %path.display(),
                "orphan-cleanup: refusing symlink; skipping",
            );
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(e) => {
                debug!(path = %path.display(), error = %e, "orphan-cleanup: metadata failed");
                continue;
            }
        };
        if !meta.is_dir() {
            // Suspicious — `.tome.tmp.*` is always a dir per the
            // atomic_dir contract. Skip rather than risk removing a
            // user-authored regular file with the prefix.
            continue;
        }
        let mtime = match meta.modified() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let age = match SystemTime::now().duration_since(mtime) {
            Ok(d) => d,
            // Future mtime → skip (clock skew; race-safe).
            Err(_) => continue,
        };
        if age < STAGING_AGE_GATE {
            debug!(
                path = %path.display(),
                age_secs = age.as_secs(),
                "orphan-cleanup: staging dir within age gate; leaving in place",
            );
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => {
                debug!(path = %path.display(), "orphan-cleanup: removed stale staging dir");
                removed += 1;
            }
            Err(e) => {
                warn!(path = %path.display(), error = %e, "orphan-cleanup: remove_dir_all failed");
            }
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn fresh_staging_dir_is_kept() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        let fresh = parent.join(format!("{}fresh123", STAGING_PREFIX));
        std::fs::create_dir(&fresh).unwrap();
        // Just created — well within the age gate.
        let removed = sweep_one(parent);
        assert_eq!(removed, 0);
        assert!(fresh.exists(), "fresh staging dir must not be removed");
    }

    #[test]
    fn unrelated_dirs_are_ignored() {
        let tmp = TempDir::new().unwrap();
        let parent = tmp.path();
        let unrelated = parent.join("just-a-dir");
        std::fs::create_dir(&unrelated).unwrap();
        let removed = sweep_one(parent);
        assert_eq!(removed, 0);
        assert!(unrelated.exists());
    }

    #[test]
    fn missing_parent_is_silent_no_op() {
        let tmp = TempDir::new().unwrap();
        let nonexistent = tmp.path().join("nope");
        assert_eq!(sweep_one(&nonexistent), 0);
    }

    /// Backdating mtime to test the stale path requires platform-specific
    /// syscalls. The integration test in `tests/doctor_orphan_tmp_cleanup.rs`
    /// uses the standard `filetime` crate (a transitive dep via
    /// `tempfile`) gated on whether it's actually available; here we
    /// settle for proving the age-gate logic via the public function's
    /// other branches.
    #[test]
    fn cleanup_returns_zero_on_fresh_paths() {
        let tmp = TempDir::new().unwrap();
        let paths = Paths::from_root(tmp.path().to_path_buf());
        std::fs::create_dir_all(&paths.workspaces_dir).unwrap();
        // Create one fresh staging dir; should be kept.
        let staging = paths.workspaces_dir.join(format!("{}abc", STAGING_PREFIX));
        std::fs::create_dir(&staging).unwrap();
        let removed = cleanup_stale_staging_dirs(&paths).unwrap();
        assert_eq!(removed, 0);
        assert!(staging.exists());
    }
}

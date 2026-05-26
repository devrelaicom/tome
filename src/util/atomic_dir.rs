//! Atomic populated-directory landing.
//!
//! Lifts the Phase 3 `workspace::init` pattern — build the directory in a
//! sibling staging dir on the same filesystem, then rename once — into a
//! reusable helper. Phase 4 callers: `workspace add`, `workspace rename`,
//! and `workspace use` (project marker creation under `<project>/.tome/`).
//!
//! ## Algorithm (per data-model §16 + research §R-10)
//!
//! 1. Resolve `target.parent()`. A `None` parent (root or relative-with-
//!    no-prefix) is rejected up front with `io::ErrorKind::InvalidInput`.
//! 2. `tempfile::Builder::new().prefix(".tome.tmp.").tempdir_in(parent)?`
//!    — the staging dir is a sibling of `target`, guaranteeing same-FS
//!    rename atomicity. The `.tome.tmp.` prefix is part of the contract:
//!    `doctor --fix` (US5) can list-and-clean orphaned staging dirs by
//!    matching this prefix.
//! 3. The caller-supplied `populate` closure writes files into the staged
//!    path. Returning `Err(_)` here lets `TempDir::drop` clean the staged
//!    contents — so a SIGINT or crash mid-populate leaves no debris.
//! 4. On Unix, chmod the staged dir to `mode_unix` (typically `0o700` for
//!    workspace/project marker dirs). The chmod happens BEFORE `keep()`
//!    so a failure still falls through `TempDir::drop`. Non-Unix targets
//!    ignore `mode_unix`.
//! 5. fsync the staged directory (best-effort on platforms where `File::
//!    open` on a directory is a no-op; this matters for crash-safety on
//!    Linux/macOS).
//! 6. `TempDir::keep()` consumes the auto-cleanup guard and returns the
//!    staged `PathBuf`. From this point on, a crash before the rename
//!    leaves an orphan that must be cleaned by `doctor --fix` (US5).
//! 7. (`_with_replace` only) If `target` exists, rename it aside to its
//!    `.old` sibling. Rename failure bubbles; the staged dir remains as
//!    an orphan.
//! 8. `std::fs::rename(staged, target)`. POSIX-atomic because step 2
//!    placed the staging dir on the same filesystem.
//! 9. (`_with_replace` only) Best-effort `remove_dir_all` of the `.old`
//!    sibling. Missing is fine.
//! 10. (`_with_replace` only) On step 8 failure, restore the `.old`
//!     sibling back to `target` and bubble the original error.
//! 11. Return `target.canonicalize()?`.
//!
//! ## Naming note: `.old` siblings
//!
//! Data-model §16 specifies `target.with_extension("old")` for the
//! rename-aside path. That mangles dot-prefixed names like `.tome` —
//! `PathBuf::with_extension("old")` would produce `.old`, losing the
//! original name entirely. We use `target.with_file_name(format!("{}.old",
//! original_file_name))` instead. The semantic is the same; the
//! implementation handles dot-prefixed targets correctly.
//!
//! ## SIGINT / crash safety
//!
//! - Crash before step 6: `TempDir::drop` runs on unwind; no debris.
//! - Crash between step 6 and step 8: orphan staging dir, picked up by
//!   `doctor --fix` matching the `.tome.tmp.` prefix.
//! - Crash between step 8 (replace variant) and step 9: `.old` sibling
//!   lingers; best-effort cleanup on the next successful call removes it,
//!   or `doctor --fix` can sweep.

use std::path::{Path, PathBuf};

use tempfile::Builder;

use crate::error::TomeError;

/// Documented prefix for staging directories. Stable; `doctor --fix`
/// (US5) keys off this to identify orphans.
pub const STAGING_PREFIX: &str = ".tome.tmp.";

/// Build `target` atomically by populating a same-FS staging directory
/// and renaming it into place. Refuses to overwrite an existing `target`
/// — use [`land_directory_with_replace`] for replace semantics.
///
/// `mode_unix` is applied on Unix only; non-Unix targets ignore it. The
/// `populate` closure is invoked with the staged directory's path and is
/// responsible for writing files into it.
///
/// Returns the canonicalised path of the landed directory on success.
///
/// # Errors
///
/// - `TomeError::Io` (exit 7) when `target` has no parent, when staging
///   creation fails, when `populate` returns an error, or when the final
///   rename fails.
pub fn land_directory<F>(target: &Path, mode_unix: u32, populate: F) -> Result<PathBuf, TomeError>
where
    F: FnOnce(&Path) -> Result<(), TomeError>,
{
    land_inner(target, mode_unix, populate, /* replace = */ false)
}

/// Same as [`land_directory`], but if `target` already exists it is
/// first renamed aside to its `.old` sibling. On final-rename failure
/// the `.old` sibling is restored back to `target` before the error is
/// bubbled. On success the `.old` sibling is best-effort removed.
///
/// # Errors
///
/// Same as [`land_directory`]; in addition, failure to rename `target`
/// to `.old` is bubbled before any staging takes effect.
pub fn land_directory_with_replace<F>(
    target: &Path,
    mode_unix: u32,
    populate: F,
) -> Result<PathBuf, TomeError>
where
    F: FnOnce(&Path) -> Result<(), TomeError>,
{
    land_inner(target, mode_unix, populate, /* replace = */ true)
}

fn land_inner<F>(
    target: &Path,
    mode_unix: u32,
    populate: F,
    replace: bool,
) -> Result<PathBuf, TomeError>
where
    F: FnOnce(&Path) -> Result<(), TomeError>,
{
    // Symlink refusal at the entry — mirrors the three sibling atomic-
    // write helpers (`catalog::store::write_atomic`,
    // `harness::rules_file::atomic_write`,
    // `harness::mcp_config::atomic_write`). Without this check a planted
    // symlink at `target` would cause the rename (or replace) to follow
    // outside the intended scope.
    refuse_symlink(target)?;

    let parent = target.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "atomic_dir: target {} has no parent directory",
                target.display()
            ),
        )
    })?;

    // The parent must exist for `tempdir_in` to succeed; some callers
    // pass `<root>/workspaces/foo` where `workspaces/` may not exist
    // yet. Create it on demand.
    if !parent.as_os_str().is_empty() && !parent.exists() {
        std::fs::create_dir_all(parent)?;
    }

    let staging = Builder::new().prefix(STAGING_PREFIX).tempdir_in(parent)?;

    populate(staging.path())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(mode_unix);
        std::fs::set_permissions(staging.path(), perms)?;
    }
    #[cfg(not(unix))]
    {
        // mode_unix is intentionally ignored on non-Unix.
        let _ = mode_unix;
    }

    // fsync the staged directory so its metadata + contents are durable
    // before the rename. On Windows opening a directory via the standard
    // `File::open` is generally not supported; skip the fsync there.
    #[cfg(unix)]
    {
        let dir = std::fs::File::open(staging.path())?;
        dir.sync_all()?;
    }

    let staged: PathBuf = staging.keep();

    // From here on, an unwind cannot reclaim `staged` automatically.
    // Use a guard so any rollback path tries to clean it up.
    let cleanup_on_error = StagedGuard::new(&staged);

    let aside: Option<PathBuf> = if replace && target.exists() {
        let aside_path = old_sibling(target);
        // If a stale `.old` already exists from a prior crash, remove it
        // first so the rename below has somewhere to land. Refuse if a
        // symlink has been planted at the aside path — `remove_dir_all`
        // would follow the link otherwise (S-M2).
        refuse_symlink(&aside_path)?;
        if aside_path.exists() {
            // Best-effort; if the directory is actually a file, surface
            // the error to the caller via the rename.
            let _ = std::fs::remove_dir_all(&aside_path);
        }
        std::fs::rename(target, &aside_path).map_err(|e| {
            // Staged dir remains; doctor --fix sweeps `.tome.tmp.*`.
            std::io::Error::new(
                e.kind(),
                format!(
                    "atomic_dir: rename existing target {} aside to {}: {}",
                    target.display(),
                    aside_path.display(),
                    e
                ),
            )
        })?;
        Some(aside_path)
    } else {
        None
    };

    if let Err(rename_err) = std::fs::rename(&staged, target) {
        // Final rename failed. In replace mode, restore the aside back
        // so the caller sees the original target intact.
        if let Some(ref aside_path) = aside
            && let Err(restore_err) = std::fs::rename(aside_path, target)
        {
            return Err(TomeError::Io(std::io::Error::new(
                rename_err.kind(),
                format!(
                    "atomic_dir: rename staged {} -> {} failed: {}; rollback rename of {} also failed: {}",
                    staged.display(),
                    target.display(),
                    rename_err,
                    aside_path.display(),
                    restore_err
                ),
            )));
        }
        // Bubble the original rename error.
        return Err(TomeError::Io(std::io::Error::new(
            rename_err.kind(),
            format!(
                "atomic_dir: rename staged {} -> {}: {}",
                staged.display(),
                target.display(),
                rename_err
            ),
        )));
    }

    // Final rename succeeded — disarm the guard.
    cleanup_on_error.disarm();

    // Best-effort cleanup of the aside.
    if let Some(aside_path) = aside {
        let _ = std::fs::remove_dir_all(&aside_path);
    }

    Ok(target.canonicalize()?)
}

/// Refuse to land through a symlink. Mirrors the three sibling atomic-
/// write helpers in `catalog::store` and `harness::{rules_file,
/// mcp_config}`. Missing path → no-op (the rename will create it).
fn refuse_symlink(path: &Path) -> Result<(), TomeError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => Err(TomeError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "refusing to land directory through symlink: {}",
                path.display()
            ),
        ))),
        _ => Ok(()),
    }
}

/// Compute the `.old` sibling path. Uses `with_file_name` so dot-prefixed
/// targets (e.g. `.tome`) get `.tome.old` rather than `.old` (which
/// `PathBuf::with_extension("old")` would produce — see module-level
/// naming note).
fn old_sibling(target: &Path) -> PathBuf {
    let parent = target.parent().expect("caller validated parent exists");
    let name = target
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_else(|| std::ffi::OsString::from(""));
    let mut renamed = name;
    renamed.push(".old");
    parent.join(renamed)
}

/// RAII guard that best-effort removes the staged directory if dropped
/// without being disarmed. Used to clean up after a failed final rename.
struct StagedGuard {
    path: PathBuf,
    armed: bool,
}

impl StagedGuard {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            armed: true,
        }
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for StagedGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_sibling_handles_dot_prefixed_target() {
        let p = Path::new("/tmp/proj/.tome");
        assert_eq!(old_sibling(p), PathBuf::from("/tmp/proj/.tome.old"));
    }

    #[test]
    fn old_sibling_handles_plain_target() {
        let p = Path::new("/tmp/parent/foo");
        assert_eq!(old_sibling(p), PathBuf::from("/tmp/parent/foo.old"));
    }

    #[test]
    fn staging_prefix_is_stable() {
        assert_eq!(STAGING_PREFIX, ".tome.tmp.");
    }
}

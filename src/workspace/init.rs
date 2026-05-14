//! `tome workspace init` — atomic `.tome/` creation.
//!
//! Contract: `contracts/workspace-init.md`. The atomicity guarantee
//! ("never a partial `.tome/`") is delivered by writing into a
//! sibling temp directory inside the workspace root and renaming
//! once everything is written. The rename is the only step visible
//! to readers.
//!
//! Notable design choices:
//!
//! - The temp directory lives **inside** the workspace root (not in
//!   `$TMPDIR`) so the final rename is on the same filesystem and is
//!   POSIX-atomic. `tempfile::Builder::tempdir_in` handles this.
//! - `--force` renames an existing `.tome/` to `.tome.old/` BEFORE the
//!   new directory lands. A crash between rename and remove leaves an
//!   orphan `.tome.old/` next to the new `.tome/`; doctor reports
//!   that as a cleanup candidate (out of scope for US2).
//! - The opt-in workspace registry (`workspaces.txt`) is appended only
//!   if the file already exists — registration is opt-in (research
//!   §R-15).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::catalog::store as catalog_store;
use crate::config::Config;
use crate::error::TomeError;
use crate::paths::Paths;
use crate::workspace::inventory;

/// What `init` produced. Emitted as a single JSON record by the CLI
/// (`workspace-init.md` §"Output (`--json`)") and consumed by callers
/// that need the post-init paths.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InitOutcome {
    pub workspace: PathBuf,
    pub catalogs: u32,
    pub inherited: bool,
    pub config_path: PathBuf,
    pub index_bootstrapped: bool,
}

pub fn init(
    target_root: &Path,
    inherit_global: bool,
    force: bool,
    paths: &Paths,
) -> Result<InitOutcome, TomeError> {
    // 1. Path must exist and be a directory. We canonicalise so the
    //    outcome record always carries an absolute path, matching the
    //    contract's example output.
    if !target_root.exists() {
        return Err(TomeError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "workspace path `{}` does not exist; create the directory first",
                target_root.display()
            ),
        )));
    }
    if !target_root.is_dir() {
        return Err(TomeError::Io(std::io::Error::other(format!(
            "workspace path `{}` is not a directory",
            target_root.display()
        ))));
    }
    let absolute = std::fs::canonicalize(target_root).map_err(TomeError::Io)?;
    let marker = absolute.join(".tome");
    let marker_exists = marker.is_dir() || marker.exists();
    if marker_exists && !force {
        return Err(TomeError::CatalogAlreadyExists(format!(
            "workspace at {}",
            marker.display()
        )));
    }

    // 2. Build a staging directory inside `absolute` so the final
    //    rename is on the same filesystem.
    let staging = tempfile::Builder::new()
        .prefix(".tome.tmp.")
        .tempdir_in(&absolute)
        .map_err(TomeError::Io)?;

    // 3. Permissions 0700 (Unix) on the staging dir before content
    //    lands so the secret-keeping window starts at directory
    //    creation, not after the first config write.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(staging.path(), perms).map_err(TomeError::Io)?;
    }

    // 4. Compose the per-workspace config and write it via the same
    //    atomic-write helper used by the global registry. Enablement
    //    state is NEVER copied — `--inherit-global` brings catalog
    //    sources across, nothing more.
    let (config, inherited) = build_initial_config(inherit_global, paths)?;
    let staging_config = staging.path().join("config.toml");
    let toml_body =
        toml::to_string_pretty(&config).map_err(|e| TomeError::Internal(anyhow::Error::new(e)))?;
    catalog_store::write_atomic(&staging_config, toml_body.as_bytes())?;

    // 5. Move an existing `.tome/` aside if `--force` allowed us this
    //    far. The aside path is deterministic so doctor can find
    //    orphans later. We best-effort-remove any pre-existing
    //    `.tome.old/` from a prior crash so the rename below
    //    doesn't fail with EEXIST.
    let aside = absolute.join(".tome.old");
    if marker_exists {
        let _ = std::fs::remove_dir_all(&aside);
        std::fs::rename(&marker, &aside).map_err(TomeError::Io)?;
    }

    // 6. Promote the staging dir to its final name. `keep()` drops the
    //    auto-cleanup that the `TempDir` `Drop` impl would otherwise
    //    perform, so the underlying directory survives.
    let staged_path = staging.keep();
    if let Err(e) = std::fs::rename(&staged_path, &marker) {
        // Rollback: try to restore the aside copy so a failed `--force`
        // doesn't leave the user with neither old nor new `.tome/`.
        if marker_exists {
            let _ = std::fs::rename(&aside, &marker);
        }
        // Best-effort: clean up the staged dir so it doesn't sit around
        // confusing the next init attempt.
        let _ = std::fs::remove_dir_all(&staged_path);
        return Err(TomeError::Io(e));
    }

    // 7. Best-effort: remove the aside copy after the new marker
    //    landed.
    if marker_exists {
        let _ = std::fs::remove_dir_all(&aside);
    }

    // 8. Append to the opt-in workspace registry. Silently no-ops when
    //    the user hasn't opted in (file absent).
    inventory::append_if_registry_exists(&paths.workspace_registry, &absolute)?;

    Ok(InitOutcome {
        workspace: absolute.clone(),
        catalogs: u32::try_from(config.catalogs.len()).unwrap_or(u32::MAX),
        inherited,
        config_path: marker.join("config.toml"),
        index_bootstrapped: false,
    })
}

fn build_initial_config(inherit: bool, paths: &Paths) -> Result<(Config, bool), TomeError> {
    if !inherit {
        return Ok((Config::default(), false));
    }
    // `load` returns `Config::default()` when the global config file
    // doesn't exist, so `--inherit-global` on a fresh install seeds an
    // empty `[catalogs]` block — same as the no-flag case. That matches
    // the contract: `--inherit-global` copies what's there, not a
    // hypothetical bootstrap.
    let global = catalog_store::load(&paths.config_file)?;
    Ok((global, true))
}

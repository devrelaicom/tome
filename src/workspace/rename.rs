//! `tome workspace rename <old> <new>` — rename a workspace.
//!
//! Phase 4 / US2.a-2. Contract reference:
//! [`contracts/workspace-commands.md` §`tome workspace rename`].
//!
//! ## Algorithm
//!
//! 1. Refuse if either `<old>` or `<new>` is the reserved `global` name
//!    (exit 15). Cannot rename FROM `global` (it is the privileged
//!    seeded workspace) nor TO `global` (would shadow the reserved
//!    default).
//! 2. Acquire the central advisory lockfile.
//! 3. Open the central DB read-write. Refuse if `<old>` has no row
//!    (exit 13). Refuse if `<new>` already has a row (exit 14).
//! 4. **Pre-check** every bound project: query `workspace_projects` for
//!    `workspace_id = <old.id>` ordered by `project_path`. For each
//!    project, verify the directory exists AND `<project>/.tome/
//!    config.toml` exists. Any miss returns
//!    [`TomeError::WorkspaceMalformed`] (exit 70) with NO state changes.
//! 5. Inside ONE DB transaction:
//!    - For each bound project, atomically rewrite
//!      `<project>/.tome/config.toml` to name `<new>` (per-file
//!      atomic-write via `catalog::store::write_atomic`).
//!    - `UPDATE workspaces SET name = <new> WHERE name = <old>`.
//!    - COMMIT.
//! 6. Outside the transaction (still holding the lock):
//!    `std::fs::rename(<root>/workspaces/<old>/, <root>/workspaces/<new>/)`.
//!    POSIX-atomic on same FS.
//!
//! ## Failure semantics
//!
//! - **Pre-check (step 4) fails**: no state changes; DB row, bound
//!   markers, and the central workspace dir remain under `<old>`.
//! - **Transaction (step 5) fails**: marker `config.toml` rewrites
//!   happen BEFORE the SQL UPDATE within the same DB transaction scope.
//!   The transaction is opened first; per-file marker rewrites land
//!   atomically inside that window; the SQL UPDATE runs last and
//!   commits. If the UPDATE fails, the transaction rolls back the
//!   workspace name change — but the marker rewrites are already on
//!   disk and can't be rolled back. Result: DB stays at `<old>`, some
//!   markers point at `<new>`. The doctor `Binding` subsystem (US5)
//!   flags the orphans; `doctor --fix` re-syncs the markers.
//! - **Step 6 fails** (e.g. cross-FS somehow): the DB transaction is
//!   already committed. Log a hard error; doctor `--fix` is **not**
//!   safe here (the central workspace directory is the canonical
//!   on-disk identity; a cross-FS situation needs manual recovery).

use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::Serialize;
use time::OffsetDateTime;

use crate::catalog::store;
use crate::commands::plugin::registry_seeds;
use crate::error::TomeError;
use crate::index::{self, OpenOptions, acquire_lock};
use crate::paths::Paths;
use crate::workspace::WorkspaceName;

/// Outcome of [`rename`]. Serialised by the CLI's `--json` mode.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RenameOutcome {
    /// The pre-rename workspace name.
    pub old_name: WorkspaceName,
    /// The post-rename workspace name.
    pub new_name: WorkspaceName,
    /// Number of bound projects whose marker `config.toml` was rewritten.
    pub bound_projects_updated: u32,
    /// Absolute on-disk path of the workspace directory after the rename
    /// (`<root>/workspaces/<new>/`).
    pub workspace_dir: PathBuf,
}

/// Rename a workspace. See module-level docs for the full algorithm.
pub fn rename(
    old: WorkspaceName,
    new: WorkspaceName,
    paths: &Paths,
) -> Result<RenameOutcome, TomeError> {
    if old.is_reserved() {
        return Err(TomeError::WorkspaceNameInvalid {
            name: old.as_str().to_owned(),
            reason: "`global` is the privileged seeded workspace; it cannot be renamed".to_owned(),
        });
    }
    if new.is_reserved() {
        return Err(TomeError::WorkspaceNameInvalid {
            name: new.as_str().to_owned(),
            reason: "cannot rename to the reserved `global` workspace name".to_owned(),
        });
    }
    if old == new {
        // Trivial no-op rename — surface as a workspace-name error so
        // the failure routes through the workspace-class exit code (15)
        // rather than the generic usage code (2). Polish R-M12.
        return Err(TomeError::WorkspaceNameInvalid {
            name: new.as_str().to_owned(),
            reason: "rename old and new names are identical (no-op)".to_owned(),
        });
    }

    if let Some(parent) = paths.index_lock.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    }

    let lock = acquire_lock(&paths.index_lock)?;

    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let mut conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
            summariser: summariser_seed,
        },
    )?;

    // Membership: `<old>` must exist.
    // Polish R-M7: route through the consolidated helper.
    let old_id: i64 = crate::index::workspaces::resolve_id_required(&conn, &old)?;

    // Refusal: `<new>` must not exist.
    let new_exists: Option<i64> = crate::index::workspaces::resolve_id_optional(&conn, &new)?;
    if new_exists.is_some() {
        return Err(TomeError::WorkspaceAlreadyExists {
            name: new.as_str().to_owned(),
        });
    }

    // Pre-check: collect bound projects + verify each one's directory
    // exists. Ordered by project_path for deterministic rewrite order.
    let bound_projects: Vec<PathBuf> = {
        let mut stmt = conn
            .prepare(
                "SELECT project_path FROM workspace_projects
                 WHERE workspace_id = ?1
                 ORDER BY project_path",
            )
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "prepare bound_projects for rename: {e}"
                ))
            })?;
        let rows = stmt
            .query_map(rusqlite::params![old_id], |row| {
                let p: String = row.get(0)?;
                Ok(PathBuf::from(p))
            })
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "query bound_projects for rename: {e}"
                ))
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("collect bound_projects for rename: {e}"))
        })?
    };

    for project in &bound_projects {
        if !project.is_dir() {
            return Err(TomeError::WorkspaceMalformed {
                path: project.clone(),
                reason: format!(
                    "bound project directory recorded in central registry is missing on disk \
                     (workspace `{}` cannot be renamed until the binding is repaired or removed)",
                    old.as_str(),
                ),
            });
        }
        let marker_config = Paths::project_marker_config(project);
        if !marker_config.is_file() {
            return Err(TomeError::WorkspaceMalformed {
                path: marker_config,
                reason: format!(
                    "bound project marker `config.toml` is missing for workspace `{}`",
                    old.as_str(),
                ),
            });
        }
    }

    // Open the transaction BEFORE the per-marker rewrite loop so the
    // SQL UPDATE happens inside the same logical scope. Marker rewrites
    // are per-file atomic via `catalog::store::write_atomic`; they
    // cannot be rolled back if the UPDATE fails (see module-level docs
    // for the partial-failure mode and doctor recovery).
    let bound_projects_updated = u32::try_from(bound_projects.len()).unwrap_or(u32::MAX);
    let tx = conn.transaction().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("begin rename transaction: {e}"))
    })?;

    for project in &bound_projects {
        let marker_config = Paths::project_marker_config(project);
        let new_body = rewrite_marker_workspace(&marker_config, new.as_str())?;
        store::write_atomic(&marker_config, new_body.as_bytes())?;
    }

    let now_unix = OffsetDateTime::now_utc().unix_timestamp();
    tx.execute(
        "UPDATE workspaces SET name = ?1, last_used_at = ?2 WHERE id = ?3",
        rusqlite::params![new.as_str(), now_unix, old_id],
    )
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!(
            "update workspaces.name from `{}` to `{}`: {e}",
            old.as_str(),
            new.as_str()
        ))
    })?;
    tx.commit().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("commit rename transaction: {e}"))
    })?;

    // Drop the DB handle so any final WAL checkpoint completes inside
    // the lock window.
    drop(conn);

    // Step 6: rename the central workspace directory atomically.
    //
    // Per Phase 5 / US2.b FR-025: this rename atomically relocates the
    // entire workspace tree, including any `<workspaces>/<old>/plugin-data/`
    // subtree that prior substitution passes created (`${TOME_WORKSPACE_DATA}`
    // resolves under `<workspaces>/<name>/plugin-data/<catalog>/<plugin>/`).
    // If a plugin-data subdir is present pre-rename, the failure path
    // surfaces as the dedicated `WorkspaceDataDirWriteFailed` (exit 25)
    // rather than the generic `Io` (exit 7) — operators reading the exit
    // code learn that the rename's data-dir contract was the affected
    // surface and can route recovery accordingly. If no plugin-data
    // subdir exists (workspace was never invoked with a
    // substitution-bearing entry), the rename failure remains classified
    // as the existing `Io` for backwards-compat with Phase 4 callers.
    let old_dir = paths.workspace_dir(&old);
    let new_dir = paths.workspace_dir(&new);
    if old_dir.exists() {
        let old_data_dir = old_dir.join("plugin-data");
        let new_data_dir = new_dir.join("plugin-data");
        let data_dir_present = old_data_dir.exists();
        std::fs::rename(&old_dir, &new_dir).map_err(|e| {
            if data_dir_present {
                TomeError::WorkspaceDataDirWriteFailed {
                    path: new_data_dir,
                    source: std::io::Error::new(
                        e.kind(),
                        format!(
                            "workspace rename: relocate plugin-data {} -> {}: {} \
                             (database transaction is already committed; manual recovery required)",
                            old_data_dir.display(),
                            new_dir.join("plugin-data").display(),
                            e,
                        ),
                    ),
                }
            } else {
                TomeError::Io(std::io::Error::new(
                    e.kind(),
                    format!(
                        "workspace rename: rename central directory {} -> {}: {} \
                         (database transaction is already committed; manual recovery required)",
                        old_dir.display(),
                        new_dir.display(),
                        e,
                    ),
                ))
            }
        })?;
    } else {
        // Edge case: the row existed but the directory was missing.
        // Stand up an empty target dir so subsequent reads see a
        // self-consistent state. Chmod 0o700 on Unix to match the
        // init-path discipline (`atomic_dir::land_directory(_, 0o700,
        // _)`); without this the dir would land at the umask default.
        std::fs::create_dir_all(&new_dir).map_err(TomeError::Io)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&new_dir, std::fs::Permissions::from_mode(0o700))
                .map_err(TomeError::Io)?;
        }
    }

    drop(lock);

    Ok(RenameOutcome {
        old_name: old,
        new_name: new,
        bound_projects_updated,
        workspace_dir: new_dir,
    })
}

/// Read the project marker at `marker_path`, parse it via
/// `toml_edit::DocumentMut`, and replace the `workspace = "<old>"` key
/// with `workspace = "<new>"`. Returns the serialised body. All other
/// top-level keys (the optional `harnesses` field per data-model §7,
/// plus any comments / key order) survive the rewrite intact.
///
/// A parse failure surfaces as `WorkspaceMalformed` (exit 70).
fn rewrite_marker_workspace(marker_path: &Path, new_name: &str) -> Result<String, TomeError> {
    let body = crate::util::bounded_read_to_string(marker_path, crate::util::TOME_CONFIG_MAX)
        .map_err(|e| TomeError::WorkspaceMalformed {
            path: marker_path.to_path_buf(),
            reason: format!("read project marker for rename: {e}"),
        })?;
    let mut doc =
        toml_edit::DocumentMut::from_str(&body).map_err(|e| TomeError::WorkspaceMalformed {
            path: marker_path.to_path_buf(),
            reason: format!("parse project marker for rename: {e}"),
        })?;
    doc["workspace"] = toml_edit::value(new_name);
    Ok(doc.to_string())
}

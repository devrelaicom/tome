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
//! - **Transaction (step 5) fails**: rollback. Marker `config.toml`
//!   rewrites happen inside the transaction by construction: each
//!   rewrite either lands atomically before the next, or the entire
//!   sequence is rolled back. In practice the per-file rename of
//!   `.tome/config.toml.tmp.*` IS visible to filesystem readers between
//!   atomic writes — true atomicity across the file group is not
//!   achievable without a journaling layer. We do the bound-project
//!   rewrites BEFORE the SQL UPDATE so a mid-loop failure leaves the
//!   workspace's DB row still named `<old>` and *some* project markers
//!   pointing at `<new>`; `tome doctor` (US5) surfaces the drift, and a
//!   re-run of `rename` (now with the central DB still pointing at
//!   `<old>` but some projects pointing at `<new>`) will re-fail the
//!   pre-check or the per-file rewrite cleanly. The transaction's
//!   rollback gates only the DB UPDATE.
//! - **Step 6 fails** (e.g. cross-FS somehow): the DB transaction is
//!   already committed. Log a hard error; doctor `--fix` is **not**
//!   safe here (the central workspace directory is the canonical
//!   on-disk identity; a cross-FS situation needs manual recovery).

use std::path::PathBuf;

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
        // Trivial no-op rename; surface as a usage error so the caller
        // notices the typo rather than us silently doing nothing.
        return Err(TomeError::Usage(format!(
            "workspace rename: `<old>` and `<new>` are the same name (`{}`)",
            old.as_str(),
        )));
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
    let old_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![old.as_str()],
            |row| row.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => TomeError::WorkspaceNotFound {
                name: old.as_str().to_owned(),
            },
            other => TomeError::IndexIntegrityCheckFailure(format!(
                "lookup workspace `{}`: {other}",
                old.as_str()
            )),
        })?;

    // Refusal: `<new>` must not exist.
    let new_exists: Option<i64> = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![new.as_str()],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "lookup workspace `{}`: {e}",
                new.as_str()
            ))
        })?;
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

    // Bound-project marker rewrites first, then the SQL UPDATE inside
    // the same transaction. See module-level docs for the partial-
    // failure rationale: the marker rewrites are per-file atomic; if
    // the transaction rolls back, the central DB row stays at `<old>`
    // but some markers may already point at `<new>` (doctor surfaces).
    let bound_projects_updated = u32::try_from(bound_projects.len()).unwrap_or(u32::MAX);

    for project in &bound_projects {
        let marker_config = Paths::project_marker_config(project);
        let body = format!("workspace = \"{}\"\n", new.as_str());
        store::write_atomic(&marker_config, body.as_bytes())?;
    }

    let tx = conn.transaction().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("begin rename transaction: {e}"))
    })?;
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
    let old_dir = paths.workspace_dir(&old);
    let new_dir = paths.workspace_dir(&new);
    if old_dir.exists() {
        std::fs::rename(&old_dir, &new_dir).map_err(|e| {
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
        })?;
    } else {
        // Edge case: the row existed but the directory was missing.
        // Stand up an empty target dir so subsequent reads see a
        // self-consistent state.
        std::fs::create_dir_all(&new_dir).map_err(TomeError::Io)?;
    }

    drop(lock);

    Ok(RenameOutcome {
        old_name: old,
        new_name: new,
        bound_projects_updated,
        workspace_dir: new_dir,
    })
}

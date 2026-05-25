//! Per-workspace RULES.md sync to bound projects.
//!
//! Phase 4 / US2.a-2 introduces the **private** library helper
//! [`sync_workspace_rules_to_bound_projects`] which copies
//! `<root>/workspaces/<name>/RULES.md` to every `<project>/.tome/RULES.md`
//! whose `workspace_projects` row binds to `<name>`.
//!
//! The public CLI `tome workspace sync` lands in US2.c as a thin wrapper
//! over this helper. `regen-summary` (US2.a-2) uses the helper directly
//! after rewriting the central RULES.md so the per-project marker copies
//! stay in lockstep.
//!
//! ## Idempotence (FR-525)
//!
//! Per-project write only happens when the destination bytes differ
//! from the source. An unchanged destination is a no-op — the function
//! returns a count of files actually written, not visited.
//!
//! ## Missing project directories
//!
//! Per the contract Edge Cases: a bound project whose directory no
//! longer exists (or whose `.tome/` marker has been removed) is
//! `debug!`-logged and skipped. Not an error — `tome doctor` (US5)
//! surfaces such drift as a separate concern.
//!
//! ## Concurrency
//!
//! This helper does NOT take the advisory lock. It is a read-only
//! query against `workspace_projects` followed by per-file atomic
//! writes. Two parallel syncs converge on the same final state via
//! POSIX-atomic rename semantics; the only observable artefact is
//! double-rename traffic.

use std::path::Path;

use crate::catalog::store;
use crate::error::TomeError;
use crate::index::{self};
use crate::paths::Paths;
use crate::workspace::WorkspaceName;

/// Copy `<root>/workspaces/<name>/RULES.md` to every bound project's
/// `<project>/.tome/RULES.md`. Idempotent — destinations whose bytes
/// match the source are left untouched (no `rename()` syscall).
///
/// Returns the number of bound projects whose marker rules file was
/// written (created OR updated). Unchanged and missing-directory
/// projects don't count.
pub(crate) fn sync_workspace_rules_to_bound_projects(
    name: &WorkspaceName,
    paths: &Paths,
) -> Result<u32, TomeError> {
    let source = paths.workspace_rules_file(name);
    let source_bytes = match std::fs::read(&source) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No central RULES.md to propagate. Caller (regen-summary)
            // writes it BEFORE calling here; if it's still absent the
            // sync is a no-op.
            tracing::debug!(
                workspace = name.as_str(),
                path = %source.display(),
                "workspace sync: central RULES.md missing; nothing to copy",
            );
            return Ok(0);
        }
        Err(e) => return Err(TomeError::Io(e)),
    };

    if !paths.index_db.is_file() {
        // No central DB → no bindings to walk.
        return Ok(0);
    }

    let conn = index::open_read_only(&paths.index_db)?;

    // Membership check: a missing workspace row → nothing to sync.
    let workspace_id: Option<i64> = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![name.as_str()],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "workspace sync: lookup workspace `{}`: {e}",
                name.as_str(),
            ))
        })?;
    let Some(workspace_id) = workspace_id else {
        return Ok(0);
    };

    let mut stmt = conn
        .prepare(
            "SELECT project_path FROM workspace_projects
             WHERE workspace_id = ?1
             ORDER BY project_path",
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("workspace sync: prepare projects: {e}"))
        })?;
    let project_iter = stmt
        .query_map(rusqlite::params![workspace_id], |row| {
            let p: String = row.get(0)?;
            Ok(p)
        })
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("workspace sync: query projects: {e}"))
        })?;

    let mut synced: u32 = 0;
    for row in project_iter {
        let project_path = row.map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("workspace sync: read project row: {e}"))
        })?;
        let project_root = Path::new(&project_path);

        if !project_root.is_dir() {
            tracing::debug!(
                workspace = name.as_str(),
                project = %project_root.display(),
                "workspace sync: project directory missing; skipping",
            );
            continue;
        }
        let marker_dir = Paths::project_marker_dir(project_root);
        if !marker_dir.is_dir() {
            tracing::debug!(
                workspace = name.as_str(),
                project = %project_root.display(),
                marker = %marker_dir.display(),
                "workspace sync: project marker `.tome/` missing; skipping",
            );
            continue;
        }
        let dest = Paths::project_marker_rules(project_root);

        if let Ok(existing) = std::fs::read(&dest)
            && existing == source_bytes
        {
            // Idempotence: bytes already match. No write.
            continue;
        }

        store::write_atomic(&dest, &source_bytes)?;
        synced = synced.saturating_add(1);
    }

    Ok(synced)
}

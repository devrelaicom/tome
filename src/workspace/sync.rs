//! Per-workspace RULES.md sync to bound projects.
//!
//! Phase 4 / US2.a-2 introduced the original private helper
//! (`sync_workspace_rules_to_bound_projects`) which copies
//! `<root>/workspaces/<name>/RULES.md` to every `<project>/.tome/RULES.md`
//! whose `workspace_projects` row binds to `<name>`.
//!
//! Phase 4 / US2.c promotes the helper to a [`sync_one`] entry point
//! that returns a richer [`WorkspaceSyncOutcome`] (synced + unchanged +
//! missing-project lists) so the new `tome workspace sync [<name>]`
//! CLI surface can render per-project detail in `--json` mode.
//!
//! `regen-summary` (US2.a-2) continues to call the legacy
//! [`sync_workspace_rules_to_bound_projects`] convenience wrapper, which
//! delegates to `sync_one` and discards the rich outcome — it only
//! cares about the count of synced projects.
//!
//! ## Idempotence (FR-525)
//!
//! Per-project write only happens when the destination bytes differ
//! from the source. An unchanged destination is a no-op — the project
//! lands in [`WorkspaceSyncOutcome::unchanged`], not
//! [`WorkspaceSyncOutcome::synced_projects`]. No `rename()` syscall is
//! issued.
//!
//! ## Missing project directories
//!
//! Per the contract Edge Cases: a bound project whose directory no
//! longer exists (or whose `.tome/` marker has been removed) is
//! `debug!`-logged and surfaces in [`WorkspaceSyncOutcome::missing_project_dirs`].
//! Not an error — `tome doctor` (US5) surfaces such drift as a separate
//! concern.
//!
//! ## Concurrency
//!
//! This helper does NOT take the advisory lock. It is a read-only
//! query against `workspace_projects` followed by per-file atomic
//! writes. Two parallel syncs converge on the same final state via
//! POSIX-atomic rename semantics; the only observable artefact is
//! double-rename traffic.

use std::path::PathBuf;

use serde::Serialize;

use crate::catalog::store;
use crate::error::TomeError;
use crate::index::{self};
use crate::paths::Paths;
use crate::workspace::WorkspaceName;

/// Per-workspace sync outcome. Field order pinned by the contract;
/// extra fields go on the surrounding `WorkspaceSyncEntry` envelope.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct WorkspaceSyncOutcome {
    /// Project roots whose `<project>/.tome/RULES.md` was (re)written.
    pub synced_projects: Vec<PathBuf>,
    /// Project roots whose existing `<project>/.tome/RULES.md` bytes
    /// already matched the source — no write performed.
    pub unchanged: Vec<PathBuf>,
    /// Project roots whose directory (or `.tome/` marker) is missing
    /// on disk; skipped.
    pub missing_project_dirs: Vec<PathBuf>,
}

/// Copy `<root>/workspaces/<name>/RULES.md` to every bound project's
/// `<project>/.tome/RULES.md`. Returns the partitioned outcome
/// (synced / unchanged / missing).
///
/// Idempotent: destinations whose bytes match the source are listed
/// in `unchanged`; no `rename()` is issued. If the central
/// RULES.md is absent, every category is empty (regen-summary writes
/// it before calling here).
///
/// Missing workspace rows return an empty outcome — callers wanting
/// "does this workspace exist?" semantics should query the registry
/// first.
pub fn sync_one(name: &WorkspaceName, paths: &Paths) -> Result<WorkspaceSyncOutcome, TomeError> {
    let mut outcome = WorkspaceSyncOutcome::default();

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
            return Ok(outcome);
        }
        Err(e) => return Err(TomeError::Io(e)),
    };

    if !paths.index_db.is_file() {
        // No central DB → no bindings to walk.
        return Ok(outcome);
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
        return Ok(outcome);
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

    for row in project_iter {
        let project_path = row.map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("workspace sync: read project row: {e}"))
        })?;
        let project_root = PathBuf::from(&project_path);

        if !project_root.is_dir() {
            tracing::debug!(
                workspace = name.as_str(),
                project = %project_root.display(),
                "workspace sync: project directory missing; skipping",
            );
            outcome.missing_project_dirs.push(project_root);
            continue;
        }
        let marker_dir = Paths::project_marker_dir(&project_root);
        if !marker_dir.is_dir() {
            tracing::debug!(
                workspace = name.as_str(),
                project = %project_root.display(),
                marker = %marker_dir.display(),
                "workspace sync: project marker `.tome/` missing; skipping",
            );
            outcome.missing_project_dirs.push(project_root);
            continue;
        }
        let dest = Paths::project_marker_rules(&project_root);

        if let Ok(existing) = std::fs::read(&dest)
            && existing == source_bytes
        {
            // Idempotence: bytes already match. No write.
            outcome.unchanged.push(project_root);
            continue;
        }

        store::write_atomic(&dest, &source_bytes)?;
        outcome.synced_projects.push(project_root);
    }

    Ok(outcome)
}

/// List every workspace name in the central registry, alphabetically.
/// Used by `tome workspace sync` with no name arg to iterate every
/// workspace.
///
/// Returns `["global"]` synthesised in the pre-bootstrap case (DB
/// file absent) so the sync command never errors on a fresh install
/// — the privileged `global` workspace is the conceptual default.
pub fn list_workspace_names(paths: &Paths) -> Result<Vec<WorkspaceName>, TomeError> {
    if !paths.index_db.is_file() {
        return Ok(vec![WorkspaceName::global()]);
    }
    let conn = index::open_read_only(&paths.index_db)?;
    let mut stmt = conn
        .prepare("SELECT name FROM workspaces ORDER BY name")
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("workspace sync: list names: {e}"))
        })?;
    let rows = stmt
        .query_map([], |row| {
            let n: String = row.get(0)?;
            Ok(n)
        })
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("workspace sync: list rows: {e}"))
        })?;
    let mut out = Vec::new();
    for r in rows {
        let raw = r.map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("workspace sync: read name: {e}"))
        })?;
        // Names in the table were validated on insert; if a row ever
        // fails to parse it's a stronger signal than "sync errored".
        let parsed = WorkspaceName::parse(&raw)?;
        out.push(parsed);
    }
    Ok(out)
}

/// Legacy convenience wrapper kept for `regen-summary`. Returns just
/// the count of projects whose marker rules file was written.
///
/// New callers should prefer [`sync_one`] for the partitioned
/// outcome.
pub fn sync_workspace_rules_to_bound_projects(
    name: &WorkspaceName,
    paths: &Paths,
) -> Result<u32, TomeError> {
    let outcome = sync_one(name, paths)?;
    Ok(u32::try_from(outcome.synced_projects.len()).unwrap_or(u32::MAX))
}

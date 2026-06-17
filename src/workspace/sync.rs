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

/// Outcome of syncing the workspace RULES.md to ONE bound project.
///
/// Maps 1:1 onto [`WorkspaceSyncOutcome`]'s three partitions so the
/// `sync_one` loop can classify a project by a single match. The write
/// path can still fail with [`TomeError`] (propagated, not collapsed
/// into `MissingProjectDir`) — only the *skip* cases (project dir or
/// `.tome/` marker absent) are non-errors that land here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RulesSync {
    /// Destination was (re)written — bytes differed or were absent.
    Synced,
    /// Destination already matched the source bytes; no write issued.
    Unchanged,
    /// Project directory or its `.tome/` marker is missing; skipped.
    MissingProjectDir,
}

/// Write the workspace's RULES.md (`source_bytes`) to ONE bound
/// project's `<project>/.tome/RULES.md`.
///
/// Byte-for-byte idempotent: returns [`RulesSync::Unchanged`] without a
/// write when the destination already matches `source_bytes`.
/// [`RulesSync::MissingProjectDir`] when the project directory or its
/// `.tome/` marker is absent (each `debug!`-logged with `name` for
/// `tome doctor` drift triage — not an error per the contract Edge
/// Cases). A genuine write failure is an error and is *propagated*, not
/// classified as missing.
///
/// Extracted from [`sync_one`]'s per-project loop so the project-scoped
/// `tome sync` command can reuse the identical classification +
/// atomic-write path.
pub fn sync_rules_to_project(
    source_bytes: &[u8],
    project_root: &std::path::Path,
    name: &WorkspaceName,
) -> Result<RulesSync, TomeError> {
    if !project_root.is_dir() {
        tracing::debug!(
            workspace = name.as_str(),
            project = %project_root.display(),
            "workspace sync: project directory missing; skipping",
        );
        return Ok(RulesSync::MissingProjectDir);
    }
    let marker_dir = Paths::project_marker_dir(project_root);
    if !marker_dir.is_dir() {
        tracing::debug!(
            workspace = name.as_str(),
            project = %project_root.display(),
            marker = %marker_dir.display(),
            "workspace sync: project marker `.tome/` missing; skipping",
        );
        return Ok(RulesSync::MissingProjectDir);
    }
    let dest = Paths::project_marker_rules(project_root);

    if let Ok(existing) = std::fs::read(&dest)
        && existing == source_bytes
    {
        // Idempotence: bytes already match. No write.
        return Ok(RulesSync::Unchanged);
    }

    store::write_atomic(&dest, source_bytes)?;
    Ok(RulesSync::Synced)
}

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
    // Polish R-M7: route through the consolidated helper.
    let Some(workspace_id) = crate::index::workspaces::resolve_id_optional(&conn, name)? else {
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

        match sync_rules_to_project(&source_bytes, &project_root, name)? {
            RulesSync::Synced => outcome.synced_projects.push(project_root),
            RulesSync::Unchanged => outcome.unchanged.push(project_root),
            RulesSync::MissingProjectDir => outcome.missing_project_dirs.push(project_root),
        }
    }

    Ok(outcome)
}

/// Copy `<root>/workspaces/<name>/RULES.md` to ONE project's
/// `<project>/.tome/RULES.md`. Project-local equivalent of [`sync_one`]
/// — does NOT walk every bound project of the workspace.
///
/// Used by `doctor::fixes::repair_binding_rules_copy` per US5 reviewer
/// C-M3: re-copying ONE project's drifted/missing RULES.md must not
/// silently broadcast to every other bound project of the same
/// workspace.
///
/// Returns `Ok(true)` when a write occurred (drift or missing copy),
/// `Ok(false)` when the destination already matched the source (idempotent
/// no-op). The source-missing case returns
/// [`TomeError::WorkspaceMalformed`] so the doctor pass surfaces the
/// underlying source-of-truth absence rather than papering over it; the
/// `RulesCopyState::SourceMissing` suggested fix is the user-facing hint.
pub fn sync_one_project(
    name: &WorkspaceName,
    paths: &Paths,
    project_root: &std::path::Path,
) -> Result<bool, TomeError> {
    let source = paths.workspace_rules_file(name);
    let source_bytes = match std::fs::read(&source) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(TomeError::WorkspaceMalformed {
                path: source.clone(),
                reason: format!("workspace `{}`: source RULES.md absent", name.as_str(),),
            });
        }
        Err(e) => return Err(TomeError::Io(e)),
    };

    let marker_dir = Paths::project_marker_dir(project_root);
    if !marker_dir.is_dir() {
        // Caller (doctor) shouldn't have flagged this project for repair
        // if its marker dir is gone; surface as an Io NotFound so the
        // residual suggested-fix list reflects the real state.
        return Err(TomeError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("project marker `.tome/` absent at {}", marker_dir.display()),
        )));
    }
    let dest = Paths::project_marker_rules(project_root);
    if let Ok(existing) = std::fs::read(&dest)
        && existing == source_bytes
    {
        // Idempotence — caller can treat this as "nothing to do".
        return Ok(false);
    }
    crate::catalog::store::write_atomic(&dest, &source_bytes)?;
    Ok(true)
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

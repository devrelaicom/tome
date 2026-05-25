//! `tome workspace sync [<name>]` CLI wrapper.
//!
//! Phase 4 / US2.c. Contract reference:
//! [`contracts/workspace-commands.md` § `tome workspace sync`].
//!
//! The compute path lives in [`crate::workspace::sync`]; this module
//! resolves the target workspace(s), invokes
//! [`crate::workspace::sync::sync_one`] for each, aggregates the
//! per-workspace outcomes into a [`WorkspaceSyncReport`], and emits.
//!
//! ## Algorithm
//!
//! 1. Parse `args.name` (if present) via [`WorkspaceName::parse`].
//!    Bad name → exit 15 (`WorkspaceNameInvalid`).
//! 2. Resolve target workspace list:
//!    - `Some(name)` → membership check via the central DB; missing →
//!      exit 13 (`WorkspaceNotFound`).
//!    - `None` → every row in `workspaces` (or `["global"]` if the DB
//!      doesn't exist yet).
//! 3. For each target, call [`crate::workspace::sync::sync_one`].
//!    Collect outcomes.
//! 4. Aggregate totals across every workspace.
//! 5. Emit human (per-workspace summary lines) or JSON (the full
//!    [`WorkspaceSyncReport`]).
//!
//! ## Idempotence
//!
//! Per [`crate::workspace::sync`] module docs: destinations whose
//! bytes already match the source land in `unchanged` and incur no
//! `rename()` syscall.
//!
//! ## Concurrency
//!
//! Does not take the advisory lock. Reads `workspaces` +
//! `workspace_projects` read-only; per-file writes via the same
//! POSIX-atomic rename helper as every other Tome write.

use std::io::Write;

use serde::Serialize;

use crate::cli::WorkspaceSyncArgs;
use crate::error::TomeError;
use crate::index;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::{self, WorkspaceName, WorkspaceSyncOutcome};

/// One row of the aggregate report — one workspace's outcome.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorkspaceSyncEntry {
    pub workspace: WorkspaceName,
    pub outcome: WorkspaceSyncOutcome,
}

/// `--json` envelope. Carries per-workspace detail plus rolled-up
/// totals so callers can render a single-line summary without
/// re-aggregating.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorkspaceSyncReport {
    pub per_workspace: Vec<WorkspaceSyncEntry>,
    pub total_synced: u32,
    pub total_unchanged: u32,
    pub total_missing: u32,
}

pub fn run(args: WorkspaceSyncArgs, paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let report = assemble(args, paths)?;
    emit(&report, mode)
}

/// Pure-compute entry point. Tests target this directly without the
/// stdout emit.
pub fn assemble(args: WorkspaceSyncArgs, paths: &Paths) -> Result<WorkspaceSyncReport, TomeError> {
    let targets = resolve_targets(args.name.as_deref(), paths)?;

    let mut per_workspace = Vec::with_capacity(targets.len());
    let mut total_synced: u32 = 0;
    let mut total_unchanged: u32 = 0;
    let mut total_missing: u32 = 0;
    for name in targets {
        let outcome = workspace::sync_one(&name, paths)?;
        total_synced = total_synced
            .saturating_add(u32::try_from(outcome.synced_projects.len()).unwrap_or(u32::MAX));
        total_unchanged = total_unchanged
            .saturating_add(u32::try_from(outcome.unchanged.len()).unwrap_or(u32::MAX));
        total_missing = total_missing
            .saturating_add(u32::try_from(outcome.missing_project_dirs.len()).unwrap_or(u32::MAX));
        per_workspace.push(WorkspaceSyncEntry {
            workspace: name,
            outcome,
        });
    }

    Ok(WorkspaceSyncReport {
        per_workspace,
        total_synced,
        total_unchanged,
        total_missing,
    })
}

/// Resolve the target workspace list:
/// - `Some(raw)`: validate via `WorkspaceName::parse` (exit 15 on
///   failure), confirm membership in the central DB (exit 13 on
///   missing), return single-element list.
/// - `None`: enumerate every workspace name from
///   [`workspace::list_workspace_names`].
fn resolve_targets(
    requested: Option<&str>,
    paths: &Paths,
) -> Result<Vec<WorkspaceName>, TomeError> {
    match requested {
        Some(raw) => {
            let name = WorkspaceName::parse(raw)?;
            if !workspace_exists(&name, paths)? {
                return Err(TomeError::WorkspaceNotFound {
                    name: name.as_str().to_owned(),
                });
            }
            Ok(vec![name])
        }
        None => workspace::list_workspace_names(paths),
    }
}

/// Membership check against the central registry. The privileged
/// `global` workspace is always considered present (it's seeded on
/// first bootstrap; calling `workspace sync global` on a fresh
/// install should not error).
fn workspace_exists(name: &WorkspaceName, paths: &Paths) -> Result<bool, TomeError> {
    if name.is_reserved() {
        return Ok(true);
    }
    if !paths.index_db.is_file() {
        return Ok(false);
    }
    let conn = index::open_read_only(&paths.index_db)?;
    let row: Option<i64> = conn
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
                "workspace sync: membership check for `{}`: {e}",
                name.as_str(),
            ))
        })?;
    Ok(row.is_some())
}

fn emit(report: &WorkspaceSyncReport, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(report),
        Mode::Json => write_json(report),
    }
}

fn emit_human(report: &WorkspaceSyncReport) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    if report.per_workspace.is_empty() {
        writeln!(out, "No workspaces to sync.")?;
        return Ok(());
    }
    for entry in &report.per_workspace {
        let o = &entry.outcome;
        writeln!(
            out,
            "Workspace `{}`: {} synced, {} unchanged, {} missing",
            entry.workspace.as_str(),
            o.synced_projects.len(),
            o.unchanged.len(),
            o.missing_project_dirs.len(),
        )?;
    }
    writeln!(
        out,
        "Total: {} synced, {} unchanged, {} missing across {} workspace(s)",
        report.total_synced,
        report.total_unchanged,
        report.total_missing,
        report.per_workspace.len(),
    )?;
    Ok(())
}

//! `tome workspace list` — every workspace in the central registry plus
//! one-row-per-workspace counts (catalogs, enabled plugins, indexed
//! skills, bound projects, last_used_at).
//!
//! Contract: `contracts/workspace-commands.md` § `tome workspace list`.
//!
//! Read-only: opens the central index via `open_read_only` (no advisory
//! lock taken). On a fresh install with no DB file yet, returns a
//! single conceptual entry for `global` with all counts zero — the
//! privileged workspace is seeded on first bootstrap.

use std::io::Write;

use comfy_table::{Cell, CellAlignment};
use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::cli::WorkspaceListArgs;
use crate::error::TomeError;
use crate::index;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::presentation::tables;

/// One wire-shape row, serialised by `--json`. Field order pinned by the
/// contract.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorkspaceListEntry {
    pub name: String,
    pub catalogs: u32,
    pub enabled_plugins: u32,
    pub indexed_skills: u32,
    pub bound_projects: u32,
    /// Unix-second timestamp (seconds since epoch). Zero when the row is
    /// the synthetic "bootstrap-not-yet" `global` entry.
    pub last_used_at: i64,
}

/// `--json` envelope. Distinct from the bare array because future
/// optional fields (filters, summary totals) belong on the envelope.
#[derive(Debug, Clone, Serialize)]
struct ListEnvelope<'a> {
    workspaces: &'a [WorkspaceListEntry],
}

pub fn run(_args: WorkspaceListArgs, paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let entries = assemble(paths)?;
    emit(&entries, mode)
}

/// Pure-compute entry point. Tests target this directly without the
/// stdout emit.
pub fn assemble(paths: &Paths) -> Result<Vec<WorkspaceListEntry>, TomeError> {
    if !paths.index_db.is_file() {
        // Pre-bootstrap: the privileged `global` workspace is the
        // conceptual default; emit one row to keep the table coherent.
        return Ok(vec![WorkspaceListEntry {
            name: "global".to_owned(),
            catalogs: 0,
            enabled_plugins: 0,
            indexed_skills: 0,
            bound_projects: 0,
            last_used_at: 0,
        }]);
    }

    let conn = index::open_read_only(&paths.index_db)?;

    let mut stmt = conn
        .prepare("SELECT id, name, last_used_at FROM workspaces ORDER BY name")
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("prepare workspaces: {e}")))?;
    let rows = stmt
        .query_map([], |row| {
            let id: i64 = row.get(0)?;
            let name: String = row.get(1)?;
            let last_used_at: i64 = row.get(2)?;
            Ok((id, name, last_used_at))
        })
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("query workspaces: {e}")))?;

    let mut out = Vec::new();
    for r in rows {
        let (id, name, last_used_at) = r.map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("read workspace row: {e}"))
        })?;
        let catalogs = count_i64(
            &conn,
            "SELECT COUNT(*) FROM workspace_catalogs WHERE workspace_id = ?1",
            id,
            "count workspace_catalogs",
        )?;
        let enabled_plugins = count_i64(
            &conn,
            "SELECT COUNT(DISTINCT s.catalog || '/' || s.plugin)
             FROM workspace_skills AS ws
             JOIN skills AS s ON s.id = ws.skill_id
             WHERE ws.workspace_id = ?1",
            id,
            "count enabled plugins",
        )?;
        let indexed_skills = count_i64(
            &conn,
            "SELECT COUNT(*) FROM workspace_skills WHERE workspace_id = ?1",
            id,
            "count workspace_skills",
        )?;
        let bound_projects = count_i64(
            &conn,
            "SELECT COUNT(*) FROM workspace_projects WHERE workspace_id = ?1",
            id,
            "count workspace_projects",
        )?;
        out.push(WorkspaceListEntry {
            name,
            catalogs,
            enabled_plugins,
            indexed_skills,
            bound_projects,
            last_used_at,
        });
    }
    Ok(out)
}

fn count_i64(
    conn: &rusqlite::Connection,
    sql: &str,
    workspace_id: i64,
    context: &str,
) -> Result<u32, TomeError> {
    let n: i64 = conn
        .query_row(sql, rusqlite::params![workspace_id], |row| row.get(0))
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("{context}: {e}")))?;
    Ok(u32::try_from(n).unwrap_or(u32::MAX))
}

fn emit(entries: &[WorkspaceListEntry], mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(entries),
        Mode::Json => write_json(&ListEnvelope {
            workspaces: entries,
        }),
    }
}

fn emit_human(entries: &[WorkspaceListEntry]) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    if entries.is_empty() {
        writeln!(out, "No workspaces.")?;
        return Ok(());
    }
    let mut table = tables::new_table();
    table.set_header(vec![
        Cell::new("Name"),
        Cell::new("Catalogs").set_alignment(CellAlignment::Right),
        Cell::new("Plugins").set_alignment(CellAlignment::Right),
        Cell::new("Skills").set_alignment(CellAlignment::Right),
        Cell::new("Bound projects").set_alignment(CellAlignment::Right),
        Cell::new("Last used"),
    ]);
    for e in entries {
        table.add_row(vec![
            Cell::new(&e.name),
            Cell::new(e.catalogs).set_alignment(CellAlignment::Right),
            Cell::new(e.enabled_plugins).set_alignment(CellAlignment::Right),
            Cell::new(e.indexed_skills).set_alignment(CellAlignment::Right),
            Cell::new(e.bound_projects).set_alignment(CellAlignment::Right),
            Cell::new(human_last_used(e.last_used_at)),
        ]);
    }
    writeln!(out, "{table}")?;
    Ok(())
}

fn human_last_used(unix_secs: i64) -> String {
    if unix_secs == 0 {
        return "—".to_owned();
    }
    let Ok(dt) = OffsetDateTime::from_unix_timestamp(unix_secs) else {
        return "—".to_owned();
    };
    dt.format(&Rfc3339).unwrap_or_else(|_| "—".to_owned())
}

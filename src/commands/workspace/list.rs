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
//!
//! Issue #300: the row whose name matches the scope resolved for the
//! current directory is flagged `current: true`. Resolution is NOT
//! re-implemented here — the caller has already run the one resolution
//! SSOT ([`crate::workspace::resolution::resolve`]) and hands us the
//! resulting [`ResolvedScope`]. The resolved name is authoritative
//! regardless of *how* it was chosen (a `--workspace` flag,
//! `TOME_WORKSPACE`, a `[workspace] default`, a project marker, or the
//! `global` fallback); we simply mark the row that carries that name.

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
use crate::workspace::ResolvedScope;

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
    /// the synthetic "bootstrap-not-yet" `global` entry. Always the
    /// absolute value in `--json`; the human `Last used` column renders it
    /// relative (or absolute with `--absolute`).
    pub last_used_at: i64,
    /// Issue #300: `true` for the workspace resolved for the current
    /// directory (the active scope), `false` for every other row.
    /// Appended LAST so the byte-stable `--json` wire shape only grows a
    /// trailing field.
    pub current: bool,
}

pub fn run(
    args: WorkspaceListArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    mode: Mode,
) -> Result<(), TomeError> {
    let entries = assemble(scope, paths)?;
    emit(&entries, args.absolute, mode)
}

/// Pure-compute entry point. Tests target this directly without the
/// stdout emit. `scope` carries the workspace resolved for the current
/// directory; the row whose name matches is flagged `current: true`.
pub fn assemble(
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<Vec<WorkspaceListEntry>, TomeError> {
    let active = scope.scope.name().as_str();
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
            current: active == "global",
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
        let current = name == active;
        out.push(WorkspaceListEntry {
            name,
            catalogs,
            enabled_plugins,
            indexed_skills,
            bound_projects,
            last_used_at,
            current,
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

fn emit(entries: &[WorkspaceListEntry], absolute: bool, mode: Mode) -> Result<(), TomeError> {
    match mode {
        Mode::Human => emit_human(entries, absolute),
        // Contract: `workspace list --json` emits a bare array. Future
        // optional fields belong on individual entries, not on a wrapping
        // envelope (cf. `contracts/workspace-commands.md` §`workspace list`).
        // `--json` is machine-readable: `last_used_at` stays the absolute
        // unix-second value (the `--absolute` flag is a human-only knob) and
        // the per-row `current` bool tells scripts which workspace is active.
        Mode::Json => write_json(entries),
    }
}

fn emit_human(entries: &[WorkspaceListEntry], absolute: bool) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    if entries.is_empty() {
        writeln!(out, "No workspaces.")?;
        return Ok(());
    }
    let mut table = tables::new_table();
    table.set_header(vec![
        Cell::new("Cur").set_alignment(CellAlignment::Center),
        Cell::new("Name"),
        Cell::new("Catalogs").set_alignment(CellAlignment::Right),
        Cell::new("Plugins").set_alignment(CellAlignment::Right),
        Cell::new("Skills").set_alignment(CellAlignment::Right),
        Cell::new("Bound projects").set_alignment(CellAlignment::Right),
        Cell::new("Last used"),
    ]);
    let now = OffsetDateTime::now_utc().unix_timestamp();
    for e in entries {
        table.add_row(vec![
            Cell::new(if e.current { "*" } else { "" }).set_alignment(CellAlignment::Center),
            Cell::new(&e.name),
            Cell::new(e.catalogs).set_alignment(CellAlignment::Right),
            Cell::new(e.enabled_plugins).set_alignment(CellAlignment::Right),
            Cell::new(e.indexed_skills).set_alignment(CellAlignment::Right),
            Cell::new(e.bound_projects).set_alignment(CellAlignment::Right),
            Cell::new(human_last_used(e.last_used_at, absolute, now)),
        ]);
    }
    writeln!(out, "{table}")?;
    Ok(())
}

/// Render `last_used_at` for the human table. Zero (the synthetic
/// bootstrap-not-yet row / never-used) always shows an em dash. Otherwise
/// the default is a relative time (`crate::util::relative_time`); with
/// `absolute`, the RFC 3339 timestamp. A timestamp we can't parse/format
/// degrades to the em dash rather than a panic.
fn human_last_used(unix_secs: i64, absolute: bool, now: i64) -> String {
    if unix_secs == 0 {
        return "—".to_owned();
    }
    if !absolute {
        return crate::util::relative_time(unix_secs, now);
    }
    let Ok(dt) = OffsetDateTime::from_unix_timestamp(unix_secs) else {
        return "—".to_owned();
    };
    dt.format(&Rfc3339).unwrap_or_else(|_| "—".to_owned())
}

#[cfg(test)]
mod tests {
    use super::human_last_used;

    /// Zero always renders the em dash regardless of the `absolute` flag —
    /// it's the synthetic "never used / bootstrap-not-yet" sentinel.
    #[test]
    fn zero_is_em_dash_in_both_modes() {
        let now = 1_700_000_000;
        assert_eq!(human_last_used(0, false, now), "—");
        assert_eq!(human_last_used(0, true, now), "—");
    }

    /// The default (non-absolute) rendering is the relative form.
    #[test]
    fn default_is_relative() {
        let now = 1_700_000_000;
        assert_eq!(human_last_used(now, false, now), "just now");
        assert_eq!(human_last_used(now - 2 * 86400, false, now), "2 days ago");
        assert_eq!(human_last_used(now - 3600, false, now), "1 hour ago");
    }

    /// `--absolute` forces the RFC 3339 timestamp (never the relative form).
    #[test]
    fn absolute_flag_forces_rfc3339() {
        let now = 1_700_000_000;
        // 1_700_000_000 == 2023-11-14T22:13:20Z.
        assert_eq!(
            human_last_used(1_700_000_000, true, now),
            "2023-11-14T22:13:20Z"
        );
        // Even a very recent timestamp renders absolute, not "just now".
        let out = human_last_used(now, true, now);
        assert!(
            out.starts_with("2023-11-14T") && out.ends_with('Z'),
            "absolute must be RFC 3339, got {out}",
        );
    }
}

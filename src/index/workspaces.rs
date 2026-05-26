//! Central-DB workspace-id lookup helpers.
//!
//! The query `SELECT id FROM workspaces WHERE name = ?1` is used at 10
//! sites across the codebase, each with a slightly different mapping
//! of `QueryReturnedNoRows` and other SQL errors. This module
//! consolidates the lookup into two helpers per the two semantic
//! categories:
//!
//! - [`resolve_id_required`] — the caller already knows the workspace
//!   should exist. `NoRows` maps to [`TomeError::WorkspaceNotFound`]
//!   (exit 13).
//! - [`resolve_id_optional`] — the caller is doing existence-checking
//!   and treats absence as a valid state. `NoRows` maps to `Ok(None)`.
//!
//! Both map other SQL errors to [`TomeError::IndexIntegrityCheckFailure`]
//! with a uniform message shape:
//! `"workspace id lookup for `<name>`: {e}"`.
//!
//! Polish R-M7.

use rusqlite::{Connection, OptionalExtension};

use crate::error::TomeError;
use crate::workspace::WorkspaceName;

/// Look up the central `workspaces.id` for `name`, treating absence as
/// failure.
///
/// `NoRows` → [`TomeError::WorkspaceNotFound`] (exit 13). Other SQL
/// errors → [`TomeError::IndexIntegrityCheckFailure`] (exit 51).
pub fn resolve_id_required(conn: &Connection, name: &WorkspaceName) -> Result<i64, TomeError> {
    match resolve_id_optional(conn, name)? {
        Some(id) => Ok(id),
        None => Err(TomeError::WorkspaceNotFound {
            name: name.as_str().to_owned(),
        }),
    }
}

/// Look up the central `workspaces.id` for `name`, treating absence as
/// `Ok(None)`.
///
/// Use this when the caller distinguishes "exists" from "doesn't yet
/// exist" — e.g. `workspace init` checking before insert, or
/// `workspace info` reporting "unknown workspace".
pub fn resolve_id_optional(
    conn: &Connection,
    name: &WorkspaceName,
) -> Result<Option<i64>, TomeError> {
    conn.query_row(
        "SELECT id FROM workspaces WHERE name = ?1",
        rusqlite::params![name.as_str()],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!(
            "workspace id lookup for `{}`: {e}",
            name.as_str(),
        ))
    })
}

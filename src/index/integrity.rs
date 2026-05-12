//! `PRAGMA integrity_check` wrapper for `tome status`.
//!
//! SQLite returns a single-row `'ok'` result when the database is clean,
//! otherwise a multi-row report. We map both shapes to a single
//! [`TomeError::IndexIntegrityCheckFailure`] (exit 51) so the caller does not
//! have to inspect SQLite's diagnostic strings.

use rusqlite::Connection;

use crate::error::TomeError;

/// Run `PRAGMA integrity_check`. Returns `Ok(())` when SQLite reports the
/// database is clean. Otherwise returns the joined diagnostic message
/// inside [`TomeError::IndexIntegrityCheckFailure`].
pub fn check(conn: &Connection) -> Result<(), TomeError> {
    let mut stmt = conn
        .prepare("PRAGMA integrity_check")
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("prepare: {e}")))?;
    let rows: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("query: {e}")))?
        .collect::<Result<_, _>>()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("collect: {e}")))?;

    match rows.as_slice() {
        [only] if only == "ok" => Ok(()),
        _ => Err(TomeError::IndexIntegrityCheckFailure(rows.join("; "))),
    }
}

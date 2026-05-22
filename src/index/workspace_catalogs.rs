//! Junction-table CRUD for catalog enrolment per workspace.
//!
//! Phase 4 / F11b moves catalog enrolment off `config.toml` and onto the
//! `workspace_catalogs` table (`(workspace_id, catalog_name) → (url,
//! pinned_ref)`). The on-disk catalog clone — `last_synced` mtime, the
//! catalogs cache dir, the parsed `tome-catalog.toml` — is derived from
//! the filesystem at read time, not stored.
//!
//! Per-URL metadata that Phase 1's `config.toml` stored:
//!
//! * `last_synced` — `std::fs::metadata(clone_path)?.modified()` against
//!   the cache dir. `git clone` / `git pull` touch the directory, so
//!   mtime advances naturally; absent clone → `None`.
//! * `path` — `Paths::cache_dir_for(&url)` (URL-hashed dir under
//!   `<root>/catalogs/`).
//! * `plugin_count` — read from `<clone>/tome-catalog.toml` lazily.
//! * `name` — stored per `(workspace, catalog)` in `catalog_name`.
//!
//! The advisory lock is **NOT** taken inside these helpers — every
//! caller that runs as part of a multi-step write wraps the relevant
//! critical section. Same discipline as [`crate::index::skills`].
//!
//! Spec: [data-model.md §4 (`workspace_catalogs`)](../../../specs/004-phase-4-refactor-harnesses/data-model.md)
//! and FR-360 / FR-361 / FR-362 / FR-363 / FR-364 / FR-365 / FR-366 /
//! FR-367.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::TomeError;
use crate::paths::Paths;

/// One enrolment row, as returned by reads. `workspace_name` is the
/// resolved scope; the rest mirrors the `workspace_catalogs` columns.
#[derive(Debug, Clone)]
pub struct CatalogEnrolment {
    pub workspace_name: String,
    pub catalog_name: String,
    pub url: String,
    pub pinned_ref: String,
}

/// INSERT a new enrolment for `(workspace_name, catalog_name)`. The PK
/// `(workspace_id, catalog_name)` enforces per-workspace uniqueness —
/// duplicates surface as [`TomeError::CatalogAlreadyExists`].
///
/// Callers participating in a multi-step write (e.g. `catalog add`'s
/// clone-or-reuse + INSERT) MUST hold the advisory lock; this helper
/// only owns the SQL.
pub fn insert(
    conn: &Connection,
    workspace_name: &str,
    catalog_name: &str,
    url: &str,
    pinned_ref: &str,
) -> Result<(), TomeError> {
    let workspace_id = lookup_workspace_id(conn, workspace_name)?;
    let affected = conn
        .execute(
            "INSERT INTO workspace_catalogs (workspace_id, catalog_name, url, pinned_ref)
             VALUES (?1, ?2, ?3, ?4)",
            params![workspace_id, catalog_name, url, pinned_ref],
        )
        .map_err(|e| match e {
            // SQLite's primary-key conflict surfaces as `SqliteFailure`
            // with extended code 1555 (SQLITE_CONSTRAINT_PRIMARYKEY) /
            // base 19 (SQLITE_CONSTRAINT). Translating to the
            // domain-level closed-set variant gives the same exit code
            // (`CatalogAlreadyExists` = 4) the Phase 1 `BTreeMap`
            // duplicate check produced.
            rusqlite::Error::SqliteFailure(err, _)
                if err.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                TomeError::CatalogAlreadyExists(catalog_name.to_owned())
            }
            other => TomeError::IndexIntegrityCheckFailure(format!(
                "insert workspace_catalogs ({workspace_name}, {catalog_name}): {other}"
            )),
        })?;
    if affected != 1 {
        return Err(TomeError::IndexIntegrityCheckFailure(format!(
            "insert workspace_catalogs ({workspace_name}, {catalog_name}): affected={affected}"
        )));
    }
    Ok(())
}

/// DELETE one enrolment. Returns `Ok(true)` when a row was removed,
/// `Ok(false)` when no matching row existed.
///
/// Concurrent removes of the same `(workspace, catalog)` resolve
/// benignly: the second observer just sees `Ok(false)` and reports
/// `CatalogNotFound`. The caller is responsible for that translation.
pub fn delete(
    conn: &Connection,
    workspace_name: &str,
    catalog_name: &str,
) -> Result<bool, TomeError> {
    let workspace_id = lookup_workspace_id(conn, workspace_name)?;
    let affected = conn
        .execute(
            "DELETE FROM workspace_catalogs
             WHERE workspace_id = ?1 AND catalog_name = ?2",
            params![workspace_id, catalog_name],
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "delete workspace_catalogs ({workspace_name}, {catalog_name}): {e}"
            ))
        })?;
    Ok(affected > 0)
}

/// All enrolments for one workspace, ordered by `catalog_name`. Used by
/// `tome catalog list` and the plugin list / interactive flows.
pub fn list_for_workspace(
    conn: &Connection,
    workspace_name: &str,
) -> Result<Vec<CatalogEnrolment>, TomeError> {
    let workspace_id = lookup_workspace_id(conn, workspace_name)?;
    let mut stmt = conn
        .prepare(
            "SELECT catalog_name, url, pinned_ref
             FROM workspace_catalogs
             WHERE workspace_id = ?1
             ORDER BY catalog_name",
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("prepare list_for_workspace: {e}"))
        })?;
    let rows = stmt
        .query_map(params![workspace_id], |row| {
            Ok(CatalogEnrolment {
                workspace_name: workspace_name.to_owned(),
                catalog_name: row.get(0)?,
                url: row.get(1)?,
                pinned_ref: row.get(2)?,
            })
        })
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("query list_for_workspace: {e}"))
        })?;
    rows.collect::<Result<_, _>>().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("collect list_for_workspace: {e}"))
    })
}

/// Look up one enrolment, returning `Ok(None)` when absent.
pub fn find(
    conn: &Connection,
    workspace_name: &str,
    catalog_name: &str,
) -> Result<Option<CatalogEnrolment>, TomeError> {
    let workspace_id = lookup_workspace_id(conn, workspace_name)?;
    conn.query_row(
        "SELECT url, pinned_ref
         FROM workspace_catalogs
         WHERE workspace_id = ?1 AND catalog_name = ?2",
        params![workspace_id, catalog_name],
        |row| {
            Ok(CatalogEnrolment {
                workspace_name: workspace_name.to_owned(),
                catalog_name: catalog_name.to_owned(),
                url: row.get(0)?,
                pinned_ref: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!(
            "find workspace_catalogs ({workspace_name}, {catalog_name}): {e}"
        ))
    })
}

/// All distinct URLs across every workspace, paired with one
/// representative `pinned_ref` (the most-recently-inserted; ties broken
/// by rowid). Used by `tome catalog update` to enumerate the URLs to
/// refresh — one git pull per URL, then per-workspace reindex.
///
/// The representative ref is informational only; the actual ref for any
/// given workspace's enrolment lives in its row and is read by the
/// reindex pass.
pub fn distinct_urls(conn: &Connection) -> Result<Vec<(String, String)>, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT url, pinned_ref
             FROM workspace_catalogs
             WHERE rowid IN (
                 SELECT MAX(rowid)
                 FROM workspace_catalogs
                 GROUP BY url
             )
             ORDER BY url",
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("prepare distinct_urls: {e}"))
        })?;
    let rows = stmt
        .query_map([], |row| {
            let url: String = row.get(0)?;
            let pinned_ref: String = row.get(1)?;
            Ok((url, pinned_ref))
        })
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("query distinct_urls: {e}")))?;
    rows.collect::<Result<_, _>>()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("collect distinct_urls: {e}")))
}

/// Count workspaces that enrol this URL. Drives `catalog remove`'s cache
/// cleanup decision: 0 → safe to `remove_dir_all` the clone; >0 → some
/// other workspace still needs it.
pub fn refcount_by_url(conn: &Connection, url: &str) -> Result<usize, TomeError> {
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_catalogs WHERE url = ?1",
            params![url],
            |row| row.get(0),
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("refcount_by_url({url}): {e}"))
        })?;
    if n < 0 {
        return Err(TomeError::IndexIntegrityCheckFailure(format!(
            "refcount_by_url({url}) returned negative count: {n}"
        )));
    }
    Ok(n as usize)
}

/// Every `(workspace_name, catalog_name)` pair pointing at `url`. Used
/// by `tome catalog update` after a successful refresh — the per-URL
/// git pull runs once, then the reindex pass visits every workspace
/// that has the catalog enrolled (FR-365).
pub fn workspaces_with_catalog_url(
    conn: &Connection,
    url: &str,
) -> Result<Vec<(String, String)>, TomeError> {
    let mut stmt = conn
        .prepare(
            "SELECT w.name, wc.catalog_name
             FROM workspace_catalogs AS wc
             JOIN workspaces         AS w ON w.id = wc.workspace_id
             WHERE wc.url = ?1
             ORDER BY w.name, wc.catalog_name",
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "prepare workspaces_with_catalog_url: {e}"
            ))
        })?;
    let rows = stmt
        .query_map(params![url], |row| {
            let workspace: String = row.get(0)?;
            let catalog: String = row.get(1)?;
            Ok((workspace, catalog))
        })
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("query workspaces_with_catalog_url: {e}"))
        })?;
    rows.collect::<Result<_, _>>().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("collect workspaces_with_catalog_url: {e}"))
    })
}

/// Resolve a `(workspace, catalog)` enrolment to the on-disk cache
/// directory via [`Paths::cache_dir_for`]. Errors with
/// [`TomeError::CatalogNotFound`] when no enrolment exists — callers
/// that need the row itself should use [`find`].
pub fn resolve_catalog_path(
    conn: &Connection,
    paths: &Paths,
    workspace_name: &str,
    catalog_name: &str,
) -> Result<std::path::PathBuf, TomeError> {
    let enrolment = find(conn, workspace_name, catalog_name)?
        .ok_or_else(|| TomeError::CatalogNotFound(catalog_name.to_owned()))?;
    Ok(paths.cache_dir_for(&enrolment.url))
}

/// Look up `workspaces.id` for the resolved workspace name. The
/// privileged `global` row is seeded at bootstrap; named workspaces are
/// seeded by US2 (`tome workspace add`). A miss here means the resolver
/// produced a name that doesn't exist in the central DB — typically a
/// stale `--workspace` flag or an orphaned project marker.
fn lookup_workspace_id(conn: &Connection, workspace_name: &str) -> Result<i64, TomeError> {
    conn.query_row(
        "SELECT id FROM workspaces WHERE name = ?1",
        params![workspace_name],
        |row| row.get::<_, i64>(0),
    )
    .map_err(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => TomeError::WorkspaceNotFound {
            name: workspace_name.to_owned(),
        },
        other => TomeError::IndexIntegrityCheckFailure(format!(
            "lookup workspace_id `{workspace_name}`: {other}"
        )),
    })
}

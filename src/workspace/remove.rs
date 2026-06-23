//! `tome workspace remove <name> [--force]` — remove a workspace with
//! the 5-step cascade per FR-405.
//!
//! Phase 4 / US2.b. Contract reference:
//! [`contracts/workspace-commands.md` §`tome workspace remove`].
//!
//! ## Algorithm
//!
//! ### Pre-checks (outside the lock)
//!
//! 1. Refuse if `name.is_reserved()` (the privileged `global` workspace
//!    is the silent default; removing it would shadow every implicit
//!    fallback). Exit 15 [`TomeError::WorkspaceNameInvalid`].
//!
//! ### Inside the advisory lockfile
//!
//! 2. Open the central DB read-write.
//! 3. Look up `workspaces.id WHERE name = ?` — exit 13
//!    [`TomeError::WorkspaceNotFound`] if absent.
//! 4. Count + fetch every bound project's path
//!    (`SELECT project_path FROM workspace_projects WHERE workspace_id
//!    = ? ORDER BY project_path`).
//! 5. If any are bound AND `!force`, exit 16
//!    [`TomeError::WorkspaceHasBoundProjects`] carrying the path list.
//!
//! ### Cascade (under the same lock; numbered per FR-405)
//!
//! **Step 1**: Tear down integration in every bound project. For each
//! `project_path` in lexicographic order:
//!   - If the project directory doesn't exist on disk, log `debug!` +
//!     skip per Edge Cases.
//!   - For each module returned by [`with_effective_modules`]: compute
//!     the rules-file target and MCP config path; call
//!     [`harness::rules_file::remove_block`] /
//!     [`harness::rules_file::remove_standalone`] +
//!     [`harness::mcp_config::remove_entry`]. Each is a no-op when the
//!     entry is absent or user-owned.
//!   - On per-project failure: `tracing::warn!` and continue. Step 1
//!     failures do NOT abort the cascade — best-effort teardown.
//!
//! **Step 2**: Remove each bound project's `<project>/.tome/` directory
//! via [`std::fs::remove_dir_all`]. Per-project failure: warn + continue.
//!
//! **Step 3**: One DB transaction:
//!   - Capture the URLs from `workspace_catalogs WHERE workspace_id = ?`
//!     BEFORE the delete (Step 5 needs them).
//!   - `DELETE FROM workspace_skills    WHERE workspace_id = ?`
//!   - `DELETE FROM workspace_catalogs  WHERE workspace_id = ?`
//!   - `DELETE FROM workspace_projects  WHERE workspace_id = ?`
//!   - `DELETE FROM workspaces          WHERE id = ?`
//!   - COMMIT.
//!
//! **Step 4**: `std::fs::remove_dir_all(<root>/workspaces/<name>/)`.
//! Failure: append to `orphaned_paths`, warn, continue.
//!
//! **Step 5**: For each URL captured in Step 3: count remaining refs in
//! `workspace_catalogs WHERE url = ?` — if zero,
//! `remove_dir_all(paths.cache_dir_for(&url))`. Successful cleanups
//! land in `catalog_caches_cleaned`; failures land in `orphaned_paths`.
//!
//! ### Failure semantics
//!
//! - Pre-checks: state unchanged.
//! - Step 1 / Step 2 failures: log + continue. Don't abort. Step 3
//!   still proceeds (the DB cascade is authoritative).
//! - Step 3 transaction failure: rollback. The workspace is NOT
//!   removed. Returns [`TomeError::IndexIntegrityCheckFailure`].
//! - Step 4 / Step 5 failures: DB already committed. Orphans recoverable
//!   by `tome doctor` (US5) — re-running `workspace remove --force
//!   <name>` for the now-already-gone workspace exits 13.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tracing::{debug, warn};

use crate::commands::plugin::registry_seeds;
use crate::error::TomeError;
use crate::harness;
use crate::index::{self, OpenOptions, acquire_lock, workspace_catalogs};
use crate::paths::Paths;
use crate::workspace::WorkspaceName;

/// Outcome of [`remove`]. Serialised by the CLI's `--json` mode.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RemoveOutcome {
    /// The name of the removed workspace.
    pub removed: WorkspaceName,
    /// Count of bound projects whose integration was torn down in
    /// Step 1.
    pub bound_projects_torn_down: u32,
    /// URLs of catalog clones whose on-disk cache was removed in
    /// Step 5 (refcount → 0 after the workspace's enrolments were
    /// dropped). Order matches enumeration of `workspace_catalogs`.
    pub catalog_caches_cleaned: Vec<String>,
    /// Paths that failed best-effort cleanup (Step 4's central
    /// `<root>/workspaces/<name>/` and / or Step 5's per-URL cache
    /// dirs). Recoverable by `tome doctor` (US5).
    pub orphaned_paths: Vec<PathBuf>,
}

/// Remove a workspace. See module-level docs for the full algorithm.
pub fn remove(
    name: WorkspaceName,
    force: bool,
    paths: &Paths,
    home_root: &Path,
) -> Result<RemoveOutcome, TomeError> {
    // Pre-check 1: refuse the reserved `global` name.
    if name.is_reserved() {
        return Err(TomeError::WorkspaceNameInvalid {
            name: name.as_str().to_owned(),
            reason: "`global` is reserved and cannot be removed".to_owned(),
        });
    }

    // Ensure the index's parent dir exists before lock acquisition.
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
            profile: None,
        },
    )?;

    // Step 3: workspace_id lookup. Exit 13 if absent.
    // Polish R-M7: route through the consolidated helper.
    let workspace_id: i64 = crate::index::workspaces::resolve_id_required(&conn, &name)?;

    // Step 4: collect bound project paths.
    let bound_projects: Vec<PathBuf> = {
        let mut stmt = conn
            .prepare(
                "SELECT project_path FROM workspace_projects
                 WHERE workspace_id = ?1
                 ORDER BY project_path",
            )
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "prepare bound_projects for remove: {e}"
                ))
            })?;
        let rows = stmt
            .query_map(rusqlite::params![workspace_id], |row| {
                let p: String = row.get(0)?;
                Ok(PathBuf::from(p))
            })
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "query bound_projects for remove: {e}"
                ))
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("collect bound_projects for remove: {e}"))
        })?
    };

    // Step 5: refuse if bound and not forced.
    if !bound_projects.is_empty() && !force {
        return Err(TomeError::WorkspaceHasBoundProjects {
            name: name.as_str().to_owned(),
            count: bound_projects.len(),
            projects: bound_projects
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
        });
    }

    // ---- Cascade Step 1: tear down integration in every bound project ----
    let mut bound_projects_torn_down: u32 = 0;
    for project in &bound_projects {
        if !project.is_dir() {
            debug!(
                project = %project.display(),
                workspace = %name.as_str(),
                "bound project directory missing on disk; skipping integration teardown"
            );
            // Still count it — the binding is being removed regardless.
            bound_projects_torn_down = bound_projects_torn_down.saturating_add(1);
            continue;
        }
        teardown_integration_for_project(project, home_root, &name, paths);
        bound_projects_torn_down = bound_projects_torn_down.saturating_add(1);
    }

    // ---- Cascade Step 2: remove each bound project's .tome/ marker ----
    for project in &bound_projects {
        let marker_dir = Paths::project_marker_dir(project);
        if !marker_dir.exists() {
            continue;
        }
        if let Err(e) = std::fs::remove_dir_all(&marker_dir) {
            warn!(
                project = %project.display(),
                marker = %marker_dir.display(),
                error = %e,
                "failed to remove project marker directory; continuing cascade"
            );
        }
    }

    // ---- Cascade Step 3: one DB transaction ----
    // Capture URLs BEFORE the delete so Step 5 can refcount them.
    let urls_to_check: Vec<String> = {
        let mut stmt = conn
            .prepare(
                "SELECT url FROM workspace_catalogs
                 WHERE workspace_id = ?1
                 ORDER BY catalog_name",
            )
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "prepare workspace_catalogs.url scan for remove: {e}"
                ))
            })?;
        let rows = stmt
            .query_map(rusqlite::params![workspace_id], |row| {
                let url: String = row.get(0)?;
                Ok(url)
            })
            .map_err(|e| {
                TomeError::IndexIntegrityCheckFailure(format!(
                    "query workspace_catalogs.url scan for remove: {e}"
                ))
            })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "collect workspace_catalogs.url scan for remove: {e}"
            ))
        })?
    };

    let tx = conn.transaction().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("begin remove transaction: {e}"))
    })?;
    tx.execute(
        "DELETE FROM workspace_skills WHERE workspace_id = ?1",
        rusqlite::params![workspace_id],
    )
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("delete workspace_skills: {e}")))?;
    tx.execute(
        "DELETE FROM workspace_catalogs WHERE workspace_id = ?1",
        rusqlite::params![workspace_id],
    )
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("delete workspace_catalogs: {e}"))
    })?;
    tx.execute(
        "DELETE FROM workspace_projects WHERE workspace_id = ?1",
        rusqlite::params![workspace_id],
    )
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("delete workspace_projects: {e}"))
    })?;
    tx.execute(
        "DELETE FROM workspaces WHERE id = ?1",
        rusqlite::params![workspace_id],
    )
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("delete workspaces row: {e}")))?;
    tx.commit().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("commit remove transaction: {e}"))
    })?;

    // ---- Cascade Step 4: remove the central workspace directory ----
    let mut orphaned_paths: Vec<PathBuf> = Vec::new();
    let workspace_dir = paths.workspace_dir(&name);
    if workspace_dir.exists()
        && let Err(e) = std::fs::remove_dir_all(&workspace_dir)
    {
        warn!(
            workspace = %name.as_str(),
            dir = %workspace_dir.display(),
            error = %e,
            "failed to remove central workspace directory; recoverable via `tome doctor`"
        );
        orphaned_paths.push(workspace_dir);
    }

    // ---- Cascade Step 5: refcount-clean orphaned catalog caches ----
    // The advisory lock is still held; the refcount check below + the
    // remove are atomic relative to any other writer (concurrent
    // `catalog add` / `workspace init --inherit-global` etc. all wait
    // on the same lockfile).
    let mut catalog_caches_cleaned: Vec<String> = Vec::new();
    let mut seen_urls = std::collections::HashSet::new();
    for url in &urls_to_check {
        if !seen_urls.insert(url.clone()) {
            // Dedupe — the same URL may be referenced under different
            // catalog_name values by the workspace being removed; we
            // only need one refcount check + one cache removal per URL.
            continue;
        }
        let refcount = workspace_catalogs::refcount_by_url(&conn, url)?;
        if refcount > 0 {
            debug!(
                url = %url,
                refcount,
                "catalog cache still referenced by other workspace(s); skipping cleanup"
            );
            continue;
        }
        let cache_dir = paths.cache_dir_for(url);
        if !cache_dir.exists() {
            // No cache on disk — record the URL as cleaned so the audit
            // trail mirrors the DB-side state. The on-disk absence is
            // benign (catalog never cloned, or already cleaned).
            catalog_caches_cleaned.push(url.clone());
            continue;
        }
        match std::fs::remove_dir_all(&cache_dir) {
            Ok(()) => catalog_caches_cleaned.push(url.clone()),
            Err(e) => {
                warn!(
                    url = %url,
                    cache_dir = %cache_dir.display(),
                    error = %e,
                    "failed to remove orphan catalog cache directory; recoverable via `tome doctor`"
                );
                orphaned_paths.push(cache_dir);
            }
        }
    }

    // Drop the DB handle so any WAL checkpoint completes inside the
    // lock window.
    drop(conn);
    drop(lock);

    Ok(RemoveOutcome {
        removed: name,
        bound_projects_torn_down,
        catalog_caches_cleaned,
        orphaned_paths,
    })
}

/// Step-1 helper: tear down EVERY Tome-owned harness integration for this
/// project. Routes through the sync orchestrator's empty-effective-set teardown
/// ([`harness::sync::teardown_project`], PW4) so it unwinds the SAME set of
/// sinks the writers populated — rules files, MCP entries, plugin hooks, Tome's
/// own session hooks, the new `CommandHook` session entries, TS plugin shims,
/// guardrails, native agents, AND the Open Plugins `tome-op` bundle — including
/// the opt-in targets (`generic` `mcp.json`, `generic-op`/`goose` bundles),
/// which the orchestrator picks up via its artifact-present probe.
///
/// Using the orchestrator's removal path (rather than a bespoke rules+MCP
/// teardown) means teardown inherits every safety guard the writers have:
/// structural-match-only removal, marker-bounded edits, symlink refusal, and
/// the never-mass-delete-what-we-don't-own checks. A harness that was never
/// enrolled for THIS project leaves no artifact, so its removal branch is a
/// no-op — nothing other than Tome's own content is touched.
///
/// Best-effort per the contract: any error is logged at `warn!` and swallowed
/// so the cascade always proceeds (the DB delete is authoritative). The
/// teardown does NOT require a readable project marker (the empty effective set
/// resolves no scope), so a project whose marker is already corrupt still tears
/// down cleanly — doctor surfaces any residual drift.
fn teardown_integration_for_project(
    project: &Path,
    home_root: &Path,
    workspace_name: &WorkspaceName,
    paths: &Paths,
) {
    let deps = harness::sync::build_deps(paths, home_root, workspace_name, false);
    if let Err(e) = harness::sync::teardown_project(project, &deps) {
        warn!(
            project = %project.display(),
            workspace = %workspace_name.as_str(),
            error = %e,
            "harness integration teardown reported an error; continuing cascade"
        );
    }
}

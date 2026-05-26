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
use crate::harness::{self, McpConfigFormat, RulesFileStrategy, with_effective_modules};
use crate::index::{self, OpenOptions, acquire_lock, workspace_catalogs};
use crate::paths::Paths;
use crate::settings::{self, GlobalSettings, resolve_effective_list, resolver::StubScope};
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

/// Step-1 helper: for the harnesses in the per-project effective list,
/// remove the Tome rules-file block / standalone file and the Tome MCP
/// entry. Per-harness failures are logged at `warn!` and swallowed —
/// best-effort teardown per the contract.
///
/// Per-project narrowing: the effective list is computed from the
/// project's own marker (`<project>/.tome/config.toml`) composed with
/// the bound workspace's `settings.toml` and the global settings — same
/// algorithm `harness::sync` uses to populate, mirrored on the way out.
/// A harness that was never enrolled for THIS project is left alone
/// even if it's enrolled for some other workspace's projects.
///
/// If the per-project marker is missing or any settings layer can't be
/// read or parsed, log warn + skip teardown for THIS project (the
/// workspace is being removed anyway; doctor surfaces residual drift).
fn teardown_integration_for_project(
    project: &Path,
    home_root: &Path,
    workspace_name: &WorkspaceName,
    paths: &Paths,
) {
    // Compute the per-project effective list. On any read/parse error
    // we warn and fall back to an empty effective list — the cascade is
    // best-effort and a remove of a no-longer-coherent workspace
    // shouldn't get stuck.
    let effective_names: std::collections::HashSet<String> =
        match compute_effective_names_for_project(project, workspace_name, paths) {
            Ok(names) => names,
            Err(e) => {
                warn!(
                    project = %project.display(),
                    workspace = %workspace_name.as_str(),
                    error = %e,
                    "could not compute effective harness list for project; skipping per-project teardown",
                );
                return;
            }
        };

    // Capture per-harness specifics into owned values up front so we
    // don't hold the registry's read guard across `std::fs` work.
    struct Snapshot {
        name: String,
        rules_path: PathBuf,
        rules_strategy: RulesFileStrategy,
        mcp_path: PathBuf,
        mcp_format: McpConfigFormat,
        mcp_parent_key: &'static str,
    }

    let snapshots: Vec<Snapshot> = with_effective_modules(|mods| {
        mods.iter()
            .filter(|m| effective_names.contains(m.name()))
            .map(|m| Snapshot {
                name: m.name().to_string(),
                rules_path: m.rules_file_target(project),
                rules_strategy: m.rules_file_strategy(),
                mcp_path: m.mcp_config_path(project, home_root),
                mcp_format: m.mcp_config_format(),
                mcp_parent_key: m.mcp_parent_key(),
            })
            .collect()
    });

    // Dedupe rules-file targets and MCP config paths — multiple
    // harnesses may share the same on-disk file (FR-482 / FR-483).
    let mut processed_rules: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut processed_mcp: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

    for snap in &snapshots {
        if processed_rules.insert(snap.rules_path.clone()) {
            let rules_result = match snap.rules_strategy {
                RulesFileStrategy::BlockInExistingFile => {
                    if snap.rules_path.exists() {
                        harness::rules_file::remove_block(&snap.rules_path)
                    } else {
                        Ok(())
                    }
                }
                RulesFileStrategy::StandaloneFile => {
                    if snap.rules_path.exists() {
                        harness::rules_file::remove_standalone(&snap.rules_path)
                    } else {
                        Ok(())
                    }
                }
            };
            if let Err(e) = rules_result {
                warn!(
                    harness = %snap.name,
                    rules_path = %snap.rules_path.display(),
                    error = %e,
                    "failed to remove rules-file integration; continuing teardown"
                );
            }
        }

        if processed_mcp.insert(snap.mcp_path.clone())
            && let Err(e) = harness::mcp_config::remove_entry(
                &snap.mcp_path,
                snap.mcp_format,
                snap.mcp_parent_key,
            )
        {
            warn!(
                harness = %snap.name,
                mcp_path = %snap.mcp_path.display(),
                error = %e,
                "failed to remove mcp-config entry; continuing teardown"
            );
        }
    }
}

/// Read the three settings layers (project marker, workspace
/// `settings.toml`, global `settings.toml`) and compute the per-project
/// effective harness name set. Mirrors `harness::sync::sync_project`'s
/// composition path; uses an empty `StubScope` because the cascade does
/// not need cross-workspace composition references resolved (they would
/// only appear in the project marker's `harnesses` array, and an
/// unresolvable reference there at cascade-time is moot — we're
/// removing the binding regardless).
fn compute_effective_names_for_project(
    project: &Path,
    _workspace_name: &WorkspaceName,
    paths: &Paths,
) -> Result<std::collections::HashSet<String>, TomeError> {
    let marker_path = Paths::project_marker_config(project);
    let marker_body =
        crate::util::bounded_read_to_string(&marker_path, crate::util::TOME_CONFIG_MAX)?;
    let marker = settings::parser::parse_project_marker(&marker_body).map_err(|e| {
        TomeError::WorkspaceMalformed {
            path: marker_path.clone(),
            reason: format!("parse project marker: {e}"),
        }
    })?;

    // Read the bound workspace's `settings.toml` (if present). The
    // workspace name from the marker is authoritative — if it disagrees
    // with the workspace being removed, that's a doctor-flaggable drift
    // that we don't need to resolve here.
    let workspace_settings = {
        let path = paths.workspace_settings_file(&marker.workspace);
        match crate::util::bounded_read_to_string(&path, crate::util::TOME_CONFIG_MAX) {
            Ok(body) => Some(settings::parser::parse_workspace(&body).map_err(|e| {
                TomeError::WorkspaceMalformed {
                    path: path.clone(),
                    reason: format!("parse workspace settings: {e}"),
                }
            })?),
            Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        }
    };

    let global_settings = match crate::util::bounded_read_to_string(
        &paths.global_settings_file,
        crate::util::TOME_CONFIG_MAX,
    ) {
        Ok(body) => {
            settings::parser::parse_global(&body).map_err(|e| TomeError::WorkspaceMalformed {
                path: paths.global_settings_file.clone(),
                reason: format!("parse global settings: {e}"),
            })?
        }
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            GlobalSettings::default()
        }
        Err(e) => return Err(e),
    };

    let scope = StubScope::new();
    let effective = resolve_effective_list(
        Some(&marker),
        workspace_settings.as_ref(),
        &global_settings,
        &scope,
    )
    .map_err(TomeError::from)?;
    Ok(effective.harnesses.into_iter().map(|h| h.name).collect())
}

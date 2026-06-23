//! Project-to-workspace binding for `tome workspace use`.
//!
//! Owns the Phase 4 / US1.a algorithm in
//! [`contracts/workspace-commands.md` §`tome workspace use`]: the lock
//! acquisition, the `workspace_projects` UPSERT, the `last_used_at`
//! bump, and the atomic `<project>/.tome/` landing carrying the binding
//! pointer `config.toml`.
//!
//! Stub-clean — this module never spawns harness sync work; that lives
//! in [`crate::commands::harness::sync_for_project_root`] (US1.a stub,
//! US1.b real). Splitting the seam this way means the integration tests
//! for binding can exercise the DB + filesystem path without depending
//! on any harness module's behaviour.
//!
//! ## Atomicity tier per contract
//!
//! - Phase A (this module): lockfile → DB UPSERT → marker landing →
//!   release lock. If the UPSERT commits but the marker landing fails,
//!   the central DB has a row but `<project>/.tome/` is absent. Doctor's
//!   `Binding` subsystem (Phase 4 follow-up) flags the orphan; re-running
//!   `tome workspace use <same-name>` recovers the marker without
//!   changing the DB row.
//! - Phase B (the harness sync): runs outside this module, outside the
//!   lockfile. US1.b owns its concurrency profile.

use std::path::{Path, PathBuf};

use serde::Serialize;
use time::OffsetDateTime;

use crate::commands::plugin::registry_seeds;
use crate::error::TomeError;
use crate::harness::sync::SyncOutcome;
use crate::index::{self, OpenOptions, acquire_lock};
use crate::paths::Paths;
use crate::workspace::WorkspaceName;

/// Inputs to [`bind_project`] that the caller supplies. `home_root` is
/// passed in (rather than read from `$HOME`) so tests can isolate the
/// dangerous-cwd check against a tempdir without env mutation; the CLI
/// wrapper resolves it via `std::env::var_os("HOME")`.
#[derive(Debug)]
pub struct BindDeps<'a> {
    pub paths: &'a Paths,
    pub home_root: &'a Path,
}

/// Wire-shape outcome of [`bind_project`]. Serialised by the CLI's
/// `--json` mode. `sync` stays `None` for the binding step itself; the
/// CLI wrapper fills it after calling
/// [`crate::commands::harness::sync_for_project_root`].
#[derive(Debug, Clone, Serialize)]
pub struct BindOutcome {
    /// The workspace name now bound to this project.
    pub workspace: WorkspaceName,
    /// Canonicalised absolute path of the project root the binding
    /// targeted.
    pub project_root: PathBuf,
    /// True iff `<project_root>/.tome/` was created by this call. False
    /// when an existing marker was replaced.
    pub created_marker: bool,
    /// When the project was previously bound to a different workspace,
    /// names that prior workspace. `None` on first bind or when re-binding
    /// to the same workspace.
    pub rebind_from: Option<WorkspaceName>,
    /// Filled by the CLI wrapper after the (stubbed in US1.a) harness
    /// sync runs. Always `None` when returned from this module directly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sync: Option<SyncOutcome>,
}

/// Refuse to bind certain CWDs that are almost certainly mistakes:
/// `$HOME` and `/`. The CLI wrapper checks this before acquiring the
/// lock so a typo doesn't materialise a `.tome/` in the user's home
/// directory. `--force` bypasses the check via the wrapper, not here.
///
/// `cwd` is canonicalised strictly — a canonicalise failure surfaces as
/// [`TomeError::Io`] (exit 7) so the dangerous-cwd check never silently
/// succeeds against a path it could not normalise. `home` canonicalises
/// best-effort: an unreadable `$HOME` is not a reason to refuse, but the
/// caller's `cwd == home` comparison still works against the literal
/// path.
pub fn is_project_root_acceptable(cwd: &Path, home: &Path) -> Result<(), TomeError> {
    let cwd_c = cwd.canonicalize().map_err(TomeError::Io)?;
    let home_c = home.canonicalize().unwrap_or_else(|_| home.to_path_buf());

    if cwd_c == home_c {
        return Err(TomeError::Usage(format!(
            "refusing to bind {}: that is the user's home directory; rerun with `--force` if you really mean it",
            cwd_c.display(),
        )));
    }
    if cwd_c == Path::new("/") {
        return Err(TomeError::Usage(format!(
            "refusing to bind {}: that is the filesystem root; rerun with `--force` if you really mean it",
            cwd_c.display(),
        )));
    }
    Ok(())
}

/// Bind `target_root` to the named workspace.
///
/// Algorithm:
///
/// 1. Canonicalise `target_root` (parent must exist on disk — the CLI
///    contract is that the CWD must exist).
/// 2. Acquire the central DB advisory lockfile.
/// 3. Open the central index (registry-seeded; the binding never needs
///    inference, but the seeds make a fresh `meta` table consistent
///    with what production opens later).
/// 4. Resolve `(workspace_id, prior_workspace)` for the target row in
///    `workspace_projects`; bail with [`TomeError::WorkspaceNotFound`]
///    if the workspace name has no row in `workspaces`.
/// 5. UPSERT the `workspace_projects` row.
/// 6. Bump `workspaces.last_used_at` for the bound workspace (FR-411).
/// 7. Atomically land `<target_root>/.tome/` (mode 0o700 on Unix) with
///    `config.toml` carrying `workspace = "<name>"` and (optionally) a
///    copy of the workspace's `RULES.md`.
/// 8. Release the lockfile.
///
/// `force` is consumed for symmetry with the contract — the actual
/// `--force` semantics (overwriting a dangerous CWD) are enforced by
/// the CLI wrapper before this function runs. Inside binding, force is
/// not currently distinct from non-force; the parameter is reserved for
/// future use (e.g. "rebind even if marker is locked").
pub fn bind_project(
    target_root: &Path,
    name: WorkspaceName,
    _force: bool,
    deps: &BindDeps<'_>,
) -> Result<BindOutcome, TomeError> {
    let canonical_target = target_root.canonicalize().map_err(TomeError::Io)?;

    // Refuse non-UTF8 project paths — every downstream consumer (DB
    // `workspace_projects.project_path` column, JSON wire format, the
    // marker config.toml string) assumes UTF-8. Surface the failure
    // crisply rather than letting `to_string_lossy` paper over the
    // invariant.
    if canonical_target.to_str().is_none() {
        return Err(TomeError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "project path is not valid UTF-8: {}",
                canonical_target.display()
            ),
        )));
    }

    // Make sure the parent of index.db exists; lock acquisition will
    // create the lockfile itself, but the surrounding directory must
    // already be present.
    if let Some(parent) = deps.paths.index_lock.parent()
        && !parent.exists()
    {
        std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    }

    let lock = acquire_lock(&deps.paths.index_lock)?;

    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let mut conn = index::open(
        &deps.paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
            summariser: summariser_seed,
            profile: None,
        },
    )?;

    // Step 4a: resolve workspace_id for the named workspace.
    // Polish R-M7: route through the consolidated helper.
    let workspace_id: i64 = crate::index::workspaces::resolve_id_required(&conn, &name)?;

    // Step 4b: capture the prior binding (if any) so we can report
    // rebind_from in the outcome.
    let project_path_str = canonical_target.to_string_lossy().into_owned();
    let prior_workspace_name: Option<String> = conn
        .query_row(
            "SELECT w.name
             FROM workspace_projects AS wp
             JOIN workspaces AS w ON w.id = wp.workspace_id
             WHERE wp.project_path = ?1",
            rusqlite::params![project_path_str.as_str()],
            |row| row.get(0),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("read prior binding: {e}")))?;

    let rebind_from = match prior_workspace_name.as_deref() {
        Some(prior) if prior != name.as_str() => Some(WorkspaceName::parse(prior)?),
        _ => None,
    };

    // Steps 5+6: UPSERT the workspace_projects row AND bump
    // workspaces.last_used_at inside one transaction (FR-411). Either
    // both writes commit, or neither — a partial that left the binding
    // installed without bumping `last_used_at` would let a stale `tome
    // workspace list --by last_used_at` ordering survive across runs.
    let now_unix = OffsetDateTime::now_utc().unix_timestamp();
    let tx = conn.transaction().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("begin bind transaction: {e}"))
    })?;
    tx.execute(
        "INSERT INTO workspace_projects (project_path, workspace_id, bound_at)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(project_path)
         DO UPDATE SET workspace_id = excluded.workspace_id,
                       bound_at     = excluded.bound_at",
        rusqlite::params![project_path_str.as_str(), workspace_id, now_unix],
    )
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("upsert workspace_projects: {e}"))
    })?;
    tx.execute(
        "UPDATE workspaces SET last_used_at = ?1 WHERE id = ?2",
        rusqlite::params![now_unix, workspace_id],
    )
    .map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("bump workspaces.last_used_at: {e}"))
    })?;
    tx.commit().map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("commit bind transaction: {e}"))
    })?;

    // Step 7: land the project marker atomically.
    let marker_dir = Paths::project_marker_dir(&canonical_target);
    let created_marker = !marker_dir.exists();

    // Capture the workspace's RULES.md path (may not yet exist; that's
    // fine — US2/US4 own the source).
    let rules_src = deps.paths.workspace_rules_file(&name);
    let workspace_line = format!("workspace = \"{}\"\n", name.as_str());

    crate::util::atomic_dir::land_directory_with_replace(
        &marker_dir,
        0o700,
        |staging: &Path| -> Result<(), TomeError> {
            std::fs::write(staging.join("config.toml"), &workspace_line).map_err(TomeError::Io)?;
            if rules_src.is_file() {
                std::fs::copy(&rules_src, staging.join("RULES.md")).map_err(TomeError::Io)?;
            }
            Ok(())
        },
    )?;

    // Drop the DB handle before releasing the lock so any final WAL
    // checkpoint completes within the lock window.
    drop(conn);
    lock.release()?;

    Ok(BindOutcome {
        workspace: name,
        project_root: canonical_target,
        created_marker,
        rebind_from,
        sync: None,
    })
}

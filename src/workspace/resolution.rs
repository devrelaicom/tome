//! Phase 4 workspace resolution.
//!
//! Picks the active workspace name for a single Tome invocation. Honours,
//! in priority order:
//!
//! 1. `--workspace <name>` CLI flag → [`ScopeSource::Flag`].
//! 2. `TOME_WORKSPACE` env var → [`ScopeSource::Env`].
//! 3. Project marker walk → [`ScopeSource::ProjectMarker`] if any ancestor
//!    of CWD contains `.tome/config.toml`.
//! 4. Global fallback → [`ScopeSource::GlobalFallback`].
//!
//! Phase 4 collapses workspace identity from on-disk paths into validated
//! [`WorkspaceName`]s. The central `workspaces` table is the source of
//! truth: every flag-/env-/marker-supplied name is verified against it.
//! A name that is not present returns [`TomeError::WorkspaceNotFound`]
//! (exit 13). A malformed marker file returns
//! [`TomeError::WorkspaceMalformed`] (exit 70). The Phase 3 `--global`
//! flag and the `WorkspaceMarkerMissing` / `WorkspaceConflict` variants
//! are gone.
//!
//! Contract: `contracts/workspace-resolution.md`.

use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;

use crate::cli::GlobalScopeArgs;
use crate::error::TomeError;
use crate::paths::Paths;
use crate::settings::parser::read_project_marker;
use crate::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

/// Compute the active scope for this invocation. Touches the filesystem
/// (project-marker walk + central DB membership check); errors surface
/// as the dedicated workspace-* exit codes (13/15/70) per FR-344..347.
pub fn resolve(args: &GlobalScopeArgs, paths: &Paths) -> Result<ResolvedScope, TomeError> {
    // 1. `--workspace <name>`.
    if let Some(raw) = args.workspace.as_deref() {
        let name = WorkspaceName::parse(raw)?;
        require_workspace_membership(&name, paths)?;
        log_resolution(&name, ScopeSource::Flag, None);
        return Ok(ResolvedScope {
            scope: Scope(name),
            source: ScopeSource::Flag,
            project_root: None,
        });
    }

    // 2. `TOME_WORKSPACE` env var.
    if let Some(raw) = std::env::var_os("TOME_WORKSPACE") {
        let s = raw.to_string_lossy();
        if !s.is_empty() {
            let name = WorkspaceName::parse(&s)?;
            require_workspace_membership(&name, paths)?;
            log_resolution(&name, ScopeSource::Env, None);
            return Ok(ResolvedScope {
                scope: Scope(name),
                source: ScopeSource::Env,
                project_root: None,
            });
        }
    }

    // 3. Project-marker walk.
    if let Some((project_root, marker_path)) = walk_for_project_marker() {
        let cfg = read_project_marker(&marker_path)?;
        require_workspace_membership(&cfg.workspace, paths)?;
        log_resolution(
            &cfg.workspace,
            ScopeSource::ProjectMarker,
            Some(&project_root),
        );
        return Ok(ResolvedScope {
            scope: Scope(cfg.workspace),
            source: ScopeSource::ProjectMarker,
            project_root: Some(project_root),
        });
    }

    // 4. Global fallback.
    let fallback = ResolvedScope::global_fallback();
    log_resolution(fallback.scope.name(), ScopeSource::GlobalFallback, None);
    Ok(fallback)
}

/// Verify `name` is present in the central `workspaces` table.
///
/// Specialised for the no-DB case: when the central index file does not
/// yet exist (e.g. the very first invocation, before any write command
/// has bootstrapped the DB), only the privileged `global` name passes.
/// Every other name returns [`TomeError::WorkspaceNotFound`] so the
/// developer learns immediately that they need to
/// `tome workspace init <name>` first.
fn require_workspace_membership(name: &WorkspaceName, paths: &Paths) -> Result<(), TomeError> {
    if !paths.index_db.exists() {
        if name.is_reserved() {
            return Ok(());
        }
        return Err(TomeError::WorkspaceNotFound {
            name: name.as_str().to_owned(),
        });
    }
    let conn = crate::index::open_read_only(&paths.index_db)?;
    let exists: bool = conn
        .query_row(
            "SELECT 1 FROM workspaces WHERE name = ?1",
            [name.as_str()],
            |_| Ok(true),
        )
        .optional()
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("workspace membership check: {e}"))
        })?
        .unwrap_or(false);
    if !exists {
        return Err(TomeError::WorkspaceNotFound {
            name: name.as_str().to_owned(),
        });
    }
    Ok(())
}

/// Walk parents from `current_dir()` toward `/`. Returns the first
/// `(ancestor_dir, marker_path)` whose `.tome/config.toml` exists. Stops
/// at the filesystem root. Non-`NotFound` IO errors from `try_exists`
/// swallow and fall through to global with a debug log (per Phase 3
/// discipline carried forward).
fn walk_for_project_marker() -> Option<(PathBuf, PathBuf)> {
    let mut here = std::env::current_dir().ok()?;
    loop {
        let marker = Paths::project_marker_config(&here);
        match marker.try_exists() {
            Ok(true) => {
                let canon = here.canonicalize().unwrap_or(here.clone());
                return Some((canon, marker));
            }
            Ok(false) => {}
            Err(e) => {
                tracing::debug!(
                    ?e,
                    here = %here.display(),
                    "workspace project-marker walk: IO error on try_exists, falling through",
                );
                return None;
            }
        }
        if !here.pop() {
            break;
        }
    }
    None
}

/// Emit the standard debug-level resolution trace per contract §Debug
/// logging. Single source so the wire format is uniform across the
/// resolver's exit points.
fn log_resolution(name: &WorkspaceName, source: ScopeSource, project_root: Option<&Path>) {
    match project_root {
        Some(root) => tracing::debug!(
            name = name.as_str(),
            ?source,
            project_root = %root.display(),
            "scope resolved",
        ),
        None => tracing::debug!(name = name.as_str(), ?source, "scope resolved",),
    }
}

//! Resolution algorithm — picks the active `Scope` for a single Tome
//! invocation. Honours, in priority order:
//!
//! 1. `--workspace <path>` CLI flag → `ScopeSource::Flag`.
//! 2. `--global` CLI flag → `ScopeSource::GlobalFlag`.
//! 3. `TOME_WORKSPACE` env var → `ScopeSource::Env`.
//! 4. CWD walk → `ScopeSource::CwdWalk` if any parent has `.tome/`.
//! 5. Global fallback → `ScopeSource::GlobalFallback`.
//!
//! Contract: `contracts/workspace-resolution.md`.
//!
//! Failure modes:
//! - Both `--workspace` and `--global` set → exit 72 `WorkspaceConflict`.
//!   We DON'T use clap's `conflicts_with` because that exits 2 (clap's
//!   usage-error code); the contract requires the dedicated 72 code so
//!   harnesses can distinguish the workspace conflict from a generic
//!   typo.
//! - Explicit `--workspace <path>` or `TOME_WORKSPACE` naming a path
//!   that doesn't exist OR has no `.tome/` marker → exit 71
//!   `WorkspaceNotFound`. Silent fall-through would mask configuration
//!   bugs (the user named a workspace; the resolver must NOT pretend
//!   they didn't).
//! - The CWD walk swallows non-`NotFound` `io::Error` and falls
//!   through to the global fallback (logged at debug). Resolution must
//!   be cheap and predictable; an unreadable parent directory shouldn't
//!   abort the entire invocation.

use std::path::PathBuf;

use crate::cli::GlobalScopeArgs;
use crate::error::TomeError;
use crate::workspace::{ResolvedScope, Scope, ScopeSource};

/// Compute the active scope for this invocation. Pure function over
/// `args` + process-state (env vars, CWD); takes no `&Paths` because
/// resolution is layer-zero — `Paths::index_db_for(&scope)` consumes
/// the result.
pub fn resolve(args: &GlobalScopeArgs) -> Result<ResolvedScope, TomeError> {
    // Priority 0: mutually-exclusive flag pair. Detect before priority
    // 1/2 so the order in which clap populated them is irrelevant.
    if args.workspace.is_some() && args.global {
        return Err(TomeError::WorkspaceConflict);
    }

    // Priority 1: --workspace <path>.
    if let Some(raw) = args.workspace.as_ref() {
        let resolved = validate_workspace_path(raw)?;
        log_resolution(&resolved.scope, ScopeSource::Flag);
        return Ok(ResolvedScope {
            scope: resolved.scope,
            source: ScopeSource::Flag,
        });
    }

    // Priority 2: --global.
    if args.global {
        log_resolution(&Scope::Global, ScopeSource::GlobalFlag);
        return Ok(ResolvedScope {
            scope: Scope::Global,
            source: ScopeSource::GlobalFlag,
        });
    }

    // Priority 3: TOME_WORKSPACE env var.
    if let Some(env_path) = std::env::var_os("TOME_WORKSPACE")
        && !env_path.is_empty()
    {
        let raw = PathBuf::from(&env_path);
        let resolved = validate_workspace_path(&raw)?;
        log_resolution(&resolved.scope, ScopeSource::Env);
        return Ok(ResolvedScope {
            scope: resolved.scope,
            source: ScopeSource::Env,
        });
    }

    // Priority 4: CWD walk.
    if let Some(root) = walk_cwd_for_marker()? {
        let scope = Scope::Workspace(root);
        log_resolution(&scope, ScopeSource::CwdWalk);
        return Ok(ResolvedScope {
            scope,
            source: ScopeSource::CwdWalk,
        });
    }

    // Priority 5: global fallback.
    log_resolution(&Scope::Global, ScopeSource::GlobalFallback);
    Ok(ResolvedScope::global_fallback())
}

/// Validate that `raw` points at a workspace root: the path exists, is
/// canonicalisable, and contains a `.tome/` subdir. Returns the scope
/// with the canonicalised absolute path on success, `WorkspaceNotFound`
/// otherwise. Used for both `--workspace` and `TOME_WORKSPACE`.
///
/// Also enforces contract `workspace-resolution.md` §Validation 1b/1c:
/// a workspace whose `.tome/config.toml` is unparsable, or whose
/// `.tome/index.db` exists but cannot be opened, exits with
/// `WorkspaceMalformed` (70). The check runs once at the resolution
/// boundary — every downstream command sees a usable workspace or a
/// clean 70 exit pointing at `tome --global doctor` as the escape
/// hatch.
fn validate_workspace_path(raw: &PathBuf) -> Result<ResolvedScope, TomeError> {
    let absolute = std::fs::canonicalize(raw)
        .map_err(|_| TomeError::WorkspaceNotFound { path: raw.clone() })?;
    let marker = absolute.join(".tome");
    if !marker.is_dir() {
        return Err(TomeError::WorkspaceNotFound { path: absolute });
    }
    validate_workspace_contents(&absolute)?;
    Ok(ResolvedScope {
        scope: Scope::Workspace(absolute),
        // `source` is filled in by the caller — callers know whether the
        // path came from the flag or the env var. We return a synthetic
        // `Flag` source here and let the caller overwrite it; cheaper
        // than threading the source through this helper.
        source: ScopeSource::Flag,
    })
}

/// Enforce contract `workspace-resolution.md` §Validation 1b/1c:
///
/// - `.tome/config.toml` must be readable as a valid `Config`. Absent is
///   allowed (a freshly-bootstrapped workspace has no config until the
///   first `catalog add`); unparsable → `WorkspaceMalformed` (70).
/// - `.tome/index.db` may or may not exist. If it exists as a file, it
///   must open read-only (Phase 2 PRAGMA + integrity rules); failure
///   → `WorkspaceMalformed` (70) with reason `"index database
///   malformed"`.
///
/// Escape hatch: `tome --global doctor` bypasses workspace resolution
/// and produces a diagnostic against the global scope.
fn validate_workspace_contents(absolute: &std::path::Path) -> Result<(), TomeError> {
    let config_path = absolute.join(".tome/config.toml");
    if config_path.exists()
        && let Err(e) = crate::catalog::store::load(&config_path)
    {
        return Err(TomeError::WorkspaceMalformed {
            path: absolute.to_path_buf(),
            reason: format!("config.toml: {e}"),
        });
    }
    let index_path = absolute.join(".tome/index.db");
    if index_path.is_file()
        && let Err(e) = crate::index::open_read_only(&index_path)
    {
        return Err(TomeError::WorkspaceMalformed {
            path: absolute.to_path_buf(),
            reason: format!("index database malformed: {e}"),
        });
    }
    Ok(())
}

/// Walk from `current_dir()` toward `/`, returning the first parent
/// directory whose `.tome/` subdir exists (canonicalised so symlinks
/// resolve once). Stops at the filesystem root. Non-`NotFound`
/// `io::Error` does NOT propagate; the walk falls through and logs.
///
/// Once a marker is found, [`validate_workspace_contents`] runs against
/// it. A malformed CWD-walked workspace yields `WorkspaceMalformed`
/// (70) — same contract gate as explicit `--workspace` / `TOME_WORKSPACE`.
fn walk_cwd_for_marker() -> Result<Option<PathBuf>, TomeError> {
    let mut here = std::env::current_dir().map_err(TomeError::Io)?;
    loop {
        let marker = here.join(".tome");
        match marker.try_exists() {
            Ok(true) => {
                let absolute = here.canonicalize().map_err(TomeError::Io)?;
                validate_workspace_contents(&absolute)?;
                return Ok(Some(absolute));
            }
            Ok(false) => {}
            Err(e) => {
                tracing::debug!(?e, here = %here.display(), "workspace cwd walk: IO error on try_exists, falling through to global");
                return Ok(None);
            }
        }
        if !here.pop() {
            break;
        }
    }
    Ok(None)
}

/// Emit the standard debug-level resolution trace per contract §Debug
/// logging. Single source so the wire format is uniform across the
/// resolver's exit points.
fn log_resolution(scope: &Scope, source: ScopeSource) {
    match scope {
        Scope::Global => {
            tracing::debug!(scope = "global", ?source, "scope resolved");
        }
        Scope::Workspace(path) => {
            tracing::debug!(
                scope = "workspace",
                path = %path.display(),
                ?source,
                "scope resolved",
            );
        }
    }
}

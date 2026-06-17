//! `tome harness session-start` — print the workspace's skill-routing
//! directive to stdout, regenerated fresh from live state.
//!
//! This is the target of the Tome-owned Claude Code SessionStart hook
//! (`src/harness/routing.rs::session_start_hook`): Claude Code runs it at the
//! start of every session and injects its stdout as `additionalContext`. It is
//! the on-demand, always-current sibling of the on-disk `RULES.md` produced by
//! [`crate::harness::routing::write_workspace_rules`] — same directive bytes,
//! but computed at session start rather than at enable/disable/tier-change time.

use std::io::Write;

use crate::cli::HarnessSessionStartArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::{ResolvedScope, WorkspaceName};

/// Print the routing directive to stdout for the resolved (or `--workspace`)
/// workspace. Always plain text — the Claude Code SessionStart hook captures
/// stdout as `additionalContext` regardless of the global `--json` flag, so
/// this command does not branch on `Mode`.
pub fn run(
    args: HarnessSessionStartArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    _mode: Mode,
) -> Result<(), TomeError> {
    let name: WorkspaceName = match args.workspace.as_deref() {
        Some(raw) => WorkspaceName::parse(raw)?,
        None => scope.scope.name().clone(),
    };

    // Reconcile this project's files before printing, so the directive we emit
    // is consistent with freshly-synced harness files (and `.tome/RULES.md`).
    // FAIL-SOFT: a sync error must never block or fail the session-start hook —
    // warn and continue; the directive prints regardless. (No `?` on the call.)
    if let Some(project_root) = scope.project_root.as_deref() {
        let sync_args = crate::cli::SyncArgs {
            all: false,
            rules_only: false,
            harness_only: false,
            harness: None,
        };
        if let Err(e) =
            crate::commands::sync::sync_one_project(&name, project_root, &sync_args, paths)
        {
            tracing::warn!(
                workspace = name.as_str(),
                error = %e,
                "session-start: project reconcile failed; printing directive anyway",
            );
        }
    }

    let entries = if paths.index_db.exists() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        crate::index::skills::tiered_entries_for_workspace(&conn, name.as_str())?
    } else {
        Vec::new()
    };
    let summary = crate::harness::routing::read_cached_long_summary(paths, &name);
    let directive = crate::harness::routing::build_directive(&entries, summary.as_deref());

    std::io::stdout().lock().write_all(directive.as_bytes())?;
    Ok(())
}

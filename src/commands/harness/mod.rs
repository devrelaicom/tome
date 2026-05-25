//! `commands::harness` — the per-project sync entry point invoked by
//! `tome workspace use` (Phase 4 / US1).
//!
//! Phase 4 / US1.b-3 lights up the real algorithm. `BindDeps` (the
//! shared call-shape for binding) doesn't carry the bound workspace
//! name — the binding step is what *establishes* it — so the CLI
//! wrapper threads the workspace name in via a separate parameter.
//! The orchestrator itself lives in [`crate::harness::sync::sync_project`];
//! this module is the thin seam.
//!
//! Sync runs **after** the bind step's lockfile has been released
//! (`bind_project` releases it before returning). The contract
//! `sync-algorithm.md` §Concurrency documents the rationale: the
//! filesystem writes that follow may touch directories outside
//! `<home>/.tome/` (a per-project `.claude/`, an `~/.codex/` global
//! config) and a slow project filesystem must not block every other
//! Tome command on the machine.

use std::path::Path;

use crate::error::TomeError;
use crate::harness::sync;
use crate::workspace::WorkspaceName;
use crate::workspace::binding::BindDeps;

pub use crate::harness::sync::SyncOutcome;

/// Sync every effective harness for `project_root` against the freshly-
/// bound `workspace_name`. Computes the effective harness list from
/// `<project_root>/.tome/config.toml` + the workspace's `settings.toml`
/// + the global `settings.toml`, then dispatches per-harness writes
///   (rules-file + MCP config).
///
/// `force` is forwarded to the orchestrator's clash-override path
/// (FR-501): a user-owned `tome` entry in a harness's MCP config will
/// be rewritten instead of returning [`TomeError::HarnessClash`].
pub fn sync_for_project_root(
    project_root: &Path,
    workspace_name: &WorkspaceName,
    deps: &BindDeps<'_>,
    force: bool,
) -> Result<SyncOutcome, TomeError> {
    let sync_deps = sync::build_deps(deps.paths, deps.home_root, workspace_name, force);
    sync::sync_project(project_root, &sync_deps)
}

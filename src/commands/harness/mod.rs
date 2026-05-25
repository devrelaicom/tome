//! Dispatcher for `tome harness <subcommand>` plus shared helpers.
//!
//! Phase 4 / US3.c-2 promotes this module from a single-function shim
//! to the full subcommand surface. The pre-existing
//! [`sync_for_project_root`] entry point (used by `tome workspace use`'s
//! binding flow) is preserved verbatim — `BindDeps`-flavoured callers
//! still go through it. The new public API is [`run`], which dispatches
//! the clap subcommand surface, and the per-subcommand modules
//! ([`bare`], [`list`], [`use_`], [`remove`], [`info`], [`sync`]).
//!
//! ## Resolving the project root
//!
//! Every subcommand other than `list <workspace>` / `info` may consult
//! the resolved scope's `project_root`. When the scope was resolved via
//! a project marker, this is the project dir; otherwise it is `None`
//! and the subcommand decides whether absence is fatal (sync / use
//! --scope project → error) or merely informational (bare / info →
//! `—` placeholder).
//!
//! ## ScopeProvider for `tome harness list`
//!
//! `harness list` (no arg) resolves the effective harness list which
//! may chase `[workspaces.<name>]` references. The production
//! [`ScopeProvider`] [`PathsScopeProvider`] reads each named
//! workspace's `settings.toml` from disk (under `paths.workspaces_dir`)
//! and returns its `harnesses` field verbatim. Missing workspace
//! settings files map to `UnknownWorkspace` per the trait contract;
//! the resolver then either succeeds (no `[workspaces.<name>]`
//! reference reached) or surfaces as exit 13 (`WorkspaceNotFound`).

pub mod bare;
pub mod info;
pub mod list;
pub mod remove;
pub mod sync;
pub mod use_;

use std::path::Path;

use crate::cli::{HarnessArgs, HarnessCommand};
use crate::error::{CompositionErrorKind, TomeError};
use crate::output::Mode;
use crate::paths::Paths;
use crate::settings::parser::parse_workspace;
use crate::settings::resolver::ScopeProvider;
use crate::workspace::binding::BindDeps;
use crate::workspace::{ResolvedScope, WorkspaceName};

pub use crate::harness::sync::SyncOutcome;

/// Sync every effective harness for `project_root` against the freshly-
/// bound `workspace_name`. Computes the effective harness list from
/// `<project_root>/.tome/config.toml` + the workspace's `settings.toml`
/// + the global `settings.toml`, then dispatches per-harness writes.
///
/// `force` is forwarded to the orchestrator's clash-override path
/// (FR-501).
pub fn sync_for_project_root(
    project_root: &Path,
    workspace_name: &WorkspaceName,
    deps: &BindDeps<'_>,
    force: bool,
) -> Result<SyncOutcome, TomeError> {
    let sync_deps =
        crate::harness::sync::build_deps(deps.paths, deps.home_root, workspace_name, force);
    crate::harness::sync::sync_project(project_root, &sync_deps)
}

/// Subcommand dispatcher invoked by `main.rs`.
pub fn run(args: HarnessArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    match args.command {
        None => bare::run(scope, &paths, mode),
        Some(HarnessCommand::List(a)) => list::run(a, scope, &paths, mode),
        Some(HarnessCommand::Use(a)) => use_::run(a, scope, &paths, mode),
        Some(HarnessCommand::Remove(a)) => remove::run(a, scope, &paths, mode),
        Some(HarnessCommand::Info(a)) => info::run(a, scope, &paths, mode),
        Some(HarnessCommand::Sync) => sync::run(scope, &paths, mode),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Production [`ScopeProvider`] backed by disk-resident workspace
/// settings files. Each `directly_declared_harnesses` call reads
/// `<root>/workspaces/<name>/settings.toml` and returns its `harnesses`
/// field verbatim.
pub(crate) struct PathsScopeProvider<'a> {
    paths: &'a Paths,
}

impl<'a> PathsScopeProvider<'a> {
    pub(crate) fn new(paths: &'a Paths) -> Self {
        Self { paths }
    }
}

impl ScopeProvider for PathsScopeProvider<'_> {
    fn directly_declared_harnesses(
        &self,
        name: &WorkspaceName,
    ) -> Result<Option<Vec<String>>, CompositionErrorKind> {
        let path = self.paths.workspace_settings_file(name);
        let body = match std::fs::read_to_string(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(CompositionErrorKind::UnknownWorkspace(
                    name.as_str().to_owned(),
                ));
            }
            Err(_) => {
                // Treat unreadable as unknown so the resolver maps to
                // exit 13 (`WorkspaceNotFound`) with a sensible message.
                return Err(CompositionErrorKind::UnknownWorkspace(
                    name.as_str().to_owned(),
                ));
            }
        };
        let ws = parse_workspace(&body)
            .map_err(|_| CompositionErrorKind::UnknownWorkspace(name.as_str().to_owned()))?;
        Ok(ws.harnesses)
    }
}

/// Resolve `$HOME` for harness-detect calls. Centralised so subcommands
/// don't sprinkle `std::env::var_os("HOME")` calls.
pub(crate) fn home_root() -> Result<std::path::PathBuf, TomeError> {
    std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| {
            TomeError::Io(std::io::Error::other(
                "HOME is not set — cannot probe harness detection paths",
            ))
        })
}

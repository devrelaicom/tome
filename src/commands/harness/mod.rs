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
//! [`ScopeProvider`] [`CentralDbScopeProvider`] consults the central
//! SQLite registry (`workspaces` table) to confirm workspace membership
//! and then reads the workspace's on-disk `settings.toml` (when present)
//! for the directly-declared harnesses list:
//!
//! * **Workspace exists, settings file present + parses** → `Ok(Some(list))`
//! * **Workspace exists, settings file absent** → `Ok(None)` (legal — no
//!   harnesses declared)
//! * **Workspace exists, file unreadable or unparsable** →
//!   `Err(SettingsReadFailure)` which maps to exit 70
//!   (`WorkspaceMalformed`) — distinct from "workspace doesn't exist"
//! * **Workspace not in central registry** → `Err(UnknownWorkspace)` which
//!   maps to exit 13 (`WorkspaceNotFound`).
//!
//! When the central DB has not yet been bootstrapped (no `index.db` file),
//! only the privileged `global` workspace is considered to exist. Any
//! other reference resolves to `UnknownWorkspace`.

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

/// Production [`ScopeProvider`] backed by the central SQLite registry
/// (the source of truth for workspace membership) and the on-disk
/// `<root>/workspaces/<name>/settings.toml` files (the source of truth
/// for the directly-declared harness list).
///
/// Three-way classification per the trait contract:
///
/// 1. **Workspace not in the registry** → `Err(UnknownWorkspace)`
///    (exit 13).
/// 2. **Workspace in the registry, settings file absent** →
///    `Ok(None)` — legal: the workspace exists but doesn't declare a
///    `harnesses` list. The resolver treats this as "no recursion shape
///    from this scope" per FR-449.
/// 3. **Workspace in the registry, settings file present** →
///    `Ok(Some(list))` with whatever the file declares. IO / parse
///    failures surface as `Err(SettingsReadFailure)` (exit 70) — distinct
///    from "unknown" so the user sees the malformed-state hint rather
///    than a misleading "workspace not found" message.
///
/// When the central DB has not been bootstrapped (no `index.db`), only
/// `WorkspaceName::global()` is considered to exist. The `global`
/// workspace is the bootstrap-seeded row in every DB; treating it as
/// always-present aligns with that invariant.
pub(crate) struct CentralDbScopeProvider<'a> {
    paths: &'a Paths,
}

impl<'a> CentralDbScopeProvider<'a> {
    pub(crate) fn new(paths: &'a Paths) -> Self {
        Self { paths }
    }

    /// Confirm `name` exists in the central `workspaces` table. Falls
    /// back to "only global is known" when the DB file is absent so a
    /// freshly-installed Tome still resolves `[global]` cleanly without
    /// requiring an initial bootstrap pass.
    fn workspace_is_registered(&self, name: &WorkspaceName) -> bool {
        // Bootstrap-not-yet shortcut: privileged `global` is always
        // considered present; everything else is unknown.
        if !self.paths.index_db.exists() {
            return name.as_str() == WorkspaceName::global().as_str();
        }
        let conn = match crate::index::open_read_only(&self.paths.index_db) {
            Ok(c) => c,
            // If we can't open read-only, the DB is in a broken state.
            // Treat the workspace as unknown — surfaces as exit 13 with
            // a hint pointing at `tome doctor`.
            Err(_) => return name.as_str() == WorkspaceName::global().as_str(),
        };
        conn.query_row(
            "SELECT 1 FROM workspaces WHERE name = ?1",
            rusqlite::params![name.as_str()],
            |_| Ok(()),
        )
        .is_ok()
    }
}

impl ScopeProvider for CentralDbScopeProvider<'_> {
    fn directly_declared_harnesses(
        &self,
        name: &WorkspaceName,
    ) -> Result<Option<Vec<String>>, CompositionErrorKind> {
        // 1. Membership check against the central registry.
        if !self.workspace_is_registered(name) {
            return Err(CompositionErrorKind::UnknownWorkspace(
                name.as_str().to_owned(),
            ));
        }

        // 2. Read the workspace's settings.toml. Absent = legal "no
        //    harnesses declared" → Ok(None). IO + parse failures =
        //    SettingsReadFailure → exit 70 via the From boundary.
        let path = self.paths.workspace_settings_file(name);
        let body = match std::fs::read_to_string(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(CompositionErrorKind::SettingsReadFailure(
                    name.as_str().to_owned(),
                    format!("read {}: {e}", path.display()),
                ));
            }
        };
        let ws = parse_workspace(&body).map_err(|e| {
            CompositionErrorKind::SettingsReadFailure(
                name.as_str().to_owned(),
                format!("parse {}: {e}", path.display()),
            )
        })?;
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

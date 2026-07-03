//! Dispatcher for `tome workspace <subcommand>`. Phase 4 / US2.a-1
//! widens the surface: `info` accepts an optional `<name>` argument and
//! exposes the new Phase 4 fields; `init` creates a named workspace in
//! the central registry; `list` lists every workspace with counts.

pub mod current;
pub mod info;
mod init;
pub mod list;
pub mod regen_summary;
pub mod remove;
pub mod rename;
pub mod use_;

use std::path::PathBuf;

use crate::cli::WorkspaceCommand;
use crate::error::TomeError;
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;
use crate::workspace::binding::{self, BindDeps, BindOutcome};
use crate::workspace::name::WorkspaceName;

pub fn run(
    cmd: WorkspaceCommand,
    global_workspace_flag: Option<&str>,
    scope: &ResolvedScope,
    mode: Mode,
) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    match cmd {
        // Read-only; the scope was already resolved (and membership-checked)
        // before dispatch, so `current` needs neither `paths` nor a DB read.
        WorkspaceCommand::Current => current::run(scope, mode),
        WorkspaceCommand::Info(args) => info::run(args, scope, &paths, mode),
        WorkspaceCommand::Init(args) => {
            let r = init::run(args, &paths, mode);
            // Anonymous action emit only on success (the workspace was created).
            // Failures are already captured by the app-boundary `tome.error`.
            emit_on_ok(&r, crate::telemetry::event::WorkspaceAction::Init);
            r
        }
        WorkspaceCommand::List(args) => list::run(args, scope, &paths, mode),
        WorkspaceCommand::Remove(args) => {
            let r = remove::run(args, &paths, mode);
            emit_on_ok(&r, crate::telemetry::event::WorkspaceAction::Remove);
            r
        }
        WorkspaceCommand::Rename(args) => {
            let r = rename::run(args, &paths, mode);
            emit_on_ok(&r, crate::telemetry::event::WorkspaceAction::Rename);
            r
        }
        WorkspaceCommand::RegenSummary(args) => regen_summary::run(args, scope, &paths, mode),
        // The Use arm threads the global `--workspace <name>` through
        // so it can emit a `tracing::debug!` when both the flag and the
        // positional `<name>` are set. The positional always wins; the
        // flag is informational here (documented on the
        // `WorkspaceCommand::Use` doc comment in `src/cli.rs`).
        WorkspaceCommand::Use(args) => {
            let r = use_::run(args, global_workspace_flag, &paths, mode);
            emit_on_ok(&r, crate::telemetry::event::WorkspaceAction::Use);
            r
        }
    }
}

/// Emit a `tome.workspace_action` telemetry event ONLY when the verb
/// succeeded (the mutation actually happened). One infallible `enqueue`;
/// never alters control flow, exit code, or output.
fn emit_on_ok(result: &Result<(), TomeError>, action: crate::telemetry::event::WorkspaceAction) {
    if result.is_ok() {
        crate::telemetry::emit(crate::telemetry::event::WorkspaceActionEvent { action });
    }
}

/// Resolve the user's home directory for the dangerous-cwd check.
/// Errors with [`TomeError::Io`] if `$HOME` is unset — same shape as
/// [`Paths::resolve`].
fn resolve_home() -> Result<PathBuf, TomeError> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        TomeError::Io(std::io::Error::other(
            "HOME is not set — cannot decide whether the current directory is the user's home",
        ))
    })
}

/// Run the dangerous-cwd guard against `$CWD` (refuses `$HOME` / `/` without
/// `--force`, exit 2). Extracted so the `use --create` / `init --bind`
/// callers can run it BEFORE the create step, making those flows
/// all-or-nothing: a refusal creates nothing, so no orphan workspace is left
/// behind. `bind_cwd_and_sync` runs the same check again as cheap,
/// idempotent defense-in-depth (a canonicalise + path comparison).
///
/// A no-op when `force` is set — mirroring the wrapper's `!args.force` gate.
pub(crate) fn guard_dangerous_cwd(force: bool) -> Result<(), TomeError> {
    if force {
        return Ok(());
    }
    let cwd = std::env::current_dir().map_err(TomeError::Io)?;
    let home_root = resolve_home()?;
    binding::is_project_root_acceptable(&cwd, &home_root)
}

/// The single "bind `$CWD` to `name` + harness sync" sequence shared by
/// `workspace use <name>`, `workspace use --create`, and
/// `workspace init --bind` (issue #321). Factoring it here means all three
/// entry points share one dangerous-cwd guard, one bind algorithm, and one
/// harness-sync seam — the mirror of the `reconcile_<sink>` SSOT discipline.
///
/// `force` bypasses the home/`/` refusal (forwarded to the orchestrator's
/// clash-override path too). Returns the [`BindOutcome`] with its `sync`
/// field populated; the caller sets `created` if it ran a create step.
pub(crate) fn bind_cwd_and_sync(
    name: WorkspaceName,
    force: bool,
    paths: &Paths,
) -> Result<BindOutcome, TomeError> {
    let cwd = std::env::current_dir().map_err(TomeError::Io)?;
    let home_root = resolve_home()?;

    if !force {
        binding::is_project_root_acceptable(&cwd, &home_root)?;
    }

    let deps = BindDeps {
        paths,
        home_root: &home_root,
    };

    let mut outcome = binding::bind_project(&cwd, name, force, &deps)?;

    // Phase B of the bind algorithm: run the harness sync orchestrator
    // against the freshly-bound workspace name. `--force` is forwarded so
    // user-owned `tome` MCP entries get rewritten instead of returning
    // HarnessClash (exit 19).
    let sync_outcome = crate::commands::harness::sync_for_project_root(
        &outcome.project_root,
        &outcome.workspace,
        &deps,
        force,
    )?;
    outcome.sync = Some(sync_outcome);

    Ok(outcome)
}

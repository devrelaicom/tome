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

use crate::cli::WorkspaceCommand;
use crate::error::TomeError;
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

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
        WorkspaceCommand::List(args) => list::run(args, &paths, mode),
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

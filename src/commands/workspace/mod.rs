//! Dispatcher for `tome workspace <subcommand>`. Phase 4 / US2.a-1
//! widens the surface: `info` accepts an optional `<name>` argument and
//! exposes the new Phase 4 fields; `init` creates a named workspace in
//! the central registry; `list` lists every workspace with counts.

pub mod info;
mod init;
pub mod list;
pub mod regen_summary;
pub mod remove;
pub mod rename;
pub mod sync;
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
        WorkspaceCommand::Info(args) => info::run(args, scope, &paths, mode),
        WorkspaceCommand::Init(args) => init::run(args, &paths, mode),
        WorkspaceCommand::List(args) => list::run(args, &paths, mode),
        WorkspaceCommand::Remove(args) => remove::run(args, &paths, mode),
        WorkspaceCommand::Rename(args) => rename::run(args, &paths, mode),
        WorkspaceCommand::RegenSummary(args) => regen_summary::run(args, scope, &paths, mode),
        WorkspaceCommand::Sync(args) => sync::run(args, &paths, mode),
        // The Use arm threads the global `--workspace <name>` through
        // so it can emit a `tracing::debug!` when both the flag and the
        // positional `<name>` are set. The positional always wins; the
        // flag is informational here (documented on the
        // `WorkspaceCommand::Use` doc comment in `src/cli.rs`).
        WorkspaceCommand::Use(args) => use_::run(args, global_workspace_flag, &paths, mode),
    }
}

//! Dispatcher for `tome workspace <subcommand>`. Adds `info`
//! (US2.a, read-only), `init` (US2.b, atomic creation), and `use`
//! (US1.a, project binding).

pub mod info;
mod init;
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
        WorkspaceCommand::Info => info::run(scope, &paths, mode),
        WorkspaceCommand::Init(args) => init::run(args, &paths, mode),
        // The Use arm threads the global `--workspace <name>` through
        // so it can emit a `tracing::debug!` when both the flag and the
        // positional `<name>` are set. The positional always wins; the
        // flag is informational here (documented on the
        // `WorkspaceCommand::Use` doc comment in `src/cli.rs`).
        WorkspaceCommand::Use(args) => use_::run(args, global_workspace_flag, &paths, mode),
    }
}

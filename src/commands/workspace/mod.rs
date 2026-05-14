//! Dispatcher for `tome workspace <subcommand>`. Adds `info`
//! (US2.a, read-only) and `init` (US2.b, atomic creation).

pub mod info;
mod init;

use crate::cli::WorkspaceCommand;
use crate::error::TomeError;
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

pub fn run(cmd: WorkspaceCommand, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    match cmd {
        WorkspaceCommand::Info => info::run(scope, &paths, mode),
        WorkspaceCommand::Init(args) => init::run(args, &paths, mode),
    }
}

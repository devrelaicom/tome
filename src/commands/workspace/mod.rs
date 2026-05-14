//! Dispatcher for `tome workspace <subcommand>`.
//!
//! Slice US2.a of Phase 3 ships `info`. Slice US2.b will add `init`.

pub mod info;

use crate::cli::WorkspaceCommand;
use crate::error::TomeError;
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

pub fn run(cmd: WorkspaceCommand, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    match cmd {
        WorkspaceCommand::Info => info::run(scope, &paths, mode),
    }
}

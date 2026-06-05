//! Dispatcher for `tome skill <subcommand>` — the third artifact level
//! (`create` / `convert` / `lint`). New in Phase 8; skills otherwise remain
//! surfaced read-only via `tome plugin show`.

mod convert;
mod create;
mod lint;

use crate::cli::SkillCommand;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(cmd: SkillCommand, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    match cmd {
        SkillCommand::Create(args) => create::run(args, scope, mode),
        SkillCommand::Convert(args) => convert::run(args, scope, mode),
        SkillCommand::Lint(args) => lint::run(args, scope, mode),
    }
}

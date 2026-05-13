//! Dispatcher for `tome catalog <subcommand>` plus shared helpers.

mod add;
mod list;
mod remove;
mod show;
mod source;
pub mod update;

use crate::cli::CatalogCommand;
use crate::error::TomeError;
use crate::output::Mode;

pub fn run(cmd: CatalogCommand, mode: Mode) -> Result<(), TomeError> {
    match cmd {
        CatalogCommand::Add(args) => add::run(args, mode),
        CatalogCommand::Remove(args) => remove::run(args, mode),
        CatalogCommand::List(args) => list::run(args, mode),
        CatalogCommand::Update(args) => update::run(args, mode),
        CatalogCommand::Show(args) => show::run(args, mode),
    }
}

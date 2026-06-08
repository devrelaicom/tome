//! Dispatcher for `tome catalog <subcommand>` plus shared helpers.

mod add;
mod convert;
mod create;
mod lint;
mod list;
// `pub` so the `#[doc(hidden)]` test-injection seam `AFTER_PRELOCK_READ_HOOK`
// (F-REMOVE-TOCTOU) is reachable from `tests/`; the surface is doc-hidden.
pub mod remove;
mod show;
pub(crate) mod source;
pub mod update;

use crate::cli::CatalogCommand;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(cmd: CatalogCommand, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    match cmd {
        CatalogCommand::Add(args) => add::run(args, scope, mode),
        CatalogCommand::Remove(args) => remove::run(args, scope, mode),
        CatalogCommand::List(args) => list::run(args, scope, mode),
        CatalogCommand::Update(args) => update::run(args, scope, mode),
        CatalogCommand::Show(args) => show::run(args, scope, mode),
        CatalogCommand::Create(args) => create::run(args, scope, mode),
        CatalogCommand::Convert(args) => convert::run(args, scope, mode),
        CatalogCommand::Lint(args) => lint::run(args, scope, mode),
    }
}

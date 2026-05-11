//! Dispatcher for `tome catalog <subcommand>`. Subcommand bodies arrive in
//! Phase 3 (User Story 1); for now each variant returns an `unimplemented!()`
//! so the binary builds end-to-end and Phase 2's `main` dispatch loop is
//! exercised.

use crate::cli::{
    CatalogAddArgs, CatalogCommand, CatalogListArgs, CatalogRemoveArgs, CatalogShowArgs,
    CatalogUpdateArgs,
};
use crate::error::TomeError;
use crate::output::Mode;

pub fn run(cmd: CatalogCommand, _mode: Mode) -> Result<(), TomeError> {
    match cmd {
        CatalogCommand::Add(args) => add(args),
        CatalogCommand::Remove(args) => remove(args),
        CatalogCommand::List(args) => list(args),
        CatalogCommand::Update(args) => update(args),
        CatalogCommand::Show(args) => show(args),
    }
}

fn add(_args: CatalogAddArgs) -> Result<(), TomeError> {
    unimplemented!("tome catalog add — implemented in Phase 3 (US1)")
}

fn remove(_args: CatalogRemoveArgs) -> Result<(), TomeError> {
    unimplemented!("tome catalog remove — implemented in Phase 3 (US1)")
}

fn list(_args: CatalogListArgs) -> Result<(), TomeError> {
    unimplemented!("tome catalog list — implemented in Phase 3 (US1)")
}

fn update(_args: CatalogUpdateArgs) -> Result<(), TomeError> {
    unimplemented!("tome catalog update — implemented in Phase 3 (US1)")
}

fn show(_args: CatalogShowArgs) -> Result<(), TomeError> {
    unimplemented!("tome catalog show — implemented in Phase 3 (US1)")
}

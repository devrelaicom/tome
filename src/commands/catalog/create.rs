//! `tome catalog create <NAME>` — scaffold a new catalog from a template.
//! Thin arg→`authoring::create`→emit wrapper; lands in Phase 8 / US4.

use crate::cli::CatalogCreateArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(_args: CatalogCreateArgs, _scope: &ResolvedScope, _mode: Mode) -> Result<(), TomeError> {
    unimplemented!("`tome catalog create` lands in Phase 8 / US4")
}

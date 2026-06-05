//! `tome catalog convert <SOURCE>` — convert a Claude Code marketplace into a
//! native Tome catalog. Thin arg→`authoring::convert`→emit wrapper; lands in
//! Phase 8 / US2.

use crate::cli::ConvertArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(_args: ConvertArgs, _scope: &ResolvedScope, _mode: Mode) -> Result<(), TomeError> {
    unimplemented!("`tome catalog convert` lands in Phase 8 / US2")
}

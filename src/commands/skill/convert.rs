//! `tome skill convert <SOURCE>` — convert a foreign native `SKILL.md` into a
//! native Tome skill. Thin arg→`authoring::convert`→emit wrapper; lands in
//! Phase 8 / US2.

use crate::cli::ConvertArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(_args: ConvertArgs, _scope: &ResolvedScope, _mode: Mode) -> Result<(), TomeError> {
    unimplemented!("`tome skill convert` lands in Phase 8 / US2")
}

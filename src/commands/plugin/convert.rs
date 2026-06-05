//! `tome plugin convert <SOURCE>` — convert a Claude Code plugin or a Codex
//! project into a native Tome plugin. Thin arg→`authoring::convert`→emit
//! wrapper; lands in Phase 8 / US2.

use crate::cli::ConvertArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(_args: ConvertArgs, _scope: &ResolvedScope, _mode: Mode) -> Result<(), TomeError> {
    unimplemented!("`tome plugin convert` lands in Phase 8 / US2")
}

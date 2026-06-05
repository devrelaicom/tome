//! `tome skill lint <SOURCE>` — validate a Tome skill for CI. Thin
//! arg→`authoring::lint`→emit wrapper; lands in Phase 8 / US3.

use crate::cli::LintArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(_args: LintArgs, _scope: &ResolvedScope, _mode: Mode) -> Result<(), TomeError> {
    unimplemented!("`tome skill lint` lands in Phase 8 / US3")
}

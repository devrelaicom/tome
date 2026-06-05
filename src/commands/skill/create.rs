//! `tome skill create <NAME>` — scaffold a new skill (wrapped in a minimal
//! plugin by default; `--bare` for a naked one). Thin arg→`authoring::create`
//! →emit wrapper; lands in Phase 8 / US4.

use crate::cli::SkillCreateArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(_args: SkillCreateArgs, _scope: &ResolvedScope, _mode: Mode) -> Result<(), TomeError> {
    unimplemented!("`tome skill create` lands in Phase 8 / US4")
}

//! `tome skill convert <SOURCE>` — convert a native `SKILL.md` (Claude Code,
//! Cursor, OpenCode, Cline, or generic Agent Skills) into a native Tome skill.
//! Thin shim over the shared [`crate::commands::convert`] wrapper at the skill
//! level.

use crate::authoring::detect::ArtifactLevel;
use crate::cli::ConvertArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(args: ConvertArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    crate::commands::convert::run(args, scope, mode, ArtifactLevel::Skill)
}

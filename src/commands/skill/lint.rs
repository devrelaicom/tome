//! `tome skill lint <SOURCE>` — validate a native Tome skill for CI. Thin shim
//! over the shared [`crate::commands::lint`] wrapper.

use crate::authoring::detect::ArtifactLevel;
use crate::cli::LintArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(args: LintArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    crate::commands::lint::run(args, scope, mode, ArtifactLevel::Skill)
}

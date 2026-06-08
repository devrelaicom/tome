//! `tome catalog convert <SOURCE>` — convert a Claude Code marketplace into a
//! native Tome catalog (marketplace recursion: relative-path plugins vendored
//! inline all-or-nothing, remote-source plugins warned-and-skipped). Thin shim
//! over the shared [`crate::commands::convert`] wrapper at the catalog level.

use crate::authoring::detect::ArtifactLevel;
use crate::cli::ConvertArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(args: ConvertArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    crate::commands::convert::run(args, scope, mode, ArtifactLevel::Catalog)
}

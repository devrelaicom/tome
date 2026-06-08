//! `tome catalog convert <SOURCE>` — convert a Claude Code marketplace into a
//! native Tome catalog. Thin shim over the shared [`crate::commands::convert`]
//! wrapper at the catalog level. Catalog-level import (marketplace recursion +
//! all-or-nothing staging) lands in a later US2 slice; until then the pipeline
//! returns a clear deferral error rather than panicking.

use crate::authoring::detect::ArtifactLevel;
use crate::cli::ConvertArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(args: ConvertArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    crate::commands::convert::run(args, scope, mode, ArtifactLevel::Catalog)
}

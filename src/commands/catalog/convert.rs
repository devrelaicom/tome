//! `tome catalog convert <SOURCE>` — convert a Claude Code marketplace into a
//! native Tome catalog (marketplace recursion: relative-path plugins vendored
//! inline all-or-nothing, remote-source plugins warned-and-skipped). Thin shim
//! over the shared [`crate::commands::convert`] wrapper at the catalog level.

use crate::authoring::detect::ArtifactLevel;
use crate::cli::CatalogConvertArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(args: CatalogConvertArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    // Only `catalog convert` recurses into a marketplace's remote-source
    // plugins, so it is the sole surface that carries `--no-fetch`.
    crate::commands::convert::run(
        args.common,
        args.no_fetch,
        scope,
        mode,
        ArtifactLevel::Catalog,
    )
}

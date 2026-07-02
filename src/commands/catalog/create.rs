//! `tome catalog create <NAME>` — scaffold a new catalog from a template. Thin
//! shim over the shared [`crate::commands::create`] wrapper at the catalog
//! level (no `--into`/`--bare`/`--plugin-name`).

use crate::authoring::detect::ArtifactLevel;
use crate::cli::CatalogCreateArgs;
use crate::commands::create::CreateRequest;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(args: CatalogCreateArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    crate::commands::create::run(
        CreateRequest {
            level: ArtifactLevel::Catalog,
            name: args.name,
            template: args.template,
            output: args.output,
            into: None,
            force: args.force,
            bare: false,
            plugin_name: None,
            description: args.description,
            author: args.author,
            dry_run: args.dry_run,
        },
        scope,
        mode,
    )
}

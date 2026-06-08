//! `tome plugin create <NAME>` — scaffold a new plugin from a template. Thin
//! shim over the shared [`crate::commands::create`] wrapper at the plugin level
//! (`--into` injects into an existing catalog; no `--bare`/`--plugin-name`).

use crate::authoring::detect::ArtifactLevel;
use crate::cli::PluginCreateArgs;
use crate::commands::create::CreateRequest;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(args: PluginCreateArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    crate::commands::create::run(
        CreateRequest {
            level: ArtifactLevel::Plugin,
            name: args.name,
            template: args.template,
            output: args.output,
            into: args.into,
            force: args.force,
            bare: false,
            plugin_name: None,
        },
        scope,
        mode,
    )
}

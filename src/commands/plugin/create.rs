//! `tome plugin create <NAME>` — scaffold a new plugin from a template. Thin
//! shim over the shared [`crate::commands::create`] wrapper at the plugin level
//! (`--into` injects into an existing catalog; no `--bare`/`--plugin-name`).
//! `--kind` selects the component scaffolded inside the plugin (default: skill).

use crate::authoring::detect::ArtifactLevel;
use crate::authoring::scaffold::ScaffoldComponent;
use crate::cli::{PluginCreateArgs, ScaffoldKindArg};
use crate::commands::create::CreateRequest;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(args: PluginCreateArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let component = match args.kind {
        None | Some(ScaffoldKindArg::Skill) => ScaffoldComponent::Skill,
        Some(ScaffoldKindArg::Command) => ScaffoldComponent::Command,
        Some(ScaffoldKindArg::Agent) => ScaffoldComponent::Agent,
        Some(ScaffoldKindArg::Hooks) => ScaffoldComponent::Hooks,
        Some(ScaffoldKindArg::Mcp) => ScaffoldComponent::Mcp,
    };
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
            description: args.description,
            author: args.author,
            dry_run: args.dry_run,
            component,
        },
        scope,
        mode,
    )
}

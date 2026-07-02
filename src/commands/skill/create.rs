//! `tome skill create <NAME>` — scaffold a new skill (wrapped in a minimal
//! plugin by default; `--bare` for a naked one). Thin shim over the shared
//! [`crate::commands::create`] wrapper at the skill level.

use crate::authoring::detect::ArtifactLevel;
use crate::cli::SkillCreateArgs;
use crate::commands::create::CreateRequest;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(args: SkillCreateArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    crate::commands::create::run(
        CreateRequest {
            level: ArtifactLevel::Skill,
            name: args.name,
            template: args.template,
            output: args.output,
            into: args.into,
            force: args.force,
            bare: args.bare,
            plugin_name: args.plugin_name,
            description: args.description,
            author: args.author,
            dry_run: args.dry_run,
        },
        scope,
        mode,
    )
}

//! `tome plugin create <NAME>` — scaffold a new plugin from a template.
//! Thin arg→`authoring::create`→emit wrapper; lands in Phase 8 / US4.

use crate::cli::PluginCreateArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(_args: PluginCreateArgs, _scope: &ResolvedScope, _mode: Mode) -> Result<(), TomeError> {
    unimplemented!("`tome plugin create` lands in Phase 8 / US4")
}

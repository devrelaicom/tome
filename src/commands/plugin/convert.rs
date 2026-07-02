//! `tome plugin convert <SOURCE>` — convert a Claude Code plugin (or, later, a
//! Codex project) into a native Tome plugin. Thin shim over the shared
//! [`crate::commands::convert`] wrapper at the plugin level.

use crate::authoring::detect::ArtifactLevel;
use crate::cli::ConvertArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

pub fn run(args: ConvertArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    // A single-plugin conversion has no remote-plugin fan-out, so `--no-fetch`
    // is not offered here — always fetch (`no_fetch = false`).
    crate::commands::convert::run(args, false, scope, mode, ArtifactLevel::Plugin)
}

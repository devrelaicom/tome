//! Thin wrapper around `tome::mcp::run`. The actual server lives in
//! `src/mcp/`; `commands::mcp` exists for dispatch symmetry with the
//! other top-level commands.
//!
//! Unlike every other command, this one does not honour `--json` at the
//! top level. The MCP protocol itself is the structured output; `--json`
//! would muddle the stdio transport channel (FR-221).

use crate::cli::McpArgs;
use crate::error::TomeError;
use crate::mcp;
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

pub fn run(_args: McpArgs, scope: &ResolvedScope, _mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    mcp::run(scope, &paths)
}

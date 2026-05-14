//! `tome mcp` — stdio MCP server scaffolding.
//!
//! Phase 3 / Foundational F8 lands the bones: a sync entry point, the
//! tokio runtime constructor, the JSON-lines file appender + rotation,
//! and the FR-110 startup pre-flight that returns a loaded embedder
//! handle. The actual `rmcp::ServerHandler` impl + tool registration
//! lands in US1.
//!
//! ### Sync boundary
//!
//! This module is the one place in `src/` where `async fn`, `.await`,
//! and `tokio::` are allowed. The structural test
//! [`tests/sync_boundary.rs`](../../tests/sync_boundary.rs) exempts every
//! file under `src/mcp/` and fails the build if any other module reaches
//! for the async runtime.
//!
//! ### Why a file log
//!
//! Per [`contracts/log-format.md`] and [`contracts/mcp-server.md`]:
//! stdout is the MCP protocol channel (FR-221), stderr is reserved for
//! fatal startup errors only (FR-222). Diagnostics go to
//! `${XDG_STATE_HOME}/tome/mcp.log`. Each startup rotates the log if
//! the existing file exceeds 10 MiB.

pub mod log;
pub mod preflight;
pub mod runtime;

pub use preflight::EmbedderHandle;

use crate::error::TomeError;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

/// Sync entry point. The wiring sequence — runtime build, log subscriber
/// install, pre-flight, `rmcp::serve_server` via `runtime.block_on` —
/// lands in US1 (T076). F8 ships only the surfaces.
pub fn run(_scope: &ResolvedScope, _paths: &Paths) -> Result<(), TomeError> {
    Err(TomeError::McpStartupFailed {
        reason: "mcp server scaffolding only; US1 wires the server loop".into(),
    })
}

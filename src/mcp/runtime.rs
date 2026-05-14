//! Single-threaded tokio runtime backing the MCP server.
//!
//! Research §R-2 pins the runtime to `tokio::runtime::Builder::new_current_thread`
//! with the minimal feature set (`rt`, `macros`, `io-std`, `sync`,
//! `signal`, `time`). The server serves one client at a time and tool
//! calls block on synchronous embedder/reranker inference; the
//! multi-threaded runtime would add ~500 KB of binary weight for no
//! concurrency win.
//!
//! Every async surface inside `src/mcp/` runs on this runtime. The CLI
//! dispatch in `commands::mcp::run` (lands in US1) hands off through
//! `runtime.block_on(...)`; the sync boundary stays inside this module.

use tokio::runtime::{Builder, Runtime};

use crate::error::TomeError;

/// Construct the runtime. Returns a fresh `Runtime` owned by the caller.
/// The MCP entry point (`mcp::run`) builds one runtime per invocation
/// and drops it on shutdown.
pub fn build_runtime() -> Result<Runtime, TomeError> {
    Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
        .map_err(|e| TomeError::McpStartupFailed {
            reason: format!("tokio runtime build failed: {e}"),
        })
}

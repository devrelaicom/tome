//! `tome mcp` — stdio MCP server.
//!
//! Phase 3 / F8 landed the scaffolding (runtime / log / preflight); US1
//! fills in the actual server loop. Two tools are advertised:
//! [`tools::search_skills`] and [`tools::get_skill`].
//!
//! ### Sync boundary
//!
//! This module is the one place in `src/` where `async fn`, `.await`,
//! and `tokio::` are allowed. The structural test
//! [`tests/sync_boundary.rs`](../../tests/sync_boundary.rs) exempts
//! every file under `src/mcp/` and fails the build if any other module
//! reaches for the async runtime.
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
pub mod server;
pub mod state;
pub mod tools;

use std::sync::Arc;

use tokio::sync::OnceCell;
use tracing::info;

pub use preflight::EmbedderHandle;

use crate::error::TomeError;
use crate::paths::Paths;
use crate::workspace::{ResolvedScope, Scope};

/// Sync entry point. Builds the tokio runtime, installs the MCP file
/// log subscriber, runs the FR-110 pre-flight, constructs the server,
/// and drives `rmcp::serve_server` through `runtime.block_on(...)`.
///
/// Returns `Ok(())` on clean shutdown (stdin closed by the harness).
/// SIGINT triggers graceful shutdown and surfaces `TomeError::Interrupted`
/// (exit 8). Pre-flight failures propagate as their specific Phase
/// 1/2/3 `TomeError` variants — `main.rs` maps each to the right exit
/// code per `contracts/exit-codes-p3.md` §"Specific-over-generic".
pub fn run(scope: &ResolvedScope, paths: &Paths) -> Result<(), TomeError> {
    // Open the file log appender before anything else can fail. If this
    // errors, the harness sees a TomeError on stderr instead of a
    // silent log.
    let log_file = log::open_appender(paths)?;
    log::init_subscriber(log_file)?;

    let runtime = runtime::build_runtime()?;

    runtime.block_on(async {
        // Pre-flight is synchronous (model load, index open, SHA-256
        // are all sync). Run it on the blocking pool so the
        // single-threaded reactor isn't held up by the hash step.
        let scope_clone = scope.clone();
        let paths_clone = paths.clone();
        let handle =
            tokio::task::spawn_blocking(move || preflight::run(&scope_clone, &paths_clone))
                .await
                .map_err(|e| TomeError::McpStartupFailed {
                    reason: format!("preflight task join: {e}"),
                })??;

        let scope_label = match &scope.scope {
            Scope::Global => "global",
            Scope::Workspace(_) => "workspace",
        };
        let workspace_path = match &scope.scope {
            Scope::Workspace(p) => Some(p.display().to_string()),
            Scope::Global => None,
        };
        info!(
            target: "tome::mcp::server",
            scope = scope_label,
            workspace = workspace_path.as_deref(),
            embedder = handle.embedder_entry.name,
            reranker_lazy = true,
            "startup ok",
        );

        let state = Arc::new(state::McpState {
            embedder: Arc::from(handle.embedder),
            reranker: OnceCell::new(),
            scope: scope.clone(),
            paths: paths.clone(),
            embedder_entry: handle.embedder_entry,
            reranker_entry: handle.reranker_entry,
        });

        let server = server::Server::new(state);

        let running = rmcp::serve_server(server, rmcp::transport::stdio())
            .await
            .map_err(|e| TomeError::McpStartupFailed {
                reason: format!("rmcp serve_server: {e}"),
            })?;

        let cancel_token = running.cancellation_token();

        tokio::select! {
            res = running.waiting() => {
                match res {
                    Ok(reason) => {
                        info!(
                            target: "tome::mcp::server",
                            ?reason,
                            in_flight = 0,
                            "graceful shutdown",
                        );
                        Ok(())
                    }
                    Err(e) => Err(TomeError::McpProtocolIo {
                        source: std::io::Error::other(format!("rmcp serve join: {e}")),
                    }),
                }
            }
            _ = tokio::signal::ctrl_c() => {
                cancel_token.cancel();
                info!(
                    target: "tome::mcp::server",
                    signal = "SIGINT",
                    in_flight = 0,
                    "graceful shutdown",
                );
                Err(TomeError::Interrupted)
            }
        }
    })
}

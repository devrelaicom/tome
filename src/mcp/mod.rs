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
use std::time::Duration;

use tokio::sync::OnceCell;
use tracing::{error, info};

pub use preflight::EmbedderHandle;

use crate::catalog::git::scrub_to_string;
use crate::error::TomeError;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

/// Graceful-shutdown deadline per `contracts/mcp-server.md` §"Signal
/// handling" step 2. After this elapses the cancellation token has been
/// triggered but in-flight tool calls haven't finished — log a "hard
/// shutdown" event and return.
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

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
            match tokio::task::spawn_blocking(move || preflight::run(&scope_clone, &paths_clone))
                .await
                .map_err(|e| TomeError::McpStartupFailed {
                    reason: format!("preflight task join: {e}"),
                })? {
                Ok(h) => h,
                Err(e) => {
                    // FR-M-LOG-4 / log-format.md §Event taxonomy:
                    // pre-flight failures emit one `error` log line before
                    // the process exits, so operators see what blocked
                    // startup in mcp.log (not just stderr).
                    error!(
                        target: "tome::mcp::preflight",
                        check = e.category(),
                        error = %scrub_to_string(e.to_string().as_bytes()),
                        "pre-flight check failed",
                    );
                    return Err(e);
                }
            };

        let scope_label = if scope.scope.is_global() {
            "global"
        } else {
            "workspace"
        };
        let workspace_path = scope
            .project_root
            .as_ref()
            .map(|p| scrub_to_string(p.display().to_string().as_bytes()));
        info!(
            target: "tome::mcp::server",
            scope = scope_label,
            workspace_name = scope.scope.name().as_str(),
            workspace_path = workspace_path.as_deref(),
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

        // `RunningService::waiting()` consumes `self`, so pin the
        // resulting future once and poll the same handle from both
        // arms of the select + the post-signal timeout below.
        let waiter = running.waiting();
        tokio::pin!(waiter);

        // FR-M-MCP-1 / mcp-server.md §Signal handling: both SIGINT and
        // SIGTERM trigger graceful shutdown. SIGTERM is Unix-only —
        // Windows lacks the concept; the future stays `pending()` so
        // the `select!` arm fires only on Ctrl-C.
        let triggered_signal = wait_for_shutdown_signal();
        tokio::pin!(triggered_signal);

        tokio::select! {
            res = &mut waiter => {
                match res {
                    Ok(reason) => {
                        info!(
                            target: "tome::mcp::server",
                            signal = "stdin_closed",
                            reason = %scrub_to_string(format!("{reason:?}").as_bytes()),
                            in_flight = 0,
                            "graceful shutdown",
                        );
                        Ok(())
                    }
                    Err(e) => {
                        // FR-M-LOG-2: log-format.md §Event taxonomy
                        // requires an `error`-level "hard shutdown"
                        // event with a `reason` field. Without it
                        // operators see the process exit but get no
                        // clue why.
                        error!(
                            target: "tome::mcp::server",
                            reason = %scrub_to_string(format!("rmcp serve join: {e}").as_bytes()),
                            "hard shutdown",
                        );
                        Err(TomeError::McpProtocolIo {
                            source: std::io::Error::other(format!("rmcp serve join: {e}")),
                        })
                    }
                }
            }
            signal = &mut triggered_signal => {
                cancel_token.cancel();
                // FR-M-MCP-2 / mcp-server.md §Signal handling step 2:
                // give in-flight tool calls up to 5 s to finish after
                // cancellation. If they don't, log "hard shutdown" and
                // return anyway — the runtime drop on the outer scope
                // tears the rest down.
                let in_flight_finished = tokio::time::timeout(
                    GRACEFUL_SHUTDOWN_TIMEOUT,
                    &mut waiter,
                )
                .await;
                match in_flight_finished {
                    Ok(_) => {
                        info!(
                            target: "tome::mcp::server",
                            signal = signal,
                            in_flight = 0,
                            "graceful shutdown",
                        );
                    }
                    Err(_) => {
                        error!(
                            target: "tome::mcp::server",
                            signal = signal,
                            reason = "graceful shutdown timed out after 5s",
                            "hard shutdown",
                        );
                    }
                }
                Err(TomeError::Interrupted)
            }
        }
    })
}

/// Resolve the OS shutdown signal future, returning the contract's
/// `signal` field literal (`"SIGINT"` / `"SIGTERM"`) so the log event
/// uses the canonical string. SIGTERM is Unix-only; on Windows the
/// future stays `pending()` so the `select!` arm fires only on Ctrl-C.
async fn wait_for_shutdown_signal() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                // Falling back to SIGINT-only if SIGTERM registration
                // fails is safer than aborting the server — keep the
                // service up.
                tokio::signal::ctrl_c().await.ok();
                return "SIGINT";
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => "SIGINT",
            _ = term.recv() => "SIGTERM",
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.ok();
        "SIGINT"
    }
}

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
pub mod prompt_collision;
pub mod prompt_name;
pub mod prompts;
pub mod runtime;
pub mod server;
pub mod state;
pub mod substitution_helpers;
pub mod tool_description;
pub mod tools;

use std::sync::Arc;
use std::time::Duration;

/// Slash-command prefix that Claude Code (and harness-compatible MCP
/// hosts) renders for a Tome MCP prompt called `<name>`. Single source of
/// truth; `tome doctor` consumes it to render the `Resolved prompts:`
/// section per `contracts/doctor-extensions-p5.md`. Changing the MCP
/// server name (currently `tome` per `rmcp` server descriptor) requires
/// changing both ends in lockstep.
pub const MCP_SLASH_PREFIX: &str = "/mcp__tome__";

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
pub fn run(
    scope: &ResolvedScope,
    paths: &Paths,
    host_harness: Option<String>,
) -> Result<(), TomeError> {
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

        // Phase 5 / US1.b: build the prompts registry from the
        // resolved workspace's enabled-and-user-invocable entries.
        // Sync work (rusqlite open + frontmatter parses) goes through
        // `spawn_blocking` per the sync-boundary discipline.
        let registry_scope = scope.clone();
        let registry_paths = paths.clone();
        let prompt_registry = match tokio::task::spawn_blocking(move || {
            build_prompt_registry(&registry_scope, &registry_paths)
        })
        .await
        .map_err(|e| TomeError::McpStartupFailed {
            reason: format!("prompt-registry task join: {e}"),
        })? {
            Ok(reg) => reg,
            Err(e) => {
                error!(
                    target: "tome::mcp::prompts",
                    error = %scrub_to_string(e.to_string().as_bytes()),
                    "prompt-registry build failed",
                );
                return Err(e);
            }
        };
        info!(
            target: "tome::mcp::prompts",
            prompt_count = prompt_registry.by_name.len(),
            collision_count = prompt_registry.collisions.len(),
            "prompt registry built",
        );

        let state = Arc::new(state::McpState {
            embedder: Arc::from(handle.embedder),
            reranker: OnceCell::new(),
            scope: scope.clone(),
            paths: paths.clone(),
            embedder_entry: handle.embedder_entry,
            reranker_entry: handle.reranker_entry,
            prompt_registry: Arc::new(prompt_registry),
            host_harness: host_harness.clone(),
        });

        let mut server = server::Server::new(state);

        // FR-425 + NFR-103: compose the `search_skills` tool
        // description from the fixed scaffold + the workspace's
        // cached `[summaries].short` (if any). One-shot synchronous
        // file read at startup — no in-process summariser invocation.
        let composed = tool_description::compose(scope.scope.name(), paths);
        tool_description::warn_if_too_long(scope.scope.name(), &composed);
        server.override_search_skills_description(composed);

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

/// Build the per-session [`prompts::PromptRegistry`] from the resolved
/// scope's central index DB. Opens read-only so the build cannot
/// trip over the advisory lock held by a concurrent writer. Returns
/// the registry on success; surface DB / parse failures as
/// [`TomeError`] so the MCP startup path can log + exit deterministically.
fn build_prompt_registry(
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<prompts::PromptRegistry, TomeError> {
    let conn = crate::index::db::open_read_only(&paths.index_db)?;
    // FR-067: the `expose_agents_as_personas` toggle's effective value is
    // read from the MCP server's SINGLE startup scope — the running
    // server is not project-bound, so project-scope layering of this key
    // has no effect on a running server (documented in
    // `contracts/agent-personas.md`). We resolve it once here, at
    // startup, via the first-declarer-wins scalar walk over the same
    // (project, workspace, global) settings the harness sync consults.
    let expose_personas = resolve_expose_personas(scope, paths)?;
    prompts::PromptRegistry::build_for_workspace(scope.scope.name(), paths, &conn, expose_personas)
}

/// Resolve `expose_agents_as_personas` for the MCP server startup scope
/// via the Phase 6 first-declarer-wins scalar walk (FR-053 / FR-067).
///
/// Loads the project marker (if the scope has a project root), the bound
/// workspace's `settings.toml` (if present), and the global
/// `settings.toml` (if present), then resolves the scalar. This is the
/// scalar resolver — NOT the `harnesses` composition grammar.
///
/// `#[doc(hidden)] pub` so the FR-067 startup-scope integration test
/// (`tests/personas_startup_scope.rs`) can drive the on-disk
/// project→workspace→global resolution directly. Test seam only —
/// production callers reach it through [`build_prompt_registry`].
#[doc(hidden)]
pub fn resolve_expose_personas(scope: &ResolvedScope, paths: &Paths) -> Result<bool, TomeError> {
    use crate::settings::{resolve_scalar_with, scopes};

    // R4-2: the three scope-loaders are promoted to `settings::scopes`;
    // this resolver no longer carries its own copy of the
    // NotFound/parse-error arms.
    let project_marker = scopes::load_project_marker(scope.project_root.as_deref())?;
    let workspace_settings = scopes::load_workspace_settings(paths, scope.scope.name())?;
    let global_settings = scopes::load_global_settings(paths)?;

    Ok(resolve_scalar_with(
        project_marker.as_ref(),
        workspace_settings.as_ref(),
        &global_settings,
        |p| p.expose_agents_as_personas,
        |w| w.expose_agents_as_personas,
        |g| g.expose_agents_as_personas,
    ))
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

//! `tome mcp` — stdio MCP server.
//!
//! Phase 3 / F8 landed the scaffolding (runtime / log / preflight); US1
//! fills in the actual server loop. The read-only surface (issue #497) is
//! `search_skills`, the consolidated `get_skill` (body-fetch plus
//! `metadata_only` introspection), `list_plugins`, `list_catalogs`, and
//! `status`; plus the write-capable `meta`. See [`tools`] for each handler.
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

pub mod live_sync;
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
use std::time::{Duration, Instant};

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
use crate::telemetry::event::Harness;
use crate::workspace::ResolvedScope;

/// Resolve this MCP session's host harness into the closed telemetry
/// [`Harness`] enum, for the `calling_harness` dimension on the MCP-surface
/// funnel events (`tome.search` / `tome.entry_info` / `tome.entry_invoked`).
///
/// `state.host_harness` is the raw id string stamped into the `tome mcp` args
/// at `harness sync` (FR-028 / Phase 9). Mapping runs through the SSOT
/// [`crate::commands::harness::harness_name_to_enum`] — the SAME bridge the CLI
/// `harness_action` emit uses — so a `None`, unstamped, or unmappable host
/// yields `None` and the optional event field is simply OMITTED (never a
/// guessed closed-enum value).
pub(crate) fn calling_harness(state: &state::McpState) -> Option<Harness> {
    state
        .host_harness
        .as_deref()
        .and_then(crate::commands::harness::harness_name_to_enum)
}

/// Resolve the funnel `rank` for `entry_name` from this session's
/// most-recent-search state (FR-028), for the `tome.entry_info` /
/// `tome.entry_invoked` events.
///
/// Looks the name up in [`state.last_search_ranks`](state::McpState::last_search_ranks),
/// which `search_skills::handle` clears + repopulates on every search. An entry
/// with no preceding search this session (or one absent from the latest result
/// list) yields `0` (the kernel buckets `0` to "no rank"). A poisoned lock
/// degrades to `0` (best-effort).
pub(crate) fn rank_for(state: &state::McpState, entry_name: &str) -> u32 {
    state
        .last_search_ranks
        .lock()
        .ok()
        .and_then(|ranks| ranks.get(entry_name).copied())
        .unwrap_or(0)
}

/// Emit a best-effort MCP-surface `tome.error` (FR-029/029a) for an error a tool
/// handler is about to return to the harness.
///
/// `category` is the closed [`ErrorCategory`](crate::error::ErrorCategory) — the
/// ONLY error detail that leaves the box (never the raw message). `surface` is
/// fixed to [`Surface::Mcp`] and `calling_harness` is resolved from this
/// session's host harness via [`calling_harness`], so the MCP funnel carries the
/// same dimensions the success-path events do.
///
/// Best-effort: this is the same infallible local append as every other enqueue —
/// it NEVER alters the returned `McpError`, produces user output, blocks, or
/// flushes. Call it at each handler's terminal `TomeError`-bearing error site.
pub(crate) fn enqueue_tool_error(state: &state::McpState, category: crate::error::ErrorCategory) {
    crate::telemetry::emit(crate::telemetry::event::ErrorEvent {
        error_class: category,
        surface: crate::telemetry::event::Surface::Mcp,
        calling_harness: calling_harness(state),
    });
}

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
    // Resolve the file log sink before anything else can fail, honouring
    // the `TOME_MCP_LOG` override (unset → default path; `off`/empty → no
    // sink; `<path>` → that path). A failure opening the *default* path
    // still surfaces as a TomeError on stderr (byte-identical to before);
    // an unopenable *override* path degrades to no sink with a stderr
    // warning so the server always starts. `None` = no file log.
    let log_file = log::open_sink(paths)?;
    // Load the logging level from config defensively (any error → default level)
    // so a malformed config.toml doesn't prevent the MCP server from starting
    // once past the workspace resolve step. In practice `harness sync` always
    // stamps `--workspace <ws>` into the `tome mcp` args, so the upstream
    // `ResolvedScope::resolve()` takes the flag branch and skips the strict config
    // read entirely. A hand-run `tome mcp` WITHOUT `--workspace` would hit the
    // strict resolve and exit 5 before reaching this point. The defensive load
    // here guards the logging initialisation only; it does not gate server startup.
    let cfg_level = crate::config::load_or_default(paths).logging.level;
    log::init_subscriber(log_file, cfg_level)?;

    let runtime = runtime::build_runtime()?;

    runtime.block_on(async {
        // Pre-flight is synchronous (model load, index open, SHA-256
        // are all sync). Run it on the blocking pool so the
        // single-threaded reactor isn't held up by the hash step.
        //
        // FR-027 cold-start timing: time the pre-flight as the `embedder_load`
        // measure. The pre-flight's dominant cost is the ONNX embedder load
        // (`FastembedEmbedder::load`); its index-open + SHA-256 verify are
        // comparatively cheap. Best-effort — the bucket is coarse (4 buckets),
        // so attributing the small verify overhead to the embedder-load bucket
        // is within tolerance. The `index_ready` measure is the separate
        // prompt-registry build below (the index READ phase).
        let scope_clone = scope.clone();
        let paths_clone = paths.clone();
        let preflight_started = Instant::now();
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
                        check = e.category().as_str(),
                        error = %scrub_to_string(e.to_string().as_bytes()),
                        "pre-flight check failed",
                    );
                    return Err(e);
                }
            };
        // Captured AFTER the pre-flight succeeds (a failed pre-flight returned
        // above, so no cold-start event fires for a non-starting server).
        let embedder_load_elapsed = preflight_started.elapsed();
        let embedder_model_id = handle.embedder_entry.name;

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
        // FR-027 cold-start timing: time the prompt-registry build as the
        // `index_ready` measure — it is the index READ phase (read-only DB
        // open + the per-entry frontmatter parses), the closest cleanly
        // measurable "index ready" step. Best-effort.
        let index_ready_started = Instant::now();
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
        let index_ready_elapsed = index_ready_started.elapsed();

        // FR-027: `tome.cold_start` ONCE per server start, fired here — after
        // both the embedder load and the index read have completed, before the
        // transport binds + serving begins. This is ALSO the MCP silent-mint
        // trigger (AC#7): the first `enqueue` lazily mints the install id with
        // no first-run notice (the notice is a CLI-only concern). Best-effort —
        // a sub-ms local append that never blocks startup or flushes.
        crate::telemetry::emit(crate::telemetry::event::ColdStart {
            // clamp: any realistic latency fits u32 (ms); saturate rather than
            // wrap (`Duration::as_millis()` is u128, `as u32` wraps at ~49 days).
            embedder_load_ms: embedder_load_elapsed.as_millis().min(u32::MAX as u128) as u32,
            index_ready_ms: index_ready_elapsed.as_millis().min(u32::MAX as u128) as u32,
            embedder_model_id: Some(embedder_model_id),
        });
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
            embedder_seed: handle.embedder_seed.clone(),
            reranker_entry: handle.reranker_entry,
            prompt_registry: Arc::new(std::sync::RwLock::new(Arc::new(prompt_registry))),
            host_harness: host_harness.clone(),
            last_search_ranks: std::sync::Mutex::new(std::collections::HashMap::new()),
        });

        // Long-running background flusher: interval drain (std thread; sync). None when
        // telemetry is disabled (Flusher::start returns None). Held for the server's
        // lifetime; dropped at shutdown stops the thread.
        let _flusher = crate::telemetry::handle().and_then(|h| {
            gauge_telemetry::Flusher::start(h, std::time::Duration::from_secs(300), 0)
        });

        // The live-sync watcher needs its own `McpState` handle (it shares the
        // server's swappable prompt-registry cell through it). Clone the `Arc`
        // before `Server::new` consumes `state`.
        let state_for_watch = state.clone();
        let mut server = server::Server::new(state);

        // FR-425 + NFR-103: compose the `search_skills` tool
        // description from the fixed scaffold + the workspace's
        // cached `[summaries].short` (if any). One-shot synchronous
        // file read at startup — no in-process summariser invocation.
        let composed = tool_description::compose(scope.scope.name(), paths);
        tool_description::warn_if_too_long(scope.scope.name(), &composed);
        server.override_search_skills_description(composed);

        // Clone the swappable prompt-router + description cells the watcher
        // rebuilds in place, BEFORE `serve_server` moves the server.
        let (prompt_cell, desc_cell) = server.live_sync_cells();

        let running = rmcp::serve_server(server, rmcp::transport::stdio())
            .await
            .map_err(|e| TomeError::McpStartupFailed {
                reason: format!("rmcp serve_server: {e}"),
            })?;

        // Live-sync watcher: when the workspace drifts out-of-process (a CLI
        // enable/disable/reindex or a regenerated summary), rebuild the prompt
        // list + tool description in place and emit the matching `list_changed`
        // notification. Aborted on shutdown alongside the telemetry task so it
        // can't outlive the server.
        let watch_task = tokio::spawn(live_sync::watch(live_sync::Handles {
            state: state_for_watch,
            prompt_cell,
            desc_cell,
            peer: running.peer().clone(),
        }));

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

        let shutdown_result = tokio::select! {
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
        };

        // Stop the background flusher (signals its std thread and joins it), then
        // do one final synchronous drain so events enqueued since the last 5-min
        // tick are delivered before the process exits. `flush_blocking` spawns a
        // worker thread internally and waits up to 3s — safe to call from async
        // on the shutdown path where brief blocking is acceptable.
        drop(_flusher);
        if let Some(h) = crate::telemetry::handle() {
            h.flush_blocking(gauge_telemetry::client::DEFAULT_FLUSH_TIMEOUT);
        }

        // Same lifetime tie for the live-sync watcher: abort it once serving
        // has ended so it can't outlive the server or leak past the `block_on`.
        // Any rebuild it might be mid-running is on the blocking pool and only
        // mutates in-process cells — losing it is harmless (the next process
        // run rebuilds the registry at startup).
        watch_task.abort();

        shutdown_result
    })
}

/// Build the per-session [`prompts::PromptRegistry`] from the resolved
/// scope's central index DB. Opens read-only so the build cannot
/// trip over the advisory lock held by a concurrent writer. Returns
/// the registry on success; surface DB / parse failures as
/// [`TomeError`] so the MCP startup path can log + exit deterministically.
pub(crate) fn build_prompt_registry(
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

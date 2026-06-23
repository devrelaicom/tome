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

/// Resolve the funnel `rank_bucket` for `entry_name` from this session's
/// most-recent-search state (FR-028), for the `tome.entry_info` /
/// `tome.entry_invoked` events.
///
/// Looks the name up in [`state.last_search_ranks`](state::McpState::last_search_ranks),
/// which `search_skills::handle` clears + repopulates on every search. An
/// entry with no preceding search this session (or one absent from the latest
/// result list) yields [`RankBucket::None`] — `RankBucket::from_rank` also maps
/// a defensive `0` to `None`. A poisoned lock degrades to `None` (best-effort).
pub(crate) fn rank_bucket_for(
    state: &state::McpState,
    entry_name: &str,
) -> crate::telemetry::buckets::RankBucket {
    use crate::telemetry::buckets::RankBucket;
    let rank = state
        .last_search_ranks
        .lock()
        .ok()
        .and_then(|ranks| ranks.get(entry_name).copied())
        .unwrap_or(0);
    RankBucket::from_rank(rank)
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
    crate::telemetry::enqueue(crate::telemetry::event::ErrorEvent {
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
    // Open the file log appender before anything else can fail. If this
    // errors, the harness sees a TomeError on stderr instead of a
    // silent log.
    let log_file = log::open_appender(paths)?;
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
        crate::telemetry::enqueue(crate::telemetry::event::ColdStart {
            embedder_load_bucket: crate::telemetry::buckets::LoadBucket::from(
                embedder_load_elapsed,
            ),
            index_ready_bucket: crate::telemetry::buckets::LoadBucket::from(index_ready_elapsed),
            embedder_model_id: Some(embedder_model_id),
        });
        info!(
            target: "tome::mcp::prompts",
            prompt_count = prompt_registry.by_name.len(),
            collision_count = prompt_registry.collisions.len(),
            "prompt registry built",
        );

        // FR-050: the "flush soon" signal, shared between the tool handlers
        // (which raise it on the ≥50-enqueue crossing via `McpState::note_enqueue`)
        // and the background flush task spawned below.
        let flush_signal = Arc::new(tokio::sync::Notify::new());

        let state = Arc::new(state::McpState {
            embedder: Arc::from(handle.embedder),
            reranker: OnceCell::new(),
            scope: scope.clone(),
            paths: paths.clone(),
            embedder_entry: handle.embedder_entry,
            reranker_entry: handle.reranker_entry,
            prompt_registry: Arc::new(std::sync::RwLock::new(Arc::new(prompt_registry))),
            host_harness: host_harness.clone(),
            last_search_ranks: std::sync::Mutex::new(std::collections::HashMap::new()),
            flush_signal: flush_signal.clone(),
            enqueued_since_flush: std::sync::atomic::AtomicUsize::new(0),
        });

        // FR-049/050: the background telemetry flush task. It owns the 5-min
        // interval + the shared "flush soon" `Notify`, and on EITHER trigger
        // `spawn_blocking`s the one sync drain (`telemetry::flush`). It NEVER
        // calls `flush()` on the async thread — that does a blocking
        // `reqwest::blocking` POST and would stall this single-thread runtime.
        // Its `JoinHandle` is aborted after `serve_server` returns so the task
        // can't outlive the server (no leak).
        let flush_task = tokio::spawn(telemetry_flush_loop(flush_signal));

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

        // Tie the background flush task's lifetime to the server: once serving
        // has ended (either arm above), abort it so it can't outlive the server
        // or leak past the `block_on`. The drain it might be mid-running is the
        // sync `telemetry::flush` on the blocking pool, which self-locks + is
        // best-effort — losing an in-flight final flush is acceptable (the next
        // process run's exit hook / timer re-drains the queue).
        flush_task.abort();

        // Same lifetime tie for the live-sync watcher: abort it once serving
        // has ended so it can't outlive the server or leak past the `block_on`.
        // Any rebuild it might be mid-running is on the blocking pool and only
        // mutates in-process cells — losing it is harmless (the next process
        // run rebuilds the registry at startup).
        watch_task.abort();

        shutdown_result
    })
}

/// Phase 10 / US3 (FR-049/050): the background telemetry flush loop.
///
/// Holds the 5-min interval and the shared "flush soon" [`Notify`]; on EITHER
/// trigger it `spawn_blocking`s the one sync drain ([`crate::telemetry::flush`]).
/// This is the ONLY bridge from the async island into the `tokio`-free
/// `telemetry/` module, and it crosses via `spawn_blocking` per the sync-boundary
/// discipline — `flush()` does a blocking `reqwest::blocking` POST that must
/// NEVER run on this single-thread runtime's reactor.
///
/// Best-effort throughout (NFR-001): the drain's result is discarded
/// (background flushes fail silent by design; the foreground `tome telemetry
/// flush` is the loud one). `flush()` self-acquires its non-blocking `flush.lock`
/// and self-gates on the grace period, so overlapping interval/notify flushes
/// are safe — the loser no-ops. The loop runs until its `JoinHandle` is aborted
/// on server shutdown.
async fn telemetry_flush_loop(flush_signal: Arc<tokio::sync::Notify>) {
    // FR-049: the 5-min cadence. `MissedTickBehavior::Skip` (the default for a
    // fresh interval is `Burst`; a blocking drain could overrun a tick, so skip
    // missed ticks rather than firing a backlog of flushes back-to-back).
    let mut interval = tokio::time::interval(Duration::from_secs(300));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // The first `interval.tick()` resolves immediately; consume it so the first
    // real flush is one full period out (the cold-start enqueue is already on
    // the queue and the exit/notify paths cover early delivery — we don't want a
    // flush in the first reactor poll of the loop).
    interval.tick().await;

    loop {
        // One select-arm await per loop turn, factored into `flush_loop_turn` so a
        // test can drive ONE iteration deterministically (a `Notify` permit, no
        // 5-min interval wait). The production loop and the test share the SAME
        // arm-dispatch shape — `flush_loop_turn` awaits either trigger, then
        // `dispatch_flush`es OFF the reactor.
        flush_loop_turn(&mut interval, &flush_signal).await;
    }
}

/// One turn of [`telemetry_flush_loop`]: await EITHER the interval tick OR the
/// "flush soon" notify, then dispatch the drain off the reactor via
/// [`dispatch_flush`]. Factored out so a `#[tokio::test]` can drive a single
/// iteration without the 5-min interval wait (it fires the `Notify` arm). The
/// production loop calls this in a bare `loop`, so the behaviour is unchanged.
async fn flush_loop_turn(interval: &mut tokio::time::Interval, flush_signal: &tokio::sync::Notify) {
    tokio::select! {
        _ = interval.tick() => {}
        _ = flush_signal.notified() => {}
    }
    dispatch_flush();
}

/// Dispatch the one sync drain OFF the reactor (NFR-001 / SC-009): the drain does
/// a blocking `reqwest::blocking` POST that must NEVER run on the single-thread
/// runtime's reactor, so it is `spawn_blocking`ed. Fire-and-forget — the join is
/// deliberately NOT awaited so the loop stays responsive to the next trigger; the
/// drain self-serialises on its own non-blocking `flush.lock` and self-gates on
/// the grace period, so overlapping dispatches are safe (the loser no-ops).
fn dispatch_flush() {
    tokio::task::spawn_blocking(|| {
        let _ = crate::telemetry::flush();
    });
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

#[cfg(test)]
mod telemetry_flush_loop_tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;
    use tempfile::TempDir;

    use crate::telemetry::flush::TransportGuard;
    use crate::telemetry::{identity, queue};

    // The flush loop drains the DEFAULT `Paths` (`telemetry::flush()` resolves
    // `$HOME`), and it POSTs through the process-global flush `TransportGuard` +
    // moves the `transport::NETWORK_CALLS` counter. Those seams are ALSO touched
    // by the `flush.rs` and `transport.rs` lib tests, so a per-module mutex is
    // NOT enough — a concurrent `flush.rs` seam test clobbered this one's
    // transport override / `$HOME` queue (the `notify_drives_exactly_one_drain`
    // flake). So this guard holds the ONE shared `crate::telemetry::test_serial()`
    // lock EVERY seam-touching lib test acquires, guaranteeing no two run at once.
    /// RAII: point `$HOME` at `dir` for the test, restoring the prior value on
    /// drop, while holding the shared telemetry-test serial lock so the
    /// process-global `$HOME` + flush seams can't be clobbered by a concurrent
    /// seam-touching test anywhere in this binary.
    struct HomeAndSerial {
        _lock: std::sync::MutexGuard<'static, ()>,
        prior_home: Option<std::ffi::OsString>,
    }
    impl HomeAndSerial {
        fn install(home: &std::path::Path) -> Self {
            let lock = crate::telemetry::test_serial();
            let prior_home = std::env::var_os("HOME");
            // SAFETY: we hold the telemetry test serial lock for the lifetime of
            // `Self`, so no other seam-touching test reads/writes `$HOME` or the
            // flush seams concurrently.
            unsafe { std::env::set_var("HOME", home) };
            Self {
                _lock: lock,
                prior_home,
            }
        }
    }
    impl Drop for HomeAndSerial {
        fn drop(&mut self) {
            // SAFETY: still under the telemetry test serial lock.
            unsafe {
                match self.prior_home.take() {
                    Some(v) => std::env::set_var("HOME", v),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
    }

    /// The `<home>/.tome`-rooted `Paths` the default resolver produces for `home`.
    fn paths_for_home(home: &std::path::Path) -> crate::paths::Paths {
        crate::paths::Paths::from_root(home.join(".tome"))
    }

    /// Mint an install id, backdate its mtime (the mint time) well past the
    /// 10-min grace, and seed the queue so a drain actually sends. The mtime is
    /// backdated rather than installing a `ClockGuard` because the drain runs on a
    /// `spawn_blocking` pool THREAD, where a thread-local clock override would NOT
    /// be visible — the grace gate must be satisfied in real wall-clock terms.
    fn seed_drainable(paths: &crate::paths::Paths) {
        identity::ensure_install_id(paths).expect("mint id");
        let past = filetime::FileTime::from_system_time(
            std::time::SystemTime::now() - std::time::Duration::from_secs(3600),
        );
        filetime::set_file_mtime(paths.telemetry_id(), past).expect("backdate id mtime");
        queue::rewrite(
            paths,
            &[r#"{"event_type":"tome.search","n":1}"#.to_string()],
        )
        .expect("seed queue");
    }

    /// A recording transport that counts every POST and returns 2xx.
    fn counting_transport() -> (TransportGuard, Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let posts = Arc::new(AtomicUsize::new(0));
        let p2 = Arc::clone(&posts);
        let guard = TransportGuard::install(move |_s, _b| {
            p2.fetch_add(1, Ordering::SeqCst);
            Ok(200)
        });
        (guard, posts)
    }

    /// Await the queue draining to empty (the `spawn_blocking` drain is async to
    /// the test), bounded so a regression fails fast rather than hanging.
    async fn await_drained(paths: &crate::paths::Paths) {
        for _ in 0..200 {
            if queue::count_pending(paths) == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("queue did not drain within the bounded wait");
    }

    /// (a) A `notify_one()` drives EXACTLY ONE drain through `flush_loop_turn`: the
    /// notify arm fires, `dispatch_flush` `spawn_blocking`s the sync drain, and the
    /// seeded queue is emptied (the recording transport received the batch).
    ///
    /// On the current-thread runtime (the MCP server's flavour — `tokio` is built
    /// without `rt-multi-thread`), `spawn_blocking` still runs the drain on the
    /// blocking pool; the test awaits (yielding to the reactor) until the queue
    /// empties, which is exactly how the production loop observes its drains.
    #[tokio::test]
    async fn notify_drives_exactly_one_drain() {
        let home = TempDir::new().unwrap();
        let _home = HomeAndSerial::install(home.path());
        let paths = paths_for_home(home.path());

        seed_drainable(&paths);
        let (_t, posts) = counting_transport();

        // A long interval so ONLY the notify arm can fire within the test.
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await; // consume the immediate first tick

        let signal = Arc::new(tokio::sync::Notify::new());
        signal.notify_one();

        // One turn: the notify arm resolves, then `dispatch_flush` runs the drain.
        flush_loop_turn(&mut interval, &signal).await;
        await_drained(&paths).await;

        assert_eq!(
            queue::count_pending(&paths),
            0,
            "the notify-triggered turn drained the queue exactly once"
        );
        assert_eq!(
            posts.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "exactly one batch POST reached the transport"
        );
    }

    /// (b, off-reactor) `dispatch_flush` runs the drain OFF the reactor via
    /// `spawn_blocking`: the call returns IMMEDIATELY (it does not block on the
    /// `reqwest::blocking` POST) and the queue drains afterwards on the blocking
    /// pool. On the current-thread runtime we prove this by showing the synchronous
    /// `dispatch_flush()` returns in well under the drain's own wall time, then the
    /// queue empties once the test yields to the reactor / blocking pool. (The
    /// fuller "a concurrently-awaited future stays responsive" form needs a
    /// multi-thread runtime, which `tokio` is not built with here; this is the
    /// current-thread-appropriate proof that the drain is NOT inline.)
    #[tokio::test]
    async fn dispatch_runs_drain_off_the_reactor() {
        let home = TempDir::new().unwrap();
        let _home = HomeAndSerial::install(home.path());
        let paths = paths_for_home(home.path());

        seed_drainable(&paths);
        let (_t, _posts) = counting_transport();

        // `dispatch_flush` must NOT block on the drain — it only `spawn_blocking`s.
        // The call itself returns promptly (the queue is still pending here).
        let started = std::time::Instant::now();
        dispatch_flush();
        let dispatch_elapsed = started.elapsed();
        assert!(
            dispatch_elapsed < Duration::from_millis(100),
            "dispatch_flush returns immediately (off-reactor), took {dispatch_elapsed:?}"
        );

        // The drain completes on the blocking pool once the test yields the reactor.
        await_drained(&paths).await;
        assert_eq!(
            queue::count_pending(&paths),
            0,
            "the off-reactor drain completed"
        );
    }

    /// (c) Aborting the loop's `JoinHandle` stops further drains: after `abort()`,
    /// a new `notify_one` produces NO further drain (the aborted task never reaches
    /// `flush_loop_turn` again). We spawn the REAL `telemetry_flush_loop`, let one
    /// notify drain, abort, then re-seed + re-notify and assert the queue is
    /// untouched.
    #[tokio::test]
    async fn abort_stops_further_drains() {
        let home = TempDir::new().unwrap();
        let _home = HomeAndSerial::install(home.path());
        let paths = paths_for_home(home.path());

        seed_drainable(&paths);
        let (_t, _posts) = counting_transport();

        let signal = Arc::new(tokio::sync::Notify::new());
        let handle = tokio::spawn(telemetry_flush_loop(signal.clone()));

        // First notify ⇒ one drain empties the seeded queue.
        signal.notify_one();
        await_drained(&paths).await;
        assert_eq!(queue::count_pending(&paths), 0, "first notify drained");

        // Abort the loop; it can no longer reach `flush_loop_turn`.
        handle.abort();
        let _ = handle.await; // observe the cancellation
        // Give any already-spawned blocking task a beat to settle.
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Re-seed and re-notify: an aborted loop ignores the signal ⇒ no drain.
        queue::rewrite(
            &paths,
            &[r#"{"event_type":"tome.search","n":2}"#.to_string()],
        )
        .expect("re-seed queue");
        signal.notify_one();
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(
            queue::count_pending(&paths),
            1,
            "after abort, a new notify produces no further drain"
        );
    }
}

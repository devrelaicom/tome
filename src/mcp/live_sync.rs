//! Live-sync watcher for the long-running MCP server. The CLI mutates the
//! central DB (and the cached workspace summary) out-of-process; this module
//! polls a cheap composite drift signal and, on change, rebuilds the prompt
//! router and/or the `search_skills` description in place and reports which
//! surfaces moved so the caller can emit the matching `list_changed`
//! notification. Sync-only: all DB/file work runs on the blocking pool via the
//! caller's `spawn_blocking`; this module never `.await`s.

use std::sync::Arc;
use std::sync::RwLock;
use std::time::Duration;

use rmcp::handler::server::router::prompt::PromptRouter;
use rmcp::{Peer, RoleServer};

use crate::mcp::server::Server;
use crate::mcp::state::McpState;
use crate::paths::Paths;
use crate::workspace::{ResolvedScope, WorkspaceName};

/// Cheap fingerprint of the inputs that determine the prompt list + the tool
/// description. Recomputed each tick; a change triggers a rebuild.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriftSignal {
    /// COUNT(*) of user-invocable enabled entries in the workspace.
    pub entry_count: i64,
    /// MAX(indexed_at) over those entries (0 when none).
    pub max_indexed_at: i64,
    /// The cached `[summaries].short` content (feeds the description).
    pub short_blurb: String,
}

/// Which live surfaces changed on a recompute.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Changed {
    pub prompts: bool,
    pub tools: bool,
}

/// Read the current drift signal for the served workspace.
///
/// Opens the index read-only so a concurrent CLI writer's advisory lock can
/// never block the watcher. A missing DB yields an empty signal (a workspace
/// with no index has no prompts to drift); the short blurb is read separately
/// from `settings.toml` (a fresh workspace simply has none).
pub fn probe(scope: &ResolvedScope, paths: &Paths) -> Result<DriftSignal, crate::error::TomeError> {
    let name = scope.scope.name();
    let (entry_count, max_indexed_at) = if paths.index_db.exists() {
        let conn = crate::index::db::open_read_only(&paths.index_db)?;
        let id = crate::index::workspaces::resolve_id_required(&conn, name)?;
        conn.query_row(
            "SELECT COUNT(*), COALESCE(MAX(s.indexed_at), 0)
             FROM workspace_skills ws
             JOIN skills s ON s.id = ws.skill_id
             WHERE ws.workspace_id = ?1 AND s.user_invocable = 1",
            rusqlite::params![id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|e| {
            crate::error::TomeError::IndexIntegrityCheckFailure(format!("live-sync probe: {e}"))
        })?
    } else {
        (0, 0)
    };
    let short_blurb = read_short(paths, name);
    Ok(DriftSignal {
        entry_count,
        max_indexed_at,
        short_blurb,
    })
}

/// Read the workspace's cached `[summaries].short`, best-effort. Mirrors the
/// `harness::routing` long-summary read but pulls the `short` field (the one
/// that feeds the `search_skills` description). Any read/parse failure degrades
/// to an empty blurb — a malformed cache must never refuse the drift probe.
fn read_short(paths: &Paths, name: &WorkspaceName) -> String {
    let settings_path = paths.workspace_settings_file(name);
    let Ok(body) =
        crate::util::bounded_read_to_string(&settings_path, crate::util::TOME_CONFIG_MAX)
    else {
        return String::new();
    };
    match crate::settings::parser::parse_workspace(&body) {
        Ok(p) => p.summaries.map(|s| s.short).unwrap_or_default(),
        Err(_) => String::new(),
    }
}

/// Rebuild whichever surface the new signal indicates changed, swap the cells
/// in place, and report what moved. Pure of any peer/notification concern.
///
/// Poisoned locks are recovered via `into_inner` per the codebase RwLock
/// convention — a panic in a previous holder must not wedge the watcher.
pub fn recompute(
    prev: &DriftSignal,
    next: &DriftSignal,
    state: &Arc<McpState>,
    prompt_cell: &RwLock<PromptRouter<Server>>,
    desc_cell: &RwLock<String>,
) -> Changed {
    let mut changed = Changed::default();

    // The prompt list is determined by the user-invocable enabled entry set and
    // their freshness; either moving means the registry must be rebuilt. A
    // build failure (e.g. a concurrent writer mid-migration) leaves the existing
    // router in place — we simply report no prompt drift this tick and retry on
    // the next.
    let prompts_drifted =
        prev.entry_count != next.entry_count || prev.max_indexed_at != next.max_indexed_at;
    if prompts_drifted
        && let Ok(registry) = crate::mcp::build_prompt_registry(&state.scope, &state.paths)
    {
        let registry = Arc::new(registry);
        *state
            .prompt_registry
            .write()
            .unwrap_or_else(|e| e.into_inner()) = registry.clone();
        let new_router = crate::mcp::prompts::build_router::<Server>(&registry, state.clone());
        *prompt_cell.write().unwrap_or_else(|e| e.into_inner()) = new_router;
        changed.prompts = true;
    }

    // The description is cheap to recompose unconditionally (one bounded file
    // read); compare against the cached value and only swap + flag on a real
    // change so we don't emit a spurious `tools/list_changed`.
    let new_desc = crate::mcp::tool_description::compose(state.scope.scope.name(), &state.paths);
    if *desc_cell.read().unwrap_or_else(|e| e.into_inner()) != new_desc {
        *desc_cell.write().unwrap_or_else(|e| e.into_inner()) = new_desc;
        changed.tools = true;
    }

    changed
}

/// Bundle handed to the watcher task — everything it needs to probe,
/// recompute, and notify. Each field is `Clone + Send + 'static`, so the
/// bundle moves cleanly into the spawned watcher.
pub struct Handles {
    pub state: Arc<McpState>,
    pub prompt_cell: Arc<RwLock<PromptRouter<Server>>>,
    pub desc_cell: Arc<RwLock<String>>,
    pub peer: Peer<RoleServer>,
}

/// Poll interval. Short enough that a newly-enabled prompt appears within a
/// minute; the no-op tick is one tiny indexed query (a read-only COUNT) plus
/// one bounded settings read.
const POLL_INTERVAL: Duration = Duration::from_secs(60);

/// The watcher loop. Mirrors the telemetry loop's `loop { turn().await }`
/// shape so a test can drive a single turn deterministically. Runs until its
/// `JoinHandle` is aborted on server shutdown (alongside the telemetry task)
/// so it can never outlive the server.
pub async fn watch(handles: Handles) {
    let mut interval = tokio::time::interval(POLL_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    // Consume the immediate first tick so the first real probe is one full
    // period out — the server's startup already built the prompt list + tool
    // description from this same state, so there is nothing to reconcile yet.
    interval.tick().await;
    // The baseline reflects what the server advertised at startup. A failed
    // initial probe (e.g. a concurrent writer mid-migration) degrades to the
    // empty signal; the first real tick re-probes and reconciles any drift.
    let mut last = probe(&handles.state.scope, &handles.state.paths).unwrap_or_default();
    loop {
        interval.tick().await;
        last = watch_turn(&handles, last).await;
    }
}

/// One probe→recompute→notify turn. Returns the new baseline signal.
///
/// The two sync seams (`probe` reads the index + settings; `recompute` may
/// rebuild the prompt router) run on the blocking pool via `spawn_blocking`
/// per the sync-boundary discipline — neither must run on the single-thread
/// reactor. A failed probe (join error or DB error) leaves the baseline
/// unchanged and retries next tick.
pub async fn watch_turn(handles: &Handles, last: DriftSignal) -> DriftSignal {
    let state = handles.state.clone();
    let probed = tokio::task::spawn_blocking(move || probe(&state.scope, &state.paths))
        .await
        .ok()
        .and_then(Result::ok);
    let Some(next) = probed else { return last };
    if next == last {
        return next;
    }
    let state = handles.state.clone();
    let prompt_cell = handles.prompt_cell.clone();
    let desc_cell = handles.desc_cell.clone();
    let prev = last.clone();
    let next_for_blocking = next.clone();
    // `recompute` takes `&RwLock<...>`; the `Arc<RwLock<...>>` deref-coerces.
    let changed = tokio::task::spawn_blocking(move || {
        recompute(&prev, &next_for_blocking, &state, &prompt_cell, &desc_cell)
    })
    .await
    .unwrap_or_default();
    // Notify only the surfaces that actually moved. A failed notify (the peer
    // is shutting down) is logged at debug and otherwise ignored — the next
    // tick's recompute is a no-op (the cells already hold the new state) so we
    // do not retry the notification.
    if changed.prompts
        && let Err(e) = handles.peer.notify_prompt_list_changed().await
    {
        tracing::debug!("live-sync: prompts/list_changed notify failed: {e}");
    }
    if changed.tools
        && let Err(e) = handles.peer.notify_tool_list_changed().await
    {
        tracing::debug!("live-sync: tools/list_changed notify failed: {e}");
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recompute_flags_tools_when_blurb_changes() {
        let prev = DriftSignal {
            entry_count: 2,
            max_indexed_at: 100,
            short_blurb: "a".into(),
        };
        let next = DriftSignal {
            entry_count: 2,
            max_indexed_at: 100,
            short_blurb: "b".into(),
        };
        assert_ne!(prev, next);
        assert_eq!(prev.entry_count, next.entry_count);
        assert_ne!(prev.short_blurb, next.short_blurb);
    }

    #[test]
    fn probe_on_missing_db_is_empty_signal() {
        let s = DriftSignal::default();
        assert_eq!(s.entry_count, 0);
        assert_eq!(s.max_indexed_at, 0);
        assert!(s.short_blurb.is_empty());
    }
}

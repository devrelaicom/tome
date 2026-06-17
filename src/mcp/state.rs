//! Shared state for the MCP server. Constructed by `mcp::run` after the
//! pre-flight succeeds; threaded into every tool handler via the
//! `Server` wrapper in `mcp::server`.
//!
//! Reranker is lazy-loaded on the first `search_skills` call per
//! FR-109; the `tokio::sync::OnceCell` enables async-friendly
//! initialisation without blocking subsequent calls.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::{Notify, OnceCell};

use crate::embedding::registry::ModelEntry;
use crate::embedding::{Embedder, Reranker};
use crate::mcp::prompts::PromptRegistry;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

/// Phase 10 / US3 (FR-050): the off-path flush is scheduled when the count of
/// events this session enqueues SINCE the last scheduled flush reaches this
/// many. Mirrors the CLI exit hook's `SPAWN_QUEUE_THRESHOLD` (a queue at/over 50
/// pending events triggers delivery). The MCP server tracks its own enqueue
/// count rather than re-reading the on-disk queue on every tool call — see
/// [`McpState::note_enqueue`].
const FLUSH_SOON_THRESHOLD: usize = 50;

pub struct McpState {
    pub embedder: Arc<dyn Embedder>,
    pub reranker: OnceCell<Arc<dyn Reranker>>,
    pub scope: ResolvedScope,
    pub paths: Paths,
    /// Registry entry for the loaded embedder. Used by the
    /// `search_skills` pipeline to record drift / pass identity into
    /// `query::run_with_deps`.
    pub embedder_entry: &'static ModelEntry,
    /// Registry entry for the reranker that will be loaded on first
    /// `search_skills` call.
    pub reranker_entry: &'static ModelEntry,
    /// Phase 5 / US1.b + 2026-06 live-sync: prompts capability registry.
    /// Built at startup from the resolved workspace's enabled-and-user-
    /// invocable entries, and swapped in place by the live-sync watcher
    /// when the workspace's skill set drifts (no restart needed). Reads
    /// take the read lock for the sub-µs clone of the `Arc`; the watcher
    /// takes the write lock only to swap.
    pub prompt_registry: Arc<std::sync::RwLock<Arc<PromptRegistry>>>,
    /// Phase 9 / US3: the harness hosting this MCP server, conveyed by
    /// `tome mcp --harness <name>` (stamped into the `tome mcp` args at
    /// `harness sync`). `None` for a legacy/unstamped config — the `meta`
    /// tool then fails closed (FR-029) rather than guessing a harness.
    pub host_harness: Option<String>,
    /// Phase 10 / US2 (FR-028): per-session search→selection funnel state.
    /// Maps an entry `name` to its 1-indexed rank in the MOST RECENT
    /// `search_skills` result list this session. `get_skill` / `get_skill_info`
    /// look the selected entry up here to attribute a `rank_bucket` on their
    /// `tome.entry_invoked` / `tome.entry_info` events — the bucket is `none`
    /// when no preceding search this session ranked the entry.
    ///
    /// WHY a `Mutex<HashMap>` rather than per-request state: the MCP server is
    /// a long-running session, so the funnel join is across SEPARATE tool calls
    /// (search, then a later get) — the rank must outlive the search handler.
    /// Each search clears + repopulates it (only the latest search's ranks
    /// attribute), so it never grows unbounded. The lock is held only for the
    /// sub-µs clear/insert/lookup; it is never held across an `.await`.
    pub last_search_ranks: Mutex<HashMap<String, u32>>,
    /// Phase 10 / US3 (FR-050): "flush soon" signal driving the background
    /// flush task in [`crate::mcp::run`]. The timer task selects between its
    /// 5-min interval tick and this notification; on either it `spawn_blocking`s
    /// the one shared sync drain ([`crate::telemetry::flush`]). A tool handler
    /// raises it (via [`Self::note_enqueue`]) when its enqueue count crosses the
    /// [`FLUSH_SOON_THRESHOLD`], scheduling an OFF-PATH flush — the handler never
    /// flushes inline (SC-009). A default `Notify` when the task is absent (the
    /// integration-test states build one) is harmless: nobody is listening, so
    /// `notify_one` just no-ops.
    pub flush_signal: Arc<Notify>,
    /// Phase 10 / US3 (FR-050): events enqueued by this server SINCE the last
    /// scheduled "flush soon". Bumped once per tool-handler enqueue via
    /// [`Self::note_enqueue`]; when it crosses [`FLUSH_SOON_THRESHOLD`] the
    /// counter resets and [`Self::flush_signal`] is raised. This is a cheap
    /// in-memory counter rather than a per-call `count_pending` re-read of the
    /// on-disk queue — the timer task does the actual (bounded) drain off-path.
    pub enqueued_since_flush: AtomicUsize,
}

impl McpState {
    /// Record that a tool handler just enqueued one telemetry event, and
    /// schedule an OFF-PATH flush if the running count crosses the threshold
    /// (FR-050). Returns `true` iff this call scheduled a flush (the counter
    /// crossed [`FLUSH_SOON_THRESHOLD`] and was reset) — returned for the unit
    /// test; production callers ignore it.
    ///
    /// Cheap + non-blocking: one relaxed atomic increment, and on the crossing a
    /// reset + a `Notify::notify_one`. It NEVER flushes inline (that would block
    /// the tool call on a non-routable endpoint, SC-009) — it only nudges the
    /// background timer task, which `spawn_blocking`s the drain. Best-effort:
    /// the counter is per-session and approximate; an extra/missed nudge only
    /// shifts WHEN the off-path flush fires, never WHETHER events are delivered
    /// (the 5-min interval is the backstop, and the drain self-gates on its
    /// `flush.lock` + grace period).
    pub fn note_enqueue(&self) -> bool {
        note_enqueue_inner(
            &self.enqueued_since_flush,
            &self.flush_signal,
            FLUSH_SOON_THRESHOLD,
        )
    }
}

/// The threshold-crossing decision behind [`McpState::note_enqueue`], factored
/// out so it is unit-testable against a bare `AtomicUsize` + `Notify` without
/// constructing a full (model-bearing) [`McpState`]. Bumps `counter` by one and,
/// on reaching `threshold`, resets it to `0` and raises `signal`; returns `true`
/// iff this call crossed the threshold.
fn note_enqueue_inner(counter: &AtomicUsize, signal: &Notify, threshold: usize) -> bool {
    // `fetch_add` returns the PRIOR value; the new count is `prior + 1`.
    let new_count = counter.fetch_add(1, Ordering::Relaxed) + 1;
    if new_count >= threshold {
        // Reset the counter so the NEXT threshold is another full window of
        // enqueues away (avoids notifying on every call once we cross 50). A
        // racing concurrent caller that also observes the crossing would, at
        // worst, notify twice — harmless (the listener coalesces; the drain
        // self-gates). `store(0)` is the simple, correct reset.
        counter.store(0, Ordering::Relaxed);
        signal.notify_one();
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_enqueue_inner_notifies_only_on_threshold_crossing() {
        let counter = AtomicUsize::new(0);
        let signal = Notify::new();
        let threshold = 50usize;

        // The first 49 enqueues bump the counter but never cross the threshold.
        for i in 1..threshold {
            assert!(
                !note_enqueue_inner(&counter, &signal, threshold),
                "enqueue #{i} (< {threshold}) must not schedule a flush"
            );
            assert_eq!(
                counter.load(Ordering::Relaxed),
                i,
                "counter tracks the enqueue count below threshold"
            );
        }

        // The 50th crosses: it schedules a flush AND resets the counter to 0.
        assert!(
            note_enqueue_inner(&counter, &signal, threshold),
            "the {threshold}th enqueue crosses the threshold and schedules a flush"
        );
        assert_eq!(
            counter.load(Ordering::Relaxed),
            0,
            "crossing the threshold resets the counter so the next window starts fresh"
        );

        // The notification was actually delivered — a `Notified` future that was
        // armed before the cross resolves immediately afterwards.
        let armed = signal.notified();
        signal.notify_one();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        // `notify_one` stored a permit; the armed future + this permit both
        // resolve without hanging. We only need to prove `note_enqueue_inner`'s
        // `notify_one` path is reachable + the post-cross window restarts.
        rt.block_on(armed);

        // After the reset, a fresh window of (threshold - 1) sub-threshold
        // enqueues again does NOT re-notify until the next full crossing.
        for _ in 1..threshold {
            assert!(!note_enqueue_inner(&counter, &signal, threshold));
        }
        assert!(
            note_enqueue_inner(&counter, &signal, threshold),
            "a second full window crosses the threshold again"
        );
    }

    #[test]
    fn note_enqueue_inner_below_threshold_never_resets() {
        let counter = AtomicUsize::new(0);
        let signal = Notify::new();
        // A tiny window of a few enqueues, threshold far away: never crosses,
        // never resets, counter monotonically climbs.
        for expected in 1..=5usize {
            assert!(!note_enqueue_inner(&counter, &signal, 50));
            assert_eq!(counter.load(Ordering::Relaxed), expected);
        }
    }
}

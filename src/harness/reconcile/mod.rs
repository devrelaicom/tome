//! Per-sink harness reconcilers.
//!
//! Phase 6 grew the [`crate::harness::sync`] orchestrator three sink-specific
//! reconcilers — hooks, guardrails, agents — each following the same shape: a
//! per-harness aggregate action map keyed on `name()`, a forward-progress
//! `first_error` (one sink's failure never stops the others from making
//! progress), and the central index DB opened read-only to enumerate the
//! enabled plugins/agents that drive the desired on-disk state.
//!
//! Phase 7 (FR-011, NFR-005) lifts those reconcilers out of `sync.rs` into this
//! module cluster — a strictly behaviour-preserving file move — leaving `sync.rs`
//! as a thin orchestrator. Each `reconcile_<sink>` fn is invoked by
//! [`crate::harness::sync::sync_project`]. The shared [`record_action`] bookkeeping
//! lives here (called by all three reconcilers plus the orchestrator's rules/MCP
//! loop).
//!
//! [`record_action`]: crate::harness::reconcile::record_action
//!
//! ## Fixed sink order
//!
//! The orchestrator always reconciles the three sinks in the order **hooks →
//! guardrails → agents**:
//!
//! * **hooks** runs FIRST so the Claude Code guardrails-suppression predicate
//!   reads the fresh hooks-presence set (FR-016) rather than stale state.
//! * **guardrails** runs SECOND, consuming that hooks-presence set.
//! * **agents** runs LAST.
//!
//! ## First-error precedence
//!
//! With forward progress, more than one sink can fail in a single pass. The
//! orchestrator surfaces the failures in the same fixed sink order: a hooks
//! error (exit 43/44) wins over a guardrails error (exit 46), which wins over
//! an agents error (exit 45). Each sink still reconciles as far as it can
//! before its `first_error` is surfaced after the prior sink's.
//!
//! ## Mass-delete safeguard
//!
//! Every reconciler opens the central DB read-only and **propagates** the open
//! error for an *existing* DB — it never `.ok()`-swallows it. Swallowing would
//! collapse the enabled set to empty and make the cleanup pass mass-delete
//! every reconciled file for a live harness. A genuinely *absent* DB is the
//! only case treated as "no enabled entries". This is the single biggest
//! behaviour-preservation risk of the decomposition and is carried into each
//! module verbatim.
//!
//! The cluster holds the three per-sink reconcilers — [`hooks`], [`guardrails`],
//! [`agents`] — invoked by the thin orchestrator in the fixed order above.

use std::path::Path;

use crate::harness::sync::{Action, SyncChange, SyncOutcome, SyncSubsystem};

pub(crate) mod agents;
pub(crate) mod guardrails;
pub(crate) mod hooks;
pub(crate) mod plugins;

/// Record one on-disk change against the running [`SyncOutcome`].
///
/// Shared bookkeeping across every sink reconciler (hooks/guardrails/agents)
/// and the orchestrator's rules/MCP loop — `pub(crate)` so each caller reuses
/// the one path. Moved out of `agents` in the Phase 7 decomposition (FR-011)
/// once all three sinks shared it.
pub(crate) fn record_action(
    outcome: &mut SyncOutcome,
    harness: &str,
    subsystem: SyncSubsystem,
    path: &Path,
    action: Action,
) {
    let change = SyncChange {
        harness: harness.to_string(),
        subsystem,
        path: path.to_path_buf(),
    };
    match action {
        Action::Created => outcome.added.push(change),
        Action::Updated => outcome.updated.push(change),
        Action::Removed => outcome.removed.push(change),
        Action::LeftAlone => outcome.leave_alones += 1,
    }
}

//! Harness sync outcome shape.
//!
//! Phase 4 / US1.a ships only the result type and a `Default` impl. The
//! real sync algorithm — recompute the effective harness list, dispatch
//! to each `HarnessModule`, edit rules files + MCP configs — lands in
//! US1.b. The skeleton here lets US1.a's `tome workspace use` flow wire
//! the seam end-to-end without taking on the full dispatch surface.
//!
//! The fields are deliberately minimal; the production shape per
//! `contracts/sync-algorithm.md` FR-547 will grow per-harness records,
//! drift counts, and clash diagnostics. Each addition lands with its
//! consumer to keep the wire shape honest.

use serde::Serialize;

/// Outcome of one sync pass — populated for real in US1.b.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncOutcome {
    /// Names of every harness that received a write during this pass.
    /// Empty in the US1.a stub.
    pub harnesses_touched: Vec<String>,
    /// Number of distinct on-disk edits (rules-file block rewrites + MCP
    /// config entry inserts/updates). Always zero in the US1.a stub.
    pub changes_count: u32,
}

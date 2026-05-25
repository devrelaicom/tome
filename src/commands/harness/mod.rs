//! `commands::harness` — the per-project sync entry point invoked by
//! `tome workspace use` (Phase 4 / US1).
//!
//! Phase 4 / US1.a ships the seam only: [`sync_for_project_root`]
//! returns `Ok(SyncOutcome::default())`. US1.b replaces the body with
//! the real algorithm:
//!
//! 1. Recompute the effective harness list for the project (composition
//!    resolver — F8).
//! 2. For each module in lex order, invoke the per-module sync via the
//!    `HarnessModule` trait.
//! 3. Collect per-harness outcomes; map clashes to `HarnessClash` (19).
//!
//! The eventual `tome harness sync` CLI command (US3.c) reuses the same
//! pipeline against an explicit list of harness names rather than the
//! computed effective list.

use std::path::Path;

use crate::error::TomeError;
use crate::workspace::binding::BindDeps;

pub use crate::harness::sync::SyncOutcome;

/// Sync every effective harness for `project_root`. **Stub** in US1.a —
/// returns an empty [`SyncOutcome`] without touching disk so the
/// `tome workspace use` flow can wire the seam end-to-end. The real
/// implementation lands in US1.b.
pub fn sync_for_project_root(
    _project_root: &Path,
    _deps: &BindDeps,
) -> Result<SyncOutcome, TomeError> {
    // US1.b replaces this stub with the real sync algorithm dispatch.
    Ok(SyncOutcome::default())
}

//! Triggered summary regeneration — invoked AFTER an enable / disable /
//! reindex / catalog-update commits its `workspace_skills` mutation.
//!
//! Phase 4 / US4.b. Contract reference:
//! [`contracts/summariser.md` §"Trigger surface"] and
//! [FR-380 / FR-381 / FR-382 / FR-365 / FR-385 / FR-423].
//!
//! ## Forward-progress (FR-385)
//!
//! The skill-state mutation (workspace_skills row INSERT / DELETE)
//! commits in its OWN transaction BEFORE [`regenerate_for_trigger`]
//! is called. If the summariser subsequently fails:
//!
//! * The surrounding command exits with code 24
//!   ([`TomeError::SummariserFailure`]).
//! * The prior `workspace_skills` rows are NOT rolled back — the
//!   enable / disable / reindex took effect.
//! * The workspace's cached `[summaries]` in `settings.toml` is NOT
//!   overwritten — the prior cache survives.
//! * `tome doctor` reports the workspace's cached summary as stale
//!   on next inspection (`summariser_drift` already covers the bulk
//!   of this; T331 widens the harness).
//!
//! This is the "summarise is downstream of state" invariant: the
//! summariser is a derived view, not a precondition.
//!
//! ## Production vs test seams
//!
//! [`regenerate_for_trigger`] constructs the production [`LlamaSummariser`]
//! (which requires the GGUF model file on disk — exit 24 if absent).
//! [`regenerate_for_trigger_with_summariser`] is the dependency-injection
//! seam used by tests to pass a [`StubSummariser`].

use std::cell::RefCell;
use std::sync::Arc;

use crate::error::{SummariserFailureKind, TomeError};
use crate::paths::Paths;
use crate::summarise::{LlamaSummariser, Summariser};
use crate::workspace::{self, WorkspaceName};

thread_local! {
    /// Test-only injection point for the summariser used by
    /// [`regenerate_for_trigger`]. When set, the production
    /// `LlamaSummariser` construction is bypassed entirely — the
    /// override is consulted in production code only because tests
    /// live outside `cfg(test)` visibility.
    ///
    /// Mirrors [`crate::index::migrations::MIGRATIONS_OVERRIDE`]'s
    /// shape: a `thread_local!` `RefCell<Option<...>>` paired with a
    /// guard struct (see [`SummariserOverrideGuard`]) that installs
    /// and clears in `Drop`.
    ///
    /// Doc-hidden to keep it out of the published API; the only
    /// legitimate caller is a test.
    #[doc(hidden)]
    pub static SUMMARISER_OVERRIDE: RefCell<Option<Arc<dyn Summariser>>> =
        const { RefCell::new(None) };
}

/// RAII guard for [`SUMMARISER_OVERRIDE`]. Installs the supplied
/// summariser for the lifetime of the guard; clears the slot on drop
/// (including on test panic).
///
/// Doc-hidden — tests only. Mirrors `MigrationsGuard` in
/// `tests/schema_migration_e2e.rs`.
#[doc(hidden)]
pub struct SummariserOverrideGuard;

impl SummariserOverrideGuard {
    pub fn install(summariser: Arc<dyn Summariser>) -> Self {
        SUMMARISER_OVERRIDE.with(|slot| {
            *slot.borrow_mut() = Some(summariser);
        });
        Self
    }
}

impl Drop for SummariserOverrideGuard {
    fn drop(&mut self) {
        SUMMARISER_OVERRIDE.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }
}

/// Regenerate the cached summary for `name` using the production
/// [`LlamaSummariser`]. Called by every trigger site after their
/// `workspace_skills` mutation commits.
///
/// Returns `Ok(())` on success. On any `SummariserFailure` other than
/// `ModelMissing`, the error bubbles — `main.rs` maps it to exit code
/// 24.
///
/// ## Missing summariser model is a no-op (FR-423 corollary)
///
/// `LlamaSummariser::new` returns `SummariserFailure { ModelMissing }`
/// if `qwen2.5-0.5b-instruct/model.gguf` is absent. Per FR-420's
/// posture, the summariser model is downloaded on-demand by
/// `tome models download` — not as a prerequisite for plugin
/// enable / disable / catalog update. When the model isn't installed
/// the trigger is a no-op: the prior cached summary survives (if
/// any), and the MCP `search_skills` description falls back to the
/// scaffold per [`crate::mcp::tool_description`]. `tome doctor`
/// surfaces the missing model so the operator knows summaries aren't
/// regenerating; the explicit [`crate::workspace::regen_summary`]
/// path still hard-fails with exit 24 if invoked.
///
/// All other `SummariserFailure` variants (inference produced empty
/// output, backend init failed, etc.) DO bubble up per FR-385.
///
/// Per FR-385, the caller MUST commit the skill-state mutation BEFORE
/// invoking this function.
pub fn regenerate_for_trigger(name: &WorkspaceName, paths: &Paths) -> Result<(), TomeError> {
    // Test-only override hook (gated by `SUMMARISER_OVERRIDE`).
    // Production paths never set the slot; the `if let Some(...)`
    // collapses to the fallthrough on every real invocation.
    let override_summariser = SUMMARISER_OVERRIDE.with(|slot| slot.borrow().as_ref().cloned());
    if let Some(s) = override_summariser {
        return regenerate_for_trigger_with_summariser(name, s.as_ref(), paths);
    }

    let summariser = match LlamaSummariser::new(paths) {
        Ok(s) => s,
        Err(TomeError::SummariserFailure {
            kind: SummariserFailureKind::ModelMissing,
        }) => {
            tracing::debug!(
                workspace = name.as_str(),
                "summariser model not installed; skipping trigger regeneration (cached summary survives if any)",
            );
            return Ok(());
        }
        Err(other) => return Err(other),
    };
    regenerate_for_trigger_with_summariser(name, &summariser, paths)
}

/// Dependency-injection variant. Production code goes through
/// [`regenerate_for_trigger`]; tests pass a [`StubSummariser`] to
/// exercise the trigger plumbing without touching real models.
pub fn regenerate_for_trigger_with_summariser(
    name: &WorkspaceName,
    summariser: &dyn Summariser,
    paths: &Paths,
) -> Result<(), TomeError> {
    let _outcome = workspace::regen_summary::regen(name, summariser, paths)?;
    Ok(())
}

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
//! is called. The production trigger wrapper degrades ALL summariser
//! failures to a non-fatal `warn!` and returns `Ok(())` (changed by
//! issue #208):
//!
//! * A missing model is a silent `debug!` no-op — the GGUF is
//!   downloaded on demand, so absence is expected on fresh installs.
//! * Any other failure (e.g. `BackendInitFailed`, `OutputEmpty`) is
//!   logged at `warn!` and swallowed — the state mutation already
//!   committed; crashing the command after the fact would leave the
//!   user with a broken experience.
//! * The prior `workspace_skills` rows are NOT rolled back — the
//!   enable / disable / reindex took effect.
//! * The workspace's cached `[summaries]` in `settings.toml` is NOT
//!   overwritten — the prior cache survives intact.
//! * `tome doctor` reports the workspace's cached summary as stale
//!   on next inspection (`summariser_drift` already covers the bulk
//!   of this; T331 widens the harness).
//! * Run `tome workspace regen-summary` to retry after installing
//!   the model or resolving the failure.
//!
//! This is the "summarise is downstream of state" invariant: the
//! summariser is a derived view, not a precondition.
//!
//! ## Production vs test seams
//!
//! [`regenerate_for_trigger`] constructs the production [`LlamaSummariser`]
//! and degrades all failures as described above.
//! [`regenerate_for_trigger_with_summariser`] is the dependency-injection
//! seam used by tests to pass a [`StubSummariser`]; it still propagates
//! errors (exit 24) so tests can assert failure paths directly.

use std::cell::RefCell;
use std::sync::Arc;

use crate::error::{SummariserFailureKind, TomeError};
use crate::paths::Paths;
use crate::summarise::prompts::validate_long_max_chars;
use crate::summarise::{LONG_MAX_CHARS, Summariser, build_summariser};
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
/// Returns `Ok(())` on success. ALL summariser failures are degraded to
/// a non-fatal warning at this layer so a post-commit summary error
/// never aborts a command that already successfully mutated workspace
/// state (issue #208).
///
/// ## Failure posture (updated per #208)
///
/// * `ModelMissing` — silent `debug!` no-op (unchanged). The model is
///   downloaded on-demand; absence is expected on fresh installs.
/// * Any other error (e.g. `BackendInitFailed`, `OutputEmpty`, etc.) —
///   logged as a `warn!` and degraded to `Ok(())`. The state mutation
///   already committed; crashing the command after the fact gives the
///   user a broken experience. The prior cached summary survives intact.
///   Run `tome workspace regen-summary` to retry.
///
/// The explicit [`crate::workspace::regen_summary`] path and the
/// [`regenerate_for_trigger_with_summariser`] DI variant still surface
/// errors — only this production-trigger wrapper degrades them.
///
/// Per FR-385, the caller MUST commit the skill-state mutation BEFORE
/// invoking this function.
pub fn regenerate_for_trigger(name: &WorkspaceName, paths: &Paths) -> Result<(), TomeError> {
    // `enabled` gate: if the user has explicitly set `[summariser] enabled = false`
    // in `~/.tome/config.toml`, skip auto-regeneration entirely. Use
    // `load_or_default` (defensive) — a malformed config.toml must NOT abort a
    // post-commit trigger and give the user a broken `plugin enable` experience.
    // Explicit `tome workspace regen-summary` is unaffected (it doesn't route
    // through this function).
    let cfg = crate::config::load_or_default(paths);
    if cfg.summariser.enabled == Some(false) {
        tracing::debug!(
            workspace = name.as_str(),
            "summariser disabled in config; skipping auto-regeneration",
        );
        return Ok(());
    }

    // Resolve effective long_max_chars from config. Use load_or_default so
    // a malformed config degrades to the default rather than aborting the trigger.
    let effective_long_max =
        validate_long_max_chars(cfg.summariser.long_max_chars.unwrap_or(LONG_MAX_CHARS));

    // Test-only override hook (gated by `SUMMARISER_OVERRIDE`). Production
    // paths never set the slot; the `if let Some(...)` collapses to the
    // real-summariser branch on every real invocation.
    let override_summariser = SUMMARISER_OVERRIDE.with(|slot| slot.borrow().as_ref().cloned());

    let result = if let Some(s) = override_summariser {
        regenerate_for_trigger_with_summariser(name, s.as_ref(), paths, effective_long_max)
    } else {
        // Select remote (when `[summariser] provider` is set) or the bundled
        // Qwen, with the TIGHTER trigger timeout so a slow remote can't make the
        // triggering command (e.g. `plugin enable`) feel hung. ALL failures —
        // a provider request failure (94), a provider-config error (93), or a
        // bundled model error — are degraded uniformly below (FR-027 /
        // issue #208).
        match build_summariser(&cfg, paths, true) {
            Ok(s) => {
                regenerate_for_trigger_with_summariser(name, s.as_ref(), paths, effective_long_max)
            }
            Err(e) => Err(e),
        }
    };

    // Single, uniform degrade site (issue #208): the skill-state mutation
    // already committed before this trigger ran, so a summary-regeneration
    // failure must NOT fail the command. A missing model is an expected,
    // silent no-op (the cached summary survives); any other failure is
    // logged at warn and swallowed. The explicit `tome workspace
    // regen-summary` path and the `_with_summariser` DI seam still surface
    // errors.
    match result {
        Ok(()) => Ok(()),
        Err(TomeError::SummariserFailure {
            kind: SummariserFailureKind::ModelMissing,
        }) => {
            tracing::debug!(
                workspace = name.as_str(),
                "summariser model not installed; skipping trigger regeneration (cached summary survives if any)",
            );
            Ok(())
        }
        Err(other) => {
            tracing::warn!(
                workspace = name.as_str(),
                error = %other,
                "summary regeneration failed after the state change committed; continuing (run `tome workspace regen-summary` to retry)",
            );
            Ok(())
        }
    }
}

/// Dependency-injection variant. Production code goes through
/// [`regenerate_for_trigger`]; tests pass a [`StubSummariser`] to
/// exercise the trigger plumbing without touching real models.
///
/// `long_max_chars` is the effective character cap, already validated
/// by the caller via `validate_long_max_chars`. Tests that bypass the
/// production trigger may pass `LONG_MAX_CHARS` directly.
pub fn regenerate_for_trigger_with_summariser(
    name: &WorkspaceName,
    summariser: &dyn Summariser,
    paths: &Paths,
    long_max_chars: usize,
) -> Result<(), TomeError> {
    let _outcome = workspace::regen_summary::regen(name, summariser, paths, long_max_chars)?;
    Ok(())
}

//! Phase 4 / US4.d-1 — T-B1: end-to-end coverage that the production
//! trigger sites actually invoke the summariser via
//! [`SummariserOverrideGuard`].
//!
//! `tests/summariser_triggers.rs` already covers the per-trigger
//! `_with_summariser` plumbing — but that test passes the stub
//! explicitly, bypassing the production `LlamaSummariser` construction
//! step. The reviewer flagged T-B1: the
//! [`SummariserOverrideGuard`] thread-local slot is the ONLY production
//! injection point, and it had ZERO coverage before US4.d-1.
//!
//! This file closes the gap by:
//!
//! 1. Installing a `CountingSummariser` (wraps `StubSummariser` so we
//!    keep the deterministic output) via [`SummariserOverrideGuard`].
//! 2. Driving the production trigger function
//!    [`tome::summarise::regenerate_for_trigger`] — the SAME entry
//!    point `commands::plugin::enable::run` / `disable::run` /
//!    `reindex::run_with_deps` / `catalog::update::run` call.
//! 3. Asserting the counter incremented, proving production code
//!    consulted the slot rather than constructing `LlamaSummariser`.
//!
//! Why not drive `commands::plugin::enable::run` directly? That entry
//! point calls `FastembedEmbedder::load` against the real embedder
//! model on disk — a CI-heavy path. The trigger logic we're verifying
//! is the slot-consultation in `regenerate_for_trigger`; bypassing the
//! embedder load keeps the test fast and isolated to the slot
//! semantics.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::common::{fabricate_models, lifecycle_paths};
use tempfile::TempDir;
use tome::error::TomeError;
use tome::summarise::{
    PluginSummariesInput, StubSummariser, Summariser, SummariserOutput, regenerate_for_trigger,
};
use tome::workspace::{self, WorkspaceName};

/// Counting wrapper that delegates to a `StubSummariser` for output.
/// Gives the test something to assert on (call counter) AND something
/// deterministic for the regen path to write into `settings.toml` /
/// `RULES.md`.
#[derive(Debug, Default)]
struct CountingSummariser {
    inner: StubSummariser,
    calls: Arc<AtomicU64>,
}

impl CountingSummariser {
    fn new() -> Self {
        Self::default()
    }

    fn counter(&self) -> Arc<AtomicU64> {
        self.calls.clone()
    }
}

impl Summariser for CountingSummariser {
    fn summarise(&self, input: &PluginSummariesInput) -> Result<SummariserOutput, TomeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.inner.summarise(input)
    }
}

#[test]
fn production_trigger_consults_summariser_override_slot() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);
    workspace::init::init(WorkspaceName::parse("override-ws").unwrap(), false, &paths)
        .expect("init workspace");

    let counter = {
        let counting = CountingSummariser::new();
        let counter = counting.counter();
        let _guard = tome::summarise::trigger::SummariserOverrideGuard::install(
            Arc::new(counting) as Arc<dyn Summariser>
        );

        // Drive the PRODUCTION trigger function — same entry point
        // `commands::plugin::enable::run` calls after a successful
        // `lifecycle::enable`. If the slot is ignored, this test
        // observes a counter of 0 (and likely an error from
        // `LlamaSummariser::new` for the missing GGUF).
        regenerate_for_trigger(&WorkspaceName::parse("override-ws").unwrap(), &paths)
            .expect("production trigger via override");

        counter
        // _guard drops here, clearing the slot for any later tests.
    };

    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "production `regenerate_for_trigger` must invoke the override summariser exactly once",
    );
}

#[test]
fn override_guard_drop_clears_slot_so_subsequent_call_falls_through() {
    // After `SummariserOverrideGuard` drops, the slot is empty and the
    // production path falls through to `LlamaSummariser::new`. With no
    // model on disk, the resulting `ModelMissing` is silenced by the
    // trigger-callers carve-out (the production no-op path), so we
    // assert `Ok(())` with zero counter increments on the OUTER probe.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // Deliberately no fabricate_models — summariser GGUF is absent so
    // the production constructor returns `ModelMissing`.
    workspace::init::init(WorkspaceName::parse("drop-ws").unwrap(), false, &paths)
        .expect("init workspace");

    let counter = Arc::new(AtomicU64::new(0));

    // Inner scope installs guard, fires once, drops guard.
    {
        let counting = CountingSummariser {
            inner: StubSummariser::default(),
            calls: counter.clone(),
        };
        let _guard = tome::summarise::trigger::SummariserOverrideGuard::install(
            Arc::new(counting) as Arc<dyn Summariser>
        );
        regenerate_for_trigger(&WorkspaceName::parse("drop-ws").unwrap(), &paths)
            .expect("trigger via override");
    }

    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "inner call must have hit the override",
    );

    // After-drop call: slot is empty, falls through to the production
    // constructor, model-missing is silent no-op.
    let after = regenerate_for_trigger(&WorkspaceName::parse("drop-ws").unwrap(), &paths);
    assert!(
        after.is_ok(),
        "after guard drop, model-missing must collapse to silent Ok via the trigger-callers carve-out, got {after:?}",
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "counter must NOT increment after guard drop — slot was cleared",
    );
}

//! Deterministic, model-free `StubSummariser` used by the test suite.
//!
//! Mirrors the `StubEmbedder` / `StubReranker` shape in
//! `src/embedding/stub.rs`. The stub:
//!
//! * Records its call count (shared between clones via `Arc<AtomicU64>`)
//!   so tests can assert "summariser invoked exactly N times" for
//!   trigger correctness (`tests/summariser_triggers.rs` in US4).
//! * Returns deterministic content-addressed `short` + `long` strings
//!   so cache-shape tests can pin the exact bytes. The algorithm is:
//!
//!   ```text
//!   topics = each plugin's skill names, flattened, in input order
//!   short  = topics.join(", ")
//!   long   = "This workspace covers: {topics}. Call search_skills when working on these topics."
//!   ```
//!
//! The stub is `#[doc(hidden)] pub` so integration tests under
//! `tests/` (which compile without `cfg(test)` visibility into the
//! crate) can reach it — same discipline as `StubEmbedder`. LTO drops
//! the type from any release binary that doesn't reference it; the
//! production code path never touches it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::TomeError;

use super::{PluginSummariesInput, Summariser, SummariserOutput};

/// Deterministic test summariser. Two clones share the same call
/// counter via `Arc<AtomicU64>` so observers across handles agree on
/// the total — the same pattern `StubEmbedder` uses.
#[doc(hidden)]
#[derive(Debug, Clone, Default)]
pub struct StubSummariser {
    call_count: Arc<AtomicU64>,
}

impl StubSummariser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of `summarise` calls observed so far. Lets tests assert
    /// the summariser fired exactly once per trigger (or zero times on
    /// a no-op path).
    pub fn call_count(&self) -> u64 {
        self.call_count.load(Ordering::SeqCst)
    }
}

impl Summariser for StubSummariser {
    fn summarise(&self, input: &PluginSummariesInput) -> Result<SummariserOutput, TomeError> {
        self.call_count.fetch_add(1, Ordering::SeqCst);
        let topics: Vec<String> = input
            .plugins
            .iter()
            .flat_map(|p| p.skills.iter().map(|s| s.name.clone()))
            .collect();
        let topics_joined = topics.join(", ");
        Ok(SummariserOutput {
            short: topics_joined.clone(),
            long: format!(
                "This workspace covers: {topics_joined}. Call search_skills when working on these topics."
            ),
        })
    }
}

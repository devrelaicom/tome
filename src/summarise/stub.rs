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
        // `short` is a deterministic comma-separated list of the
        // workspace's skill names — it stands in for the real model's
        // topics-and-tasks output. It varies with the workspace yet stays
        // byte-stable for a given input so cache-shape tests can pin it.
        let short = if input.plugins.is_empty() {
            String::new()
        } else {
            input
                .plugins
                .iter()
                .flat_map(|p| p.skills.iter().map(|s| s.name.clone()))
                .collect::<Vec<_>>()
                .join(", ")
        };
        Ok(SummariserOutput {
            long: format!(
                "This workspace covers: {short}. Call search_skills when working on these topics."
            ),
            short,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::summarise::{PluginSummaryItem, SkillSummaryItem};

    #[test]
    fn stub_short_is_comma_joined_skill_names() {
        let input = PluginSummariesInput {
            plugins: vec![PluginSummaryItem {
                catalog: "c".into(),
                plugin: "p".into(),
                description: "d".into(),
                skills: vec![
                    SkillSummaryItem {
                        name: "alpha".into(),
                        description: "x".into(),
                    },
                    SkillSummaryItem {
                        name: "beta".into(),
                        description: "y".into(),
                    },
                ],
            }],
        };
        let out = StubSummariser::default().summarise(&input).unwrap();
        assert_eq!(out.short, "alpha, beta");
    }
}

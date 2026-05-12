//! Deterministic, model-free [`Embedder`] / [`Reranker`] implementations
//! used by the integration test suite.
//!
//! Construction strategy (research §R10):
//!
//! * Hash the input text with SHA-256 (32 bytes).
//! * Tile the hash across the 384-element output vector.
//! * Treat consecutive bytes as little-endian u32 / u16 fragments, normalise
//!   to `[-1.0, 1.0]`, then L2-normalise the whole vector so cosine
//!   similarity matches dot product.
//!
//! Properties tested in `tests/embedding_stub.rs`:
//!
//! * **Determinism** — the same input always produces the same vector.
//! * **Distinguishability** — different inputs produce vectors whose cosine
//!   similarity is `< 0.99` (a real model would too; the stub mirrors that).
//! * **Length** — every output is exactly 384 elements.
//!
//! The stub is intentionally compiled into the release binary as well; LTO
//! eliminates it from any binary that does not reference it, and the
//! `#[doc(hidden)]` markers keep it off the public API surface. See the plan's
//! complexity-tracking justification (principle VIII boundary case).

use sha2::{Digest, Sha256};

use crate::embedding::{Embedder, Reranker, Scored};
use crate::error::TomeError;
use crate::index::query::Candidate;

const VECTOR_DIM: usize = 384;

/// Deterministic embedder. Two clones of `StubEmbedder` produce identical
/// vectors for identical input.
#[doc(hidden)]
#[derive(Debug, Clone, Default)]
pub struct StubEmbedder;

impl StubEmbedder {
    pub fn new() -> Self {
        Self
    }
}

impl Embedder for StubEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, TomeError> {
        Ok(deterministic_vector(text))
    }

    fn model_name(&self) -> &str {
        "stub-embedder"
    }

    fn model_version(&self) -> &str {
        "0"
    }
}

/// Identity reranker — preserves the input order and assigns a score equal
/// to `1.0 - distance` for each candidate (so higher = better, matching the
/// real reranker's convention).
#[doc(hidden)]
#[derive(Debug, Clone, Default)]
pub struct StubReranker;

impl StubReranker {
    pub fn new() -> Self {
        Self
    }
}

impl Reranker for StubReranker {
    fn rerank(&self, _query: &str, candidates: Vec<Candidate>) -> Result<Vec<Scored>, TomeError> {
        Ok(candidates
            .into_iter()
            .map(|c| {
                let score = 1.0 - c.distance;
                Scored {
                    candidate: c,
                    score,
                }
            })
            .collect())
    }

    fn model_name(&self) -> &str {
        "stub-reranker"
    }

    fn model_version(&self) -> &str {
        "0"
    }
}

/// Reverse-order reranker — same shape as [`StubReranker`] but flips the
/// input order. Tests use this to distinguish "the reranker stage ran" from
/// "the embedder stage ran" (since the embedder ordering passes through).
#[doc(hidden)]
#[derive(Debug, Clone, Default)]
pub struct ReverseStubReranker;

impl ReverseStubReranker {
    pub fn new() -> Self {
        Self
    }
}

impl Reranker for ReverseStubReranker {
    fn rerank(
        &self,
        _query: &str,
        mut candidates: Vec<Candidate>,
    ) -> Result<Vec<Scored>, TomeError> {
        candidates.reverse();
        let n = candidates.len() as f32;
        Ok(candidates
            .into_iter()
            .enumerate()
            .map(|(i, c)| Scored {
                candidate: c,
                score: (n - i as f32) / n.max(1.0),
            })
            .collect())
    }

    fn model_name(&self) -> &str {
        "reverse-stub-reranker"
    }

    fn model_version(&self) -> &str {
        "0"
    }
}

/// Convenience constructor: `(embedder, reranker)` pair for plumbing into
/// integration tests.
#[doc(hidden)]
pub fn make_test_pair() -> (StubEmbedder, StubReranker) {
    (StubEmbedder, StubReranker)
}

fn deterministic_vector(text: &str) -> Vec<f32> {
    // Build a 32-byte SHA-256, then tile it across the 384-element output as
    // 4-byte chunks reinterpreted as i32 → f32. This is the construction
    // research §R10 calls out as a "simpler XOR-based hash that distributes
    // well"; the cosine distinguishability tests in tests/embedding_stub.rs
    // pin the actual guarantee.
    let digest = Sha256::digest(text.as_bytes());
    let bytes: [u8; 32] = digest.into();

    let mut out = vec![0.0f32; VECTOR_DIM];
    for (i, slot) in out.iter_mut().enumerate() {
        // Stride a window through the digest; mix in the index so different
        // positions get distinct values even when the digest happens to
        // repeat. The cast normalises i32 → f32 in [-1, 1].
        let off = (i * 4) % 32;
        let chunk = [
            bytes[off],
            bytes[(off + 1) % 32],
            bytes[(off + 2) % 32],
            bytes[(off + 3) % 32],
        ];
        let raw = i32::from_le_bytes(chunk).wrapping_add(i as i32);
        *slot = (raw as f32) / (i32::MAX as f32);
    }

    // L2-normalise so cosine similarity = dot product.
    let norm: f32 = out.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
    for x in &mut out {
        *x /= norm;
    }
    out
}

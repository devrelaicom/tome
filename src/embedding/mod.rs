//! Embedding pipeline — embedder, reranker, model registry, and download.
//!
//! [`Embedder`] and [`Reranker`] are the boundary traits between Tome and
//! the ONNX-backed [`fastembed`] crate. The production implementations live
//! in [`fastembed`](self::fastembed); a deterministic, model-free [`stub`]
//! is available for tests (constitution principle VIII deviation — the
//! embedder is at an external system boundary).
//!
//! Spec: data-model.md §7 (`ModelManifest`), research §R5 (registry), §R6
//! (cancellation), §R10 (stub).

pub mod download;
pub mod fastembed;
pub mod registry;
pub mod runtime;
pub mod stub;

use crate::error::TomeError;
use crate::index::query::Candidate;

pub use registry::{MODEL_REGISTRY, ModelEntry, ModelKind, ModelManifest};

/// Produces a 384-dimensional embedding for arbitrary text.
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, TomeError>;

    /// Stable identifier of the embedder model. Recorded in the `meta`
    /// table at bootstrap and compared against on every open to detect drift.
    fn model_name(&self) -> &str;
    fn model_version(&self) -> &str;
}

/// Re-orders candidate skill rows by a cross-encoder score. The input order
/// reflects raw embedding similarity; the output order reflects the
/// reranker's verdict.
pub trait Reranker: Send + Sync {
    fn rerank(&self, query: &str, candidates: Vec<Candidate>) -> Result<Vec<Scored>, TomeError>;

    fn model_name(&self) -> &str;
    fn model_version(&self) -> &str;
}

/// A candidate with a reranker score attached. Higher scores are better.
#[derive(Debug, Clone)]
pub struct Scored {
    pub candidate: Candidate,
    pub score: f32,
}

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
pub mod profile;
pub mod registry;
pub mod remote;
pub mod runtime;
pub mod stub;

use crate::error::TomeError;
use crate::index::query::Candidate;

pub use profile::Profile;
pub use registry::{MODEL_REGISTRY, ModelEntry, ModelKind, ModelManifest};
pub use remote::{
    REMOTE_EMBEDDER_VERSION, RemoteEmbedder, build_embedder, embedder_seed, validate_embedding,
};

/// Produces an embedding vector for arbitrary text. The output dimension
/// depends on the active model profile (small: 384-d, medium: 768-d, large: 1024-d).
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>, TomeError>;

    /// Stable identifier of the embedder model. Recorded in the `meta`
    /// table at bootstrap and compared against on every open to detect drift.
    fn model_name(&self) -> &str;
    fn model_version(&self) -> &str;

    /// Phase 12 / US2: the output dimension this embedder has established for
    /// the current run, if it is a REMOTE embedder that needs its dimension
    /// persisted to `meta.embedder_dimension` after a reindex (FR-015a).
    ///
    /// Defaulted to `None`: a BUNDLED embedder NEVER persists this key (NFR-006
    /// — a new meta row would change stored artefacts), and a remote embedder
    /// that has not yet embedded anything returns `None` too. The reindex path
    /// reads this AFTER its embed loop to persist the dimension. Overridden only
    /// by [`remote::RemoteEmbedder`].
    fn established_dimension(&self) -> Option<usize> {
        None
    }
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

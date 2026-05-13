//! Pinned model registry — `MODEL_REGISTRY` carries one entry per model
//! Tome can download. Values are intentionally compile-time constants so
//! `cargo build` is enough to ensure a downstream Tome install agrees on
//! which model bytes are canonical.
//!
//! `ModelManifest` is the Tome-owned, strict on-disk record written into
//! `${XDG_DATA_HOME}/tome/models/<name>/manifest.json` after a successful
//! verified download (FR-013a, data-model §7).
//!
//! The pinned SHA-256 + size_bytes values below are real upstream artefact
//! digests, fetched and verified at the start of Phase 3 (slice 1) against
//! the canonical Hugging Face URLs. Downloads enforce both the pinned hash
//! and pinned size; any drift surfaces as `ModelChecksumMismatch`.
//!
//! Spec: data-model.md §7, research §R5.

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy)]
pub struct ModelEntry {
    pub name: &'static str,
    pub version: &'static str,
    pub kind: ModelKind,
    pub source_url: &'static str,
    pub sha256: &'static str,
    pub size_bytes: u64,
    pub licence: &'static str,
    /// Relative paths inside the model directory once installation completes.
    pub files: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum ModelKind {
    Embedder,
    Reranker,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelManifest {
    pub name: String,
    pub version: String,
    pub kind: ModelKind,
    pub source_url: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub licence: String,
    pub files: Vec<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub installed_at: OffsetDateTime,
}

/// Embedder + reranker the rest of Tome assumes are pinned. Hashes and sizes
/// are real upstream digests verified at the start of Phase 3 slice 1.
pub const MODEL_REGISTRY: &[ModelEntry] = &[
    ModelEntry {
        name: "bge-small-en-v1.5",
        version: "1.5",
        kind: ModelKind::Embedder,
        source_url: "https://huggingface.co/qdrant/bge-small-en-v1.5-onnx-Q/resolve/main/model_optimized.onnx",
        sha256: "51f1bd0addd6e859e42c2c8021a5e5461385bb676a649f4b269aa445449f2431",
        size_bytes: 66_465_124,
        licence: "MIT",
        files: &["model.onnx", "tokenizer.json"],
    },
    // Source moved: BAAI/bge-reranker-base no longer hosts a quantised ONNX
    // (only fp32 model.onnx remains upstream). The onnx-community group is
    // the canonical HF mirror for ONNX-quantised variants of community
    // models; weight (~280 MB INT8) matches the spec target.
    ModelEntry {
        name: "bge-reranker-base",
        version: "base",
        kind: ModelKind::Reranker,
        source_url: "https://huggingface.co/onnx-community/bge-reranker-base-ONNX/resolve/main/onnx/model_quantized.onnx",
        sha256: "46a1bb4cf46ff1e300d27589d620141fbf04fc0eaf8e7bb6dea5e044475ff387",
        size_bytes: 279_252_659,
        licence: "MIT",
        files: &["model.onnx", "tokenizer.json"],
    },
];

/// Look up an entry by name. Returns `None` if the name is not registered.
pub fn lookup(name: &str) -> Option<&'static ModelEntry> {
    MODEL_REGISTRY.iter().find(|m| m.name == name)
}

impl ModelEntry {
    /// True iff the registry entry's checksum is still the all-zero
    /// placeholder. Download paths must refuse to install in that case.
    pub fn has_placeholder_checksum(&self) -> bool {
        self.sha256.chars().all(|c| c == '0')
    }
}

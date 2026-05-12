//! Pinned model registry — `MODEL_REGISTRY` carries one entry per model
//! Tome can download. Values are intentionally compile-time constants so
//! `cargo build` is enough to ensure a downstream Tome install agrees on
//! which model bytes are canonical.
//!
//! `ModelManifest` is the Tome-owned, strict on-disk record written into
//! `${XDG_DATA_HOME}/tome/models/<name>/manifest.json` after a successful
//! verified download (FR-013a, data-model §7).
//!
//! Note: the pinned SHA-256 + size_bytes values below are placeholders to
//! be replaced when the first end-to-end model-download integration test
//! lands (slice 5 follow-up). CI verifies the pinned values match the
//! upstream artefacts; downloads against a placeholder hash will fail
//! with `ModelChecksumMismatch` until the values are updated.
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

/// Embedder + reranker the rest of Tome assumes are pinned. Slice-5 follow-up
/// (T057 integration test) replaces the placeholder hash + size with verified
/// values from the upstream Hugging Face artefact.
pub const MODEL_REGISTRY: &[ModelEntry] = &[
    ModelEntry {
        name: "bge-small-en-v1.5",
        version: "1.5",
        kind: ModelKind::Embedder,
        source_url: "https://huggingface.co/qdrant/bge-small-en-v1.5-onnx-Q/resolve/main/model_optimized.onnx",
        sha256: "0000000000000000000000000000000000000000000000000000000000000000",
        size_bytes: 0,
        licence: "MIT",
        files: &["model.onnx", "tokenizer.json"],
    },
    ModelEntry {
        name: "bge-reranker-base",
        version: "base",
        kind: ModelKind::Reranker,
        source_url: "https://huggingface.co/BAAI/bge-reranker-base/resolve/main/onnx/model_quantized.onnx",
        sha256: "0000000000000000000000000000000000000000000000000000000000000000",
        size_bytes: 0,
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

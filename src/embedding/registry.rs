//! Pinned model registry — `MODEL_REGISTRY` carries one entry per model
//! Tome can download. Values are intentionally compile-time constants so
//! `cargo build` is enough to ensure a downstream Tome install agrees on
//! which model bytes are canonical.
//!
//! `ModelManifest` is the Tome-owned, strict on-disk record written into
//! `${XDG_DATA_HOME}/tome/models/<name>/manifest.toml` after a successful
//! verified download (FR-013a, data-model §7). Phase 8 moved this from
//! `manifest.json` to TOML for consistency with `tome-plugin.toml`; a
//! pre-cutover `manifest.json` is migrated by `doctor --fix`.
//!
//! The pinned SHA-256 + size_bytes values below are real upstream artefact
//! digests, fetched and verified at the start of Phase 3 (slice 1) against
//! the canonical Hugging Face URLs. Downloads enforce the pinned SHA-256;
//! any mismatch surfaces as `ModelChecksumMismatch`. `size_bytes` drives the
//! progress-bar total and is not independently verified.
//!
//! Spec: data-model.md §7, research §R5.

use std::path::Path;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::error::TomeError;

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
    /// `files[0]` is the primary artefact (fetched from [`source_url`]);
    /// `files[1..]` are the non-primary files, fetched from [`aux_urls`].
    pub files: &'static [&'static str],
    /// Remote URLs for the non-primary files (`files[1..]`) in the SAME
    /// ORDER. The primary file's URL is [`source_url`]. Single-file models
    /// (the summariser) use `&[]`. The positional `files[1..]` ↔ `aux_urls`
    /// pairing is enforced by `model_registry_invariant` (a fast test) and a
    /// `debug_assert!` in `download::download_model`.
    pub aux_urls: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", deny_unknown_fields)]
pub enum ModelKind {
    Embedder,
    Reranker,
    /// Phase 4 — Qwen2.5-0.5B-Instruct GGUF, served via `llama-cpp-2`.
    /// Added in F6 alongside the summariser skeleton; the production
    /// download path lands in US4.a with the real SHA-256.
    Summariser,
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

impl ModelManifest {
    /// Serialize to the on-disk **TOML** form (Phase 8 cutover — the model
    /// manifest moves `manifest.json` → `manifest.toml` for consistency with
    /// `tome-plugin.toml`; same fields, still strict). `installed_at` round-trips
    /// as an RFC3339 string.
    pub fn to_toml(&self, file: &Path) -> Result<String, TomeError> {
        toml::to_string(self).map_err(|e| TomeError::ModelRegistrationParseError {
            file: file.to_path_buf(),
            message: format!("serialise: {e}"),
        })
    }

    /// Parse from on-disk TOML bytes (strict, `deny_unknown_fields`).
    pub fn from_toml_slice(file: &Path, bytes: &[u8]) -> Result<Self, TomeError> {
        let text =
            std::str::from_utf8(bytes).map_err(|e| TomeError::ModelRegistrationParseError {
                file: file.to_path_buf(),
                message: format!("not valid UTF-8: {e}"),
            })?;
        toml::from_str(text).map_err(|e| TomeError::ModelRegistrationParseError {
            file: file.to_path_buf(),
            message: e.to_string(),
        })
    }
}

/// Embedder + reranker the rest of Tome assumes are pinned. Hashes and sizes
/// are real upstream digests verified at the start of Phase 3 slice 1.
pub const MODEL_REGISTRY: &[ModelEntry] = &[
    ModelEntry {
        name: "bge-small-en-v1.5",
        version: "1.5",
        kind: ModelKind::Embedder,
        // CPU-COMPATIBLE PIN (F-MODEL-ONNX-CPU, Phase 7): the previous pin —
        // qdrant's `bge-small-en-v1.5-onnx-Q/model_optimized.onnx` — ships an
        // `ort_config.json` with `optimize_for_gpu:true` / `fp16:true` /
        // transformer-specific graph fusions. On Tome's CPU-only `ort` stack
        // `FastembedEmbedder::embed` failed at inference with
        // `Missing Input: encoder.layer.0.attention.output.LayerNorm.weight`
        // inside a fused `SkipLayerNormalization` op, so `tome query` + the MCP
        // `search_skills` tool returned errors despite a successful `load`.
        // Xenova/bge-small-en-v1.5 is the canonical self-contained CPU INT8
        // graph (the same publisher fastembed-rs uses for this model); no
        // `ort_config.json`, plain dynamic-quantised ops that run on CPU `ort`.
        // sha256 below is the COMPUTED digest of the downloaded artefact
        // (`shasum -a 256`); it matches the upstream-claimed value. The entry
        // NAME + VERSION are unchanged so the index `meta`/`MetaSeed` identity
        // is preserved (no index drift); only source_url/sha256/size_bytes and
        // the aux_urls were re-pinned. The `files` local-name set is unchanged.
        source_url: "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/onnx/model_quantized.onnx",
        sha256: "6c9c6101a956d62dfb5e7190c538226c0c5bb9cb27b651234b6df063ee7dbfe4",
        size_bytes: 34_014_426,
        licence: "MIT",
        // tokenizer.json is REQUIRED — fastembed's `build_tokenizer_files`
        // reads it via `read_required`; without it `FastembedEmbedder::load`
        // returns ModelMissing. config.json / special_tokens_map.json /
        // tokenizer_config.json are the standard fastembed layout, read via
        // `read_optional`; we ship them so truncation length + special-token
        // handling match upstream. All five are pinned, verified 200 upstream.
        files: &[
            "model.onnx",
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ],
        // Positional with files[1..]. The primary .onnx lives under /onnx/;
        // the tokenizer + config files live at the Xenova repo root (no /onnx/
        // prefix) — hence these URLs are NOT just `source_url`'s dir + filename.
        aux_urls: &[
            "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/tokenizer.json",
            "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/config.json",
            "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/special_tokens_map.json",
            "https://huggingface.co/Xenova/bge-small-en-v1.5/resolve/main/tokenizer_config.json",
        ],
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
        // Same layout as the embedder above. All four non-primary files were
        // verified 200 at the onnx-community mirror's /resolve/main/ base.
        // NOTE: the primary .onnx lives under /onnx/, but tokenizer + config
        // files live at the repo root (no /onnx/ prefix) — hence the URLs
        // here are NOT just `source_url`'s dir + filename.
        files: &[
            "model.onnx",
            "tokenizer.json",
            "config.json",
            "special_tokens_map.json",
            "tokenizer_config.json",
        ],
        // Positional with files[1..].
        aux_urls: &[
            "https://huggingface.co/onnx-community/bge-reranker-base-ONNX/resolve/main/tokenizer.json",
            "https://huggingface.co/onnx-community/bge-reranker-base-ONNX/resolve/main/config.json",
            "https://huggingface.co/onnx-community/bge-reranker-base-ONNX/resolve/main/special_tokens_map.json",
            "https://huggingface.co/onnx-community/bge-reranker-base-ONNX/resolve/main/tokenizer_config.json",
        ],
    },
    // Phase 4 — Summariser. The SHA-256 and size below were computed
    // against the canonical Hugging Face artefact (download via
    // `curl -L <SUMMARISER_SOURCE_URL>` + `shasum -a 256`) on
    // 2026-05-26 as part of US4.d-1 (PR #74). The named constants in
    // `src/summarise/registry.rs` mirror these values; the two
    // sources MUST agree (the
    // `registry::tests::summariser_entry_is_in_global_registry` test
    // catches drift). The download path's `has_placeholder_checksum`
    // gate now passes for this entry — `tome models download` will
    // install it normally; a tampered artefact surfaces as
    // `ModelChecksumMismatch` (exit 32) at install time.
    ModelEntry {
        name: "qwen2.5-0.5b-instruct",
        version: "0.5b-Q4_K_M",
        kind: ModelKind::Summariser,
        source_url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf",
        sha256: "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db",
        size_bytes: 491_400_032,
        licence: "Apache-2.0",
        files: &["model.gguf"],
        // Single-file model: the GGUF carries its own tokenizer. No aux files.
        aux_urls: &[],
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

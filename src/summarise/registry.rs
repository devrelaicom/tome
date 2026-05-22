//! Summariser model registry entry.
//!
//! Phase 4 introduces a third inference runtime (llama-cpp-2) alongside
//! the Phase 2 embedder + reranker. The summariser model is a GGUF
//! quantised Qwen2.5-0.5B-Instruct — small enough to load on a typical
//! laptop, large enough to write coherent rule-section prose for
//! `RULES.md` (research §R-3).
//!
//! Storage layout mirrors the embedder/reranker pattern from
//! `src/embedding/registry.rs`:
//!
//! ```text
//! <root>/models/qwen2.5-0.5b-instruct/
//!     model.gguf       — the primary artefact (size pinned)
//!     manifest.json    — strict, written atomically post-download
//! ```
//!
//! The `kind` is `ModelKind::Summariser` (added to the closed enum in F6
//! alongside this entry). Existing call sites that walk `MODEL_REGISTRY`
//! (status check, doctor cache audit, models list/download/remove)
//! handle the new variant via the same exhaustive `match`es that already
//! cover embedder/reranker.
//!
//! **Checksum placeholder**: F6 ships a skeleton — the production
//! download path will be wired in US4.a. Until the model is fetched
//! against the canonical Hugging Face URL and its SHA-256 + size_bytes
//! recorded, the entry below carries the all-zero placeholder hash. The
//! download path refuses to install when `ModelEntry::has_placeholder_checksum`
//! returns true (existing F2-era guard in `embedding::download::download_model`),
//! so a stray invocation surfaces as `ModelCorrupt` (exit 31) with the
//! "registry checksum is an unverified placeholder" message rather than
//! silently installing untrusted bytes.

use crate::embedding::registry::{ModelEntry, ModelKind};

/// Stable identifier — matches the on-disk directory name and the value
/// recorded in `index.db.meta` as `summariser_name`. The repeated `0.5b`
/// after `qwen2.5` is the canonical upstream name (Hugging Face model
/// card: `Qwen/Qwen2.5-0.5B-Instruct-GGUF`).
pub const SUMMARISER_NAME: &str = "qwen2.5-0.5b-instruct";

/// Pinned upstream version. Updates when the Hugging Face revision the
/// `SHA256_PLACEHOLDER` is recomputed against changes.
pub const SUMMARISER_VERSION: &str = "0.5b-Q4_K_M";

/// Phase 4 / F6 placeholder. US4.a replaces this with the real SHA-256
/// after fetching the canonical GGUF and recording its digest. Until
/// then `ModelEntry::has_placeholder_checksum` returns true for this
/// entry — the download path refuses with `ModelCorrupt` (exit 31).
pub const SHA256_PLACEHOLDER: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// Approximate size of the Q4_K_M quantisation. Real value is pinned in
/// US4.a alongside the SHA-256. Until then the size is advisory; the
/// streaming download does not gate on it (the SHA-256 gate is
/// authoritative).
pub const SUMMARISER_SIZE_BYTES_APPROX: u64 = 400_000_000;

/// Canonical upstream URL. The exact filename inside the Hugging Face
/// repo (`qwen2.5-0.5b-instruct-q4_k_m.gguf`) is pinned here so US4.a
/// only needs to flip the checksum + size_bytes; the URL stays put.
pub const SUMMARISER_SOURCE_URL: &str = "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf";

/// `ModelEntry` describing the summariser model. Returned by
/// [`summariser_entry`] and registered into the global `MODEL_REGISTRY`
/// via the `inventory!` macro pattern — but Phase 2's `MODEL_REGISTRY`
/// is a `const &[ModelEntry]`, not a runtime collection, so F6
/// re-exports `MODEL_REGISTRY` from `embedding::registry` and the
/// summariser entry is appended there directly.
pub const SUMMARISER_ENTRY: ModelEntry = ModelEntry {
    name: SUMMARISER_NAME,
    version: SUMMARISER_VERSION,
    kind: ModelKind::Summariser,
    source_url: SUMMARISER_SOURCE_URL,
    sha256: SHA256_PLACEHOLDER,
    size_bytes: SUMMARISER_SIZE_BYTES_APPROX,
    licence: "Apache-2.0",
    files: &["model.gguf"],
};

/// Look up the summariser registry entry. Convenience wrapper over
/// `embedding::registry::lookup(SUMMARISER_NAME)` returning the static
/// reference so callers don't have to handle `Option` for a value that
/// the registry is guaranteed to contain.
pub fn summariser_entry() -> &'static ModelEntry {
    crate::embedding::registry::lookup(SUMMARISER_NAME)
        .expect("summariser entry is registered in MODEL_REGISTRY")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::registry::MODEL_REGISTRY;

    #[test]
    fn summariser_entry_is_in_global_registry() {
        let entry = summariser_entry();
        assert_eq!(entry.name, SUMMARISER_NAME);
        assert_eq!(entry.kind, ModelKind::Summariser);
    }

    #[test]
    fn registry_includes_three_kinds() {
        // F6 invariant: registry now carries an embedder, a reranker, and
        // a summariser. Status / doctor / models commands exhaustively
        // match on `kind`; this test would catch a stray drop.
        let has_embedder = MODEL_REGISTRY.iter().any(|e| e.kind == ModelKind::Embedder);
        let has_reranker = MODEL_REGISTRY.iter().any(|e| e.kind == ModelKind::Reranker);
        let has_summariser = MODEL_REGISTRY
            .iter()
            .any(|e| e.kind == ModelKind::Summariser);
        assert!(has_embedder && has_reranker && has_summariser);
    }

    #[test]
    fn summariser_entry_carries_placeholder_until_us4_a() {
        // F6 ships with a placeholder hash. The download path's
        // `has_placeholder_checksum` guard refuses to install; flipping
        // this to a real digest is US4.a's job.
        assert!(summariser_entry().has_placeholder_checksum());
    }
}

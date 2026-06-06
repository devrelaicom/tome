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
//!     manifest.toml    — strict, written atomically post-download
//! ```
//!
//! The `kind` is `ModelKind::Summariser` (added to the closed enum in F6
//! alongside this entry). Existing call sites that walk `MODEL_REGISTRY`
//! (status check, doctor cache audit, models list/download/remove)
//! handle the new variant via the same exhaustive `match`es that already
//! cover embedder/reranker.
//!
//! **Checksum pinning**: the SHA-256 and size_bytes below were computed
//! against the canonical Hugging Face artefact on 2026-05-26 (US4.d-1,
//! PR #74). The values are duplicated in `src/embedding/registry.rs`'s
//! `MODEL_REGISTRY` entry; the
//! [`tests::summariser_entry_is_in_global_registry`] test catches drift
//! between the two sources. The download path's
//! `has_placeholder_checksum` gate no longer trips for this entry —
//! `tome models download` installs normally and a tampered artefact
//! surfaces as `ModelChecksumMismatch` (exit 32) at install time.

use crate::embedding::registry::{ModelEntry, ModelKind};

/// Stable identifier — matches the on-disk directory name and the value
/// recorded in `index.db.meta` as `summariser_name`. The repeated `0.5b`
/// after `qwen2.5` is the canonical upstream name (Hugging Face model
/// card: `Qwen/Qwen2.5-0.5B-Instruct-GGUF`).
pub const SUMMARISER_NAME: &str = "qwen2.5-0.5b-instruct";

/// Pinned upstream version. Updates when the Hugging Face revision the
/// [`SUMMARISER_SHA256`] is recomputed against changes.
pub const SUMMARISER_VERSION: &str = "0.5b-Q4_K_M";

/// Pinned SHA-256 of the canonical Qwen2.5-0.5B-Instruct-GGUF Q4_K_M
/// artefact. Computed against `SUMMARISER_SOURCE_URL` on 2026-05-26
/// (US4.d-1, PR #74). The same digest is mirrored verbatim in
/// `MODEL_REGISTRY`'s summariser entry; the drift test in this module
/// keeps them aligned.
pub const SUMMARISER_SHA256: &str =
    "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db";

/// Exact size in bytes of the canonical Qwen2.5-0.5B-Instruct-GGUF
/// Q4_K_M artefact. Verified against the source URL on 2026-05-26
/// (US4.d-1). The download streaming path SHA-checks the bytes
/// (authoritative); size is a secondary cross-check.
pub const SUMMARISER_SIZE_BYTES: u64 = 491_400_032;

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
    sha256: SUMMARISER_SHA256,
    size_bytes: SUMMARISER_SIZE_BYTES,
    licence: "Apache-2.0",
    files: &["model.gguf"],
    // Single-file model: the GGUF carries its own tokenizer. No aux files.
    aux_urls: &[],
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
    fn summariser_entry_has_real_checksum_not_placeholder() {
        // US4.d-1 (PR #74) replaced the F6 all-zero placeholder with
        // the real SHA-256 of the canonical Qwen2.5-0.5B-Instruct
        // Q4_K_M artefact. The download path's
        // `has_placeholder_checksum` guard MUST report false now;
        // otherwise installs fall back to the "registry checksum is
        // an unverified placeholder" failure mode (regression net for
        // a future placeholder reintroduction).
        let entry = summariser_entry();
        assert!(!entry.has_placeholder_checksum());
        assert_eq!(entry.sha256, SUMMARISER_SHA256);
        assert_eq!(entry.size_bytes, SUMMARISER_SIZE_BYTES);
    }

    #[test]
    fn summariser_entry_in_global_registry_matches_named_constants() {
        // Drift catcher: the named constants in this module and the
        // hard-coded `MODEL_REGISTRY` literals in
        // `src/embedding/registry.rs` are two sources for the same
        // value. Either source updating without the other should fail
        // here.
        let entry = summariser_entry();
        assert_eq!(entry.sha256, SUMMARISER_SHA256);
        assert_eq!(entry.size_bytes, SUMMARISER_SIZE_BYTES);
        assert_eq!(entry.version, SUMMARISER_VERSION);
        assert_eq!(entry.source_url, SUMMARISER_SOURCE_URL);
    }
}

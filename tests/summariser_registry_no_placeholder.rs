//! Phase 4 / US4.d-1 — regression net for the C-B1 blocker fix.
//!
//! The summariser registry entry shipped with an all-zero placeholder
//! SHA-256 from F6 through US4.c. US4.d-1 (PR #74) flipped it to the
//! real digest of the canonical Qwen2.5-0.5B-Instruct-GGUF Q4_K_M
//! artefact. A future regression that re-introduces the placeholder
//! (intentional or accidental) would silently re-enable the
//! "ModelChecksumMismatch on every install" failure mode — the
//! reviewer-flagged blocker. This test catches that.
//!
//! Belt-and-braces against `src/embedding/registry.rs` AND
//! `src/summarise/registry.rs` drifting apart: both are checked.

use tome::embedding::registry::{MODEL_REGISTRY, ModelKind};
use tome::summarise::registry::{
    SUMMARISER_NAME, SUMMARISER_SHA256, SUMMARISER_SIZE_BYTES, summariser_entry,
};

#[test]
fn registry_sha256_is_not_all_zero() {
    let entry = summariser_entry();
    assert!(
        !entry.has_placeholder_checksum(),
        "summariser registry entry must not carry the all-zero placeholder SHA-256",
    );
    assert_ne!(
        entry.sha256, "0000000000000000000000000000000000000000000000000000000000000000",
        "literal all-zero hash must not be re-introduced",
    );
}

#[test]
fn registry_sha256_matches_pinned_constant() {
    let entry = summariser_entry();
    assert_eq!(
        entry.sha256, SUMMARISER_SHA256,
        "MODEL_REGISTRY hash diverged from the named SUMMARISER_SHA256 constant",
    );
    assert_eq!(
        entry.size_bytes, SUMMARISER_SIZE_BYTES,
        "MODEL_REGISTRY size_bytes diverged from the named SUMMARISER_SIZE_BYTES constant",
    );
}

#[test]
fn global_registry_summariser_entry_present_and_pinned() {
    let from_global = MODEL_REGISTRY
        .iter()
        .find(|e| e.kind == ModelKind::Summariser)
        .expect("MODEL_REGISTRY must carry a summariser entry");
    assert_eq!(from_global.name, SUMMARISER_NAME);
    assert!(!from_global.has_placeholder_checksum());
    assert_eq!(from_global.sha256, SUMMARISER_SHA256);
}

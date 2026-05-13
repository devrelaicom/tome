//! Integration tests for `tome models list` against the CLI binary.
//!
//! The handler classifies each registered model into one of
//! `Ok | Missing | Corrupt | ChecksumMismatched`. Without `--verify`, the
//! check is cheap (existence + size). With `--verify`, the primary artefact
//! is rehashed against the registry's pinned SHA-256. Sparse-file fixtures
//! (~no disk space) let us stage all states in CI.
//!
//! Spec: `contracts/models-commands.md` §"`tome models list`".

mod common;

use common::{ToolEnv, fabricate_all_installed_models, fabricate_installed_model, paths_for};
use serde_json::Value;
use tome::embedding::registry::MODEL_REGISTRY;

#[test]
fn list_with_no_models_installed_reports_missing_for_every_entry() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();

    let out = env
        .cmd()
        .args(["models", "list", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    let records: Vec<Value> =
        serde_json::from_slice(&out.stdout).expect("--json must emit a JSON array");
    assert_eq!(records.len(), MODEL_REGISTRY.len());
    for r in records {
        assert_eq!(r["state"], "missing");
    }
}

#[test]
fn list_with_all_models_installed_reports_ok_under_cheap_check() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_all_installed_models(&paths);

    let out = env
        .cmd()
        .args(["models", "list", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let records: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();
    for r in records {
        assert_eq!(
            r["state"], "ok",
            "fabricated models must read `ok` under the cheap probe, got {r:?}",
        );
    }
}

#[test]
fn list_with_verify_flips_tampered_artefact_to_checksum_mismatched() {
    // The fabricate helper writes all-zero sparse files. Their SHA-256
    // (the digest of (size_bytes) zero bytes) does NOT match the pinned
    // registry hash for either model. Under the cheap probe they read `ok`;
    // under `--verify` they must read `checksum_mismatched`.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    // Only stage the embedder — keeps the rehash bounded to ~66 MB of
    // streamed zero-bytes (a couple of hundred ms on modern hardware) and
    // leaves the reranker in `missing` for a clean control case.
    let embedder = MODEL_REGISTRY
        .iter()
        .find(|e| matches!(e.kind, tome::embedding::registry::ModelKind::Embedder))
        .expect("registry has an embedder");
    fabricate_installed_model(&paths, embedder);

    let out = env
        .cmd()
        .args(["models", "list", "--verify", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let records: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();
    let embedder_record = records
        .iter()
        .find(|r| r["name"] == embedder.name)
        .expect("embedder must appear in the list");
    assert_eq!(
        embedder_record["state"], "checksum_mismatched",
        "tampered artefact must surface as checksum_mismatched under --verify",
    );

    // The non-fabricated entry must still read `missing` — `--verify` is a
    // no-op on entries that fail the cheap probe.
    for r in &records {
        if r["name"] != embedder.name {
            assert_eq!(r["state"], "missing");
        }
    }
}

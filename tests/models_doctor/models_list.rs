//! Integration tests for `tome models list` against the CLI binary.
//!
//! The handler classifies each registered model into one of
//! `Ok | Missing | Corrupt | ChecksumMismatched`. Without `--verify`, the
//! check is cheap (existence + size). With `--verify`, the primary artefact
//! is rehashed against the registry's pinned SHA-256. Sparse-file fixtures
//! (~no disk space) let us stage all states in CI.
//!
//! Spec: `contracts/models-commands.md` §"`tome models list`".

use crate::common::{
    ToolEnv, fabricate_all_registry_models, fabricate_installed_models, paths_for,
};
use serde_json::Value;
use tome::embedding::registry::MODEL_REGISTRY;

#[test]
fn list_with_no_models_installed_reports_missing_for_every_entry() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

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
fn list_json_annotates_each_row_with_profiles_and_marks_the_active_set() {
    // Default profile is Medium: bge-base-en-v1.5 + bge-reranker-large are the
    // active set; the summariser is referenced by every profile but is never
    // `active` (profile-independent).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

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

    let records: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();
    let by_name = |name: &str| {
        records
            .iter()
            .find(|r| r["name"] == name)
            .unwrap_or_else(|| panic!("`{name}` must appear in the list"))
            .clone()
    };

    // Every entry carries a non-empty `profiles` array + a boolean `active`.
    for r in &records {
        assert!(
            r["profiles"].as_array().is_some_and(|a| !a.is_empty()),
            "every row must carry a non-empty `profiles` array, got {r:?}",
        );
        assert!(r["active"].is_boolean(), "every row carries a boolean `active`: {r:?}");
    }

    // Per-entry profile mapping.
    assert_eq!(by_name("bge-small-en-v1.5")["profiles"], serde_json::json!(["small"]));
    assert_eq!(by_name("bge-base-en-v1.5")["profiles"], serde_json::json!(["medium"]));
    assert_eq!(by_name("bge-large-en-v1.5")["profiles"], serde_json::json!(["large"]));
    assert_eq!(
        by_name("qwen2.5-0.5b-instruct")["profiles"],
        serde_json::json!(["small", "medium", "large"]),
    );

    // The Medium set is active; no other entry is.
    assert_eq!(by_name("bge-base-en-v1.5")["active"], true);
    assert_eq!(by_name("bge-reranker-large")["active"], true);
    assert_eq!(by_name("bge-small-en-v1.5")["active"], false);
    assert_eq!(by_name("bge-large-en-v1.5")["active"], false);
    assert_eq!(by_name("qwen2.5-0.5b-instruct")["active"], false);
}

#[test]
fn list_active_marker_follows_the_active_profile() {
    // After switching to `large`, the large pair becomes the active set.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let set = env
        .cmd()
        .args(["models", "profile", "large"])
        .output()
        .unwrap();
    assert!(
        set.status.success(),
        "profile set failed: {}",
        String::from_utf8_lossy(&set.stderr),
    );

    let out = env
        .cmd()
        .args(["models", "list", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());

    let records: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();
    let active: Vec<&str> = records
        .iter()
        .filter(|r| r["active"] == true)
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        active,
        vec!["bge-large-en-v1.5", "bge-reranker-v2-m3"],
        "after `profile large`, the active set is the large pair",
    );
}

#[test]
fn list_with_all_models_installed_reports_ok_under_cheap_check() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

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
    std::fs::create_dir_all(&paths.root).unwrap();
    // Only stage the embedder — keeps the rehash bounded to ~66 MB of
    // streamed zero-bytes (a couple of hundred ms on modern hardware) and
    // leaves the reranker in `missing` for a clean control case.
    let embedder = MODEL_REGISTRY
        .iter()
        .find(|e| matches!(e.kind, tome::embedding::registry::ModelKind::Embedder))
        .expect("registry has an embedder");
    fabricate_installed_models(&paths, &[embedder]);

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

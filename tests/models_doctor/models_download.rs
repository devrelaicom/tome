//! Integration tests for `tome models download` against the CLI binary.
//!
//! The atomic-rename / streaming-hash pipeline that `download_model` wraps is
//! exhaustively covered at the library level in `tests/model_download.rs`
//! (success, checksum mismatch, HTTP 404, placeholder refusal). This file
//! covers the CLI handler's behaviour ON TOP of that library: idempotent
//! skip when the on-disk state is already `Ok`, and JSON envelope shape.
//!
//! The CLI uses the compile-time `MODEL_REGISTRY` with real upstream URLs.
//! We cannot drive a network download from CI, so the `--force` path is
//! exercised at the library level instead.
//!
//! Spec: `contracts/models-commands.md` §"`tome models download`".

use crate::common::{ToolEnv, fabricate_all_registry_models, paths_for};
use serde_json::Value;

#[test]
fn download_with_all_models_already_installed_emits_skipped_records() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let out = env
        .cmd()
        .args(["models", "download", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "exit: {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let envelope: Value = serde_json::from_slice(&out.stdout)
        .expect("--json envelope must parse as a single JSON object");
    let models = envelope["models"]
        .as_array()
        .expect("envelope.models must be an array");
    assert_eq!(
        models.len(),
        3,
        "three registered models (embedder + reranker + summariser)"
    );
    for m in models {
        assert_eq!(
            m["action"], "skipped",
            "every pre-installed model must report `skipped`, got {m:?}",
        );
        assert_eq!(
            m["duration_ms"], 0,
            "skipped record must record zero duration"
        );
    }
}

#[test]
fn download_with_no_models_installed_reports_missing_via_list_first() {
    // Sanity that the test harness gives us a clean state — no installed
    // models. We don't invoke `download` here (it would hit the network); we
    // confirm via `models list --json` that the cheap-state probe correctly
    // sees a fresh layout, which is the same probe `download` uses for its
    // skip decision.
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
        String::from_utf8_lossy(&out.stderr)
    );

    let records: Vec<Value> =
        serde_json::from_slice(&out.stdout).expect("--json records array must parse");
    for r in &records {
        assert_eq!(
            r["state"], "missing",
            "fresh layout must report missing, got {r:?}"
        );
    }
}

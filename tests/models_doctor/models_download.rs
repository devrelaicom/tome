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

/// Pull `envelope.models` from a `--json` download envelope.
fn models_array(stdout: &[u8]) -> Vec<Value> {
    let envelope: Value = serde_json::from_slice(stdout)
        .expect("--json envelope must parse as a single JSON object");
    envelope["models"]
        .as_array()
        .expect("envelope.models must be an array")
        .clone()
}

#[test]
fn download_with_all_flag_targets_every_registry_entry() {
    // `--all` ignores the active profile and fetches the full registry.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let out = env
        .cmd()
        .args(["models", "download", "--all", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "exit: {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let models = models_array(&out.stdout);
    assert_eq!(
        models.len(),
        7,
        "`--all` must target all seven registered models (3 embedders + 3 rerankers + 1 summariser)"
    );
    for m in &models {
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
fn download_default_targets_only_the_active_profile_set() {
    // Default (no `--all`) targets exactly the active profile's
    // {embedder, reranker, summariser} — three entries, not the whole
    // registry. Fabricating every model means each is `skipped`, so no
    // network is touched; the count proves the targeting.
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

    let models = models_array(&out.stdout);
    assert_eq!(
        models.len(),
        3,
        "default download targets only the active profile's embedder + reranker + summariser, got {models:?}"
    );

    // The default profile is Medium, so the embedder/reranker are the medium
    // pair; the summariser is shared across profiles.
    let names: Vec<&str> = models.iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"bge-base-en-v1.5"), "medium embedder targeted: {names:?}");
    assert!(names.contains(&"bge-reranker-large"), "medium reranker targeted: {names:?}");
    assert!(names.contains(&"qwen2.5-0.5b-instruct"), "summariser targeted: {names:?}");
}

#[test]
fn download_default_follows_the_active_profile_after_a_switch() {
    // Switching the profile to `large` (which creates the index DB) must
    // re-scope the default download to the large pair.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

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
        .args(["models", "download", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    let models = models_array(&out.stdout);
    let names: Vec<&str> = models.iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert_eq!(models.len(), 3, "still three targets after the switch: {names:?}");
    assert!(names.contains(&"bge-large-en-v1.5"), "large embedder targeted: {names:?}");
    assert!(names.contains(&"bge-reranker-v2-m3"), "large reranker targeted: {names:?}");
    assert!(names.contains(&"qwen2.5-0.5b-instruct"), "summariser still targeted: {names:?}");
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

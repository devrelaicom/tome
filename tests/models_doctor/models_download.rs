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
    let envelope: Value =
        serde_json::from_slice(stdout).expect("--json envelope must parse as a single JSON object");
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
    assert!(
        names.contains(&"bge-base-en-v1.5"),
        "medium embedder targeted: {names:?}"
    );
    assert!(
        names.contains(&"bge-reranker-large"),
        "medium reranker targeted: {names:?}"
    );
    assert!(
        names.contains(&"qwen2.5-0.5b-instruct"),
        "summariser targeted: {names:?}"
    );
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
    assert_eq!(
        models.len(),
        3,
        "still three targets after the switch: {names:?}"
    );
    assert!(
        names.contains(&"bge-large-en-v1.5"),
        "large embedder targeted: {names:?}"
    );
    assert!(
        names.contains(&"bge-reranker-v2-m3"),
        "large reranker targeted: {names:?}"
    );
    assert!(
        names.contains(&"qwen2.5-0.5b-instruct"),
        "summariser still targeted: {names:?}"
    );
}

/// Read the stored active profile via the `models profile --json` show record.
/// Returns `None` when no DB exists yet (fresh install → the default is
/// reported but nothing is persisted).
fn stored_profile(env: &ToolEnv) -> String {
    let out = env
        .cmd()
        .args(["--json", "models", "profile"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "profile show failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let rec: Value = serde_json::from_slice(&out.stdout).expect("profile --json object");
    rec["profile"].as_str().expect("profile string").to_owned()
}

#[test]
fn download_profile_flag_targets_that_tier() {
    // `--profile large` targets the large pair + summariser (three entries),
    // regardless of the active profile (default medium). Fabricating every
    // model means each is `skipped`, so no network is touched; the count +
    // names prove the targeting.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let out = env
        .cmd()
        .args(["models", "download", "--profile", "large", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "exit: {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let models = models_array(&out.stdout);
    let names: Vec<&str> = models.iter().map(|m| m["name"].as_str().unwrap()).collect();
    assert_eq!(models.len(), 3, "large tier is three targets: {names:?}");
    assert!(
        names.contains(&"bge-large-en-v1.5"),
        "large embedder targeted: {names:?}"
    );
    assert!(
        names.contains(&"bge-reranker-v2-m3"),
        "large reranker targeted: {names:?}"
    );
    assert!(
        names.contains(&"qwen2.5-0.5b-instruct"),
        "summariser targeted: {names:?}"
    );
}

#[test]
fn download_profile_flag_does_not_flip_the_stored_active_profile() {
    // The headline invariant of `--profile`: downloading a non-active tier
    // must NOT mutate the stored active profile. We first switch to `small`
    // (writing meta), then `download --profile large`, then assert the stored
    // profile is still `small` (before == after).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    // Establish a definite non-default stored profile.
    let set = env
        .cmd()
        .args(["models", "profile", "small"])
        .output()
        .unwrap();
    assert!(
        set.status.success(),
        "profile set failed: {}",
        String::from_utf8_lossy(&set.stderr),
    );
    let before = stored_profile(&env);
    assert_eq!(before, "small", "precondition: active profile is small");

    // Download the LARGE tier's models via --profile.
    let out = env
        .cmd()
        .args(["models", "download", "--profile", "large", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "download --profile large failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // The stored active profile is UNCHANGED.
    let after = stored_profile(&env);
    assert_eq!(
        after, before,
        "download --profile must NOT flip the stored active profile"
    );

    // And confirmed at the on-disk meta layer.
    let conn = tome::index::open_read_only(&paths.index_db).expect("open index");
    let profile = tome::index::meta::active_profile(&conn).expect("active profile");
    assert_eq!(
        profile,
        tome::embedding::profile::Profile::Small,
        "meta.model_profile must remain small"
    );
}

#[test]
fn download_profile_conflicts_with_all() {
    // `--profile` and `--all` are mutually exclusive (--all already spans every
    // tier). clap rejects the combination with a usage error (exit 2).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let out = env
        .cmd()
        .args(["models", "download", "--profile", "small", "--all"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "--profile with --all must be a usage error; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn download_profile_rejects_invalid_tier_via_value_enum() {
    // The `--profile` value is a clap `ValueEnum`; an unknown tier is rejected
    // at parse time (exit 2) with the valid values listed.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let out = env
        .cmd()
        .args(["models", "download", "--profile", "extra-large"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2), "invalid tier must usage-error");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("small") && stderr.contains("medium") && stderr.contains("large"),
        "clap must list the valid tiers: {stderr}",
    );
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

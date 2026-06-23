//! Task 10: `[doctor] verify_by_default` in `~/.tome/config.toml`.
//!
//! When `verify_by_default = true`, `tome doctor --json` (no `--verify` flag)
//! rehashes model files. Sparse / zeroed fabricated files differ from the
//! registry SHA-256, so the embedder state becomes `checksum_mismatched`.
//!
//! Active models are resolved from Profile::DEFAULT (Medium = bge-base-en-v1.5
//! + bge-reranker-large) when no index DB is present. We fabricate only those
//! two so the doctor sees the files and can (or can't) hash them.

use crate::common::{ToolEnv, fabricate_installed_models, paths_for};
use serde_json::Value;
use tome::embedding::profile::{Profile, embedder_for, reranker_for};

/// `verify_by_default = true` in config causes checksum rehash without --verify.
#[test]
fn verify_by_default_in_config_triggers_checksum_check() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // Fabricate the default-profile (Medium) embedder and reranker.
    // Sparse files have wrong SHA-256 → checksum_mismatched when verified.
    let emb = embedder_for(Profile::DEFAULT);
    let rnk = reranker_for(Profile::DEFAULT);
    fabricate_installed_models(&paths, &[emb, rnk]);

    // Write [doctor] verify_by_default = true — no --verify flag passed.
    std::fs::write(
        &paths.global_config_file,
        "[doctor]\nverify_by_default = true\n",
    )
    .unwrap();

    // Run `tome --json doctor` (no --verify flag).
    let out = env.cmd().args(["--json", "doctor"]).output().unwrap();
    // May or may not be a success exit code — we only inspect JSON output.
    let v: Value = serde_json::from_slice(&out.stdout).expect("JSON output");

    // The embedder state should be checksum_mismatched because the sparse
    // file's SHA-256 differs from the registry value.
    let emb_state = v["embedder"]["state"].as_str().unwrap_or("");
    assert_eq!(
        emb_state, "checksum_mismatched",
        "verify_by_default should trigger checksum rehash; embedder state was {emb_state:?}"
    );
}

/// Without `verify_by_default`, a sparse file shows `ok` (no rehash).
#[test]
fn without_verify_by_default_sparse_file_shows_ok() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let emb = embedder_for(Profile::DEFAULT);
    let rnk = reranker_for(Profile::DEFAULT);
    fabricate_installed_models(&paths, &[emb, rnk]);

    // No config with verify_by_default.

    let out = env.cmd().args(["--json", "doctor"]).output().unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).expect("JSON output");

    let emb_state = v["embedder"]["state"].as_str().unwrap_or("");
    assert_eq!(
        emb_state, "ok",
        "without verify, sparse file should appear ok; got {emb_state:?}"
    );
}

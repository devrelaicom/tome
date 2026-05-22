//! Phase 8 / US6 slice 2 — `tome status` health report.
//!
//! Library-API tests for `commands::status::assemble_report`. We bypass
//! the CLI binary's `run()` because that function calls `std::process::exit`
//! in degraded / unhealthy cases — which would tear down the test runner.
//! `assemble_report` is the pure function that produces the report; the
//! exit semantics are tested separately via the CLI binary.

mod common;

use common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, lifecycle_paths, paths_for,
};
use tempfile::TempDir;
use tome::commands::plugin::registry_seeds;
use tome::commands::status::{OverallHealth, assemble_report};
use tome::embedding::registry::MODEL_REGISTRY;
use tome::embedding::stub::StubEmbedder;
use tome::index::meta::{DriftStatus, MetaKey, write as write_meta};
use tome::index::{self, OpenOptions};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

/// Enable plugin-alpha with `registry_seeds()` (the real BGE seeds) so that
/// the `meta` table records BGE values, matching what `assemble_report` reads
/// from the configured-side. This decouples the StubEmbedder used for the
/// embed call from the identity stored in meta — exactly the contract: the
/// seed identifies the model, the embedder produces the vectors.
fn enable_alpha(
    paths: &tome::paths::Paths,
    config: &tome::config::Config,
    embedder: &StubEmbedder,
) {
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let (embedder_seed, reranker_seed) = registry_seeds();
    let deps = LifecycleDeps {
        paths,
        scope: &tome::workspace::Scope::Global,
        config,
        embedder,
        embedder_seed,
        reranker_seed,
        allow_model_download: false,
    };
    lifecycle::enable(&id, &deps).expect("enable alpha");
}

// ---- Healthy --------------------------------------------------------------

#[test]
fn status_healthy_with_models_and_index() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    common::fabricate_all_installed_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha(&paths, &config, &embedder);

    let report = assemble_report(&paths, &tome::workspace::Scope::Global, false).expect("assemble");
    assert_eq!(report.overall, OverallHealth::Ok);
    assert_eq!(report.embedder.state, "ok");
    assert_eq!(report.reranker.state, "ok");
    assert!(report.index.present);
    assert!(report.index.integrity_ok);
    assert_eq!(report.index.plugins_enabled, 1);
    assert_eq!(report.index.skills_indexed, 4);
    assert_eq!(report.drift, DriftStatus::None);
}

#[test]
fn status_healthy_with_no_index_yet() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    common::fabricate_all_installed_models(&paths);

    let report = assemble_report(&paths, &tome::workspace::Scope::Global, false).expect("assemble");
    // No index bootstrapped — but models present, no drift to detect.
    assert!(!report.index.present);
    assert_eq!(report.drift, DriftStatus::None);
    assert_eq!(report.overall, OverallHealth::Ok);
}

// ---- Unhealthy: embedder missing -----------------------------------------

#[test]
fn status_unhealthy_when_embedder_missing() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // Deliberately do NOT fabricate_models — embedder + reranker both
    // report Missing. Embedder Missing trumps reranker Missing in
    // classify(): the overall verdict is Unhealthy.

    let report = assemble_report(&paths, &tome::workspace::Scope::Global, false).expect("assemble");
    assert_eq!(report.embedder.state, "missing");
    assert_eq!(report.overall, OverallHealth::Unhealthy);
}

// ---- Degraded: reranker only -----------------------------------------

#[test]
fn status_degraded_when_only_reranker_missing() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    common::fabricate_all_installed_models(&paths);

    // Now remove just the reranker dir.
    let reranker_name = MODEL_REGISTRY
        .iter()
        .find(|e| matches!(e.kind, tome::embedding::ModelKind::Reranker))
        .unwrap()
        .name;
    let reranker_dir = paths.models_dir.join(reranker_name);
    std::fs::remove_dir_all(&reranker_dir).unwrap();

    let report = assemble_report(&paths, &tome::workspace::Scope::Global, false).expect("assemble");
    assert_eq!(report.embedder.state, "ok");
    assert_eq!(report.reranker.state, "missing");
    assert_eq!(report.overall, OverallHealth::Degraded);
}

// ---- Drift: reranker drift -----------------------------------------------

#[test]
fn status_degraded_on_reranker_drift_in_meta() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    common::fabricate_all_installed_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    let embedder = StubEmbedder::new();
    enable_alpha(&paths, &config, &embedder);

    // Mutate meta to simulate a reranker upgrade: the stored value records
    // an older reranker name while the registry's reranker (= currently
    // configured) is unchanged.
    let (embedder_seed, reranker_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
        },
    )
    .unwrap();
    write_meta(&conn, MetaKey::RerankerName, "bge-reranker-OLD").unwrap();

    let report = assemble_report(&paths, &tome::workspace::Scope::Global, false).expect("assemble");
    assert!(
        matches!(report.drift, DriftStatus::RerankerDrift { .. }),
        "expected RerankerDrift, got {:?}",
        report.drift,
    );
    assert_eq!(report.overall, OverallHealth::Degraded);
}

// ---- Drift: embedder drift -> Unhealthy ----------------------------------

#[test]
fn status_unhealthy_on_embedder_drift() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    common::fabricate_all_installed_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    let embedder = StubEmbedder::new();
    enable_alpha(&paths, &config, &embedder);

    let (embedder_seed, reranker_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
        },
    )
    .unwrap();
    write_meta(&conn, MetaKey::EmbedderName, "bge-OLD").unwrap();

    let report = assemble_report(&paths, &tome::workspace::Scope::Global, false).expect("assemble");
    assert!(
        matches!(report.drift, DriftStatus::EmbedderNameDrift { .. }),
        "expected EmbedderNameDrift, got {:?}",
        report.drift,
    );
    assert_eq!(report.overall, OverallHealth::Unhealthy);
}

// ---- Verify flag rehashes models -----------------------------------------

#[test]
fn status_verify_flag_detects_checksum_mismatch() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    // Use the real-sized sparse-file fabricator so the on-disk SHA-256 is
    // an all-zero hash, which by construction does NOT match the registry's
    // pinned SHA.
    common::fabricate_all_installed_models(&paths);

    let report = assemble_report(&paths, &tome::workspace::Scope::Global, true).expect("assemble");
    assert_eq!(
        report.embedder.state, "checksum_mismatched",
        "expected checksum mismatch on the embedder",
    );
    assert_eq!(report.overall, OverallHealth::Unhealthy);
}

// ---- CLI binary: exit code semantics -------------------------------------

#[test]
fn status_cli_exits_0_when_healthy() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    common::fabricate_all_installed_models(&paths);

    let out = env.cmd().args(["status"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn status_cli_exits_1_when_embedder_missing() {
    let env = ToolEnv::new();
    // No model fabrication — both embedder and reranker report Missing,
    // which classifies as Unhealthy.
    let out = env.cmd().args(["status"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn status_cli_json_emits_structured_record() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    common::fabricate_all_installed_models(&paths);

    let out = env.cmd().args(["--json", "status"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("parse JSON");
    assert!(v.get("tome").is_some());
    assert_eq!(v["embedder"]["state"], "ok");
    assert_eq!(v["reranker"]["state"], "ok");
    assert_eq!(v["overall"], "ok");
}

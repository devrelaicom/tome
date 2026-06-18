//! Phase 8 / US6 slice 2 — `tome status` health report.
//!
//! Library-API tests for `commands::status::assemble_report`. We bypass
//! the CLI binary's `run()` because that function calls `std::process::exit`
//! in degraded / unhealthy cases — which would tear down the test runner.
//! `assemble_report` is the pure function that produces the report; the
//! exit semantics are tested separately via the CLI binary.

use crate::common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, enrol_catalog_symlinked,
    lifecycle_paths, paths_for,
};
use tempfile::TempDir;
use tome::commands::plugin::registry_seeds;
use tome::commands::status::{OverallHealth, assemble_report};
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
    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let deps = LifecycleDeps {
        paths,
        scope: &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        config,
        embedder,
        embedder_seed,
        reranker_seed,
        summariser_seed,
        allow_model_download: false,
    };
    lifecycle::enable(&id, &deps).expect("enable alpha");
}

// ---- Healthy --------------------------------------------------------------

#[test]
fn status_healthy_with_models_and_index() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog + symlink the cache dir so the DB-backed
    // `resolve_plugin_dir` used by `enable_alpha` resolves the fixture.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha(&paths, &config, &embedder);

    let report = assemble_report(
        &paths,
        &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        false,
    )
    .expect("assemble");
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
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    let report = assemble_report(
        &paths,
        &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        false,
    )
    .expect("assemble");
    // No index bootstrapped — but models present, no drift to detect.
    assert!(!report.index.present);
    assert_eq!(report.drift, DriftStatus::None);
    assert_eq!(report.overall, OverallHealth::Ok);
}

// ---- Unhealthy: embedder missing -----------------------------------------

#[test]
fn status_unhealthy_when_embedder_missing() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // Deliberately do NOT fabricate_models — embedder + reranker both
    // report Missing. Embedder Missing trumps reranker Missing in
    // classify(): the overall verdict is Unhealthy.

    let report = assemble_report(
        &paths,
        &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        false,
    )
    .expect("assemble");
    assert_eq!(report.embedder.state, "missing");
    assert_eq!(report.overall, OverallHealth::Unhealthy);
}

// ---- Degraded: reranker only -----------------------------------------

#[test]
fn status_degraded_when_only_reranker_missing() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    // Now remove just the reranker dir — use the DEFAULT profile's reranker
    // so the removal matches what `assemble_report` checks.
    use tome::embedding::profile::{Profile, reranker_for};
    let reranker_name = reranker_for(Profile::DEFAULT).name;
    let reranker_dir = paths.models_dir.join(reranker_name);
    std::fs::remove_dir_all(&reranker_dir).unwrap();

    let report = assemble_report(
        &paths,
        &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        false,
    )
    .expect("assemble");
    assert_eq!(report.embedder.state, "ok");
    assert_eq!(report.reranker.state, "missing");
    assert_eq!(report.overall, OverallHealth::Degraded);
}

// ---- Drift: reranker drift -----------------------------------------------

#[test]
fn status_degraded_on_reranker_drift_in_meta() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog + symlink the cache dir so the DB-backed
    // `resolve_plugin_dir` used by `enable_alpha` resolves the fixture.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);
    let embedder = StubEmbedder::new();
    enable_alpha(&paths, &config, &embedder);

    // Mutate meta to simulate a reranker upgrade: the stored value records
    // an older reranker name while the registry's reranker (= currently
    // configured) is unchanged.
    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
            summariser: summariser_seed,
        },
    )
    .unwrap();
    write_meta(&conn, MetaKey::RerankerName, "bge-reranker-OLD").unwrap();

    let report = assemble_report(
        &paths,
        &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        false,
    )
    .expect("assemble");
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
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog + symlink the cache dir so the DB-backed
    // `resolve_plugin_dir` used by `enable_alpha` resolves the fixture.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);
    let embedder = StubEmbedder::new();
    enable_alpha(&paths, &config, &embedder);

    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
            summariser: summariser_seed,
        },
    )
    .unwrap();
    write_meta(&conn, MetaKey::EmbedderName, "bge-OLD").unwrap();

    let report = assemble_report(
        &paths,
        &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        false,
    )
    .expect("assemble");
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
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    // Use the real-sized sparse-file fabricator so the on-disk SHA-256 is
    // an all-zero hash, which by construction does NOT match the registry's
    // pinned SHA.
    crate::common::fabricate_all_registry_models(&paths);

    let report = assemble_report(
        &paths,
        &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        true,
    )
    .expect("assemble");
    assert_eq!(
        report.embedder.state, "checksum_mismatched",
        "expected checksum mismatch on the embedder",
    );
    assert_eq!(report.overall, OverallHealth::Unhealthy);
}

// ---- New fields: summariser, scope, models_on_disk_bytes -----------------

#[test]
fn status_reports_summariser_scope_and_models_on_disk() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    let scope = tome::workspace::Scope(tome::workspace::WorkspaceName::global());
    let report = assemble_report(&paths, &scope, false).unwrap();

    // Third model is reported.
    assert_eq!(report.summariser.state, "ok");
    // Scope fields reflect the global default.
    assert_eq!(report.current_workspace, "global");
    assert_eq!(report.current_scope, "global");
    // Fabricated models occupy non-zero disk.
    assert!(report.models_on_disk_bytes > 0);
}

// ---- New fields: workspace-scoped entry/catalog/reindex counts -----------

#[test]
fn status_reports_workspace_scoped_counts() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // Enrol the catalog + symlink the cache dir so `enable_alpha` resolves
    // the fixture via the DB-backed `resolve_plugin_dir`.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha(&paths, &config, &embedder);

    let scope = tome::workspace::Scope(tome::workspace::WorkspaceName::global());
    let report = assemble_report(&paths, &scope, false).unwrap();

    // plugin-alpha ships at least one skill → entries.skills > 0.
    assert!(report.entries.skills > 0, "expected indexed skills, got 0");
    // `global` is excluded from the user-workspace count.
    assert_eq!(report.workspaces_total, 0);
    // alpha came from an enrolled catalog.
    assert!(
        report.catalogs_enrolled >= 1,
        "expected at least one enrolled catalog"
    );
    // something was indexed → a timestamp exists.
    assert!(
        report.reindexed_at.is_some(),
        "expected a reindexed_at timestamp"
    );
}

// ---- CLI binary: exit code semantics -------------------------------------

#[test]
fn status_cli_exits_0_when_healthy() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    let out = env.cmd().args(["status"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn status_cli_exits_1_when_embedder_missing() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    // No model fabrication — both embedder and reranker report Missing,
    // which classifies as Unhealthy.
    let out = env.cmd().args(["status"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn status_cli_json_emits_structured_record() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

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
    // Enriched fields pinned in Task 4 of the bookshelf redesign.
    // fabricate_all_registry_models fabricates all three models including the
    // summariser, so state == "ok" is strict.
    assert_eq!(v["summariser"]["state"], "ok");
    assert!(v["workspaces_total"].is_number());
    assert!(v["current_workspace"].is_string());
    assert!(v["current_scope"].is_string());
    assert!(v["entries"]["skills"].is_number());
    assert!(v["entries"]["commands"].is_number());
    assert!(v["entries"]["agents"].is_number());
    assert!(v["catalogs_enrolled"].is_number());
    assert!(v.get("reindexed_at").is_some()); // null or number
    assert!(v["models_on_disk_bytes"].is_number());
}

#[test]
fn status_human_plain_is_grouped_and_labeled() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    let out = env.cmd().args(["status"]).output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);

    // Title + group headers + a sampling of labels (piped => no colour/art).
    assert!(s.contains("Tome v"));
    assert!(s.contains("Global"));
    assert!(s.contains("Workspace"));
    assert!(s.contains("Models:"));
    assert!(s.contains("Workspaces:"));
    assert!(s.contains("Entries:"));
    assert!(s.contains("Catalogs:"));
    assert!(s.contains("Reindexed:"));
    assert!(s.contains("Overall:"));
    // No box-drawing art when piped.
    assert!(!s.contains('┌'));
}

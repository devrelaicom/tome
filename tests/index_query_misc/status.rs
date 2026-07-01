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
            profile: None,
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
            profile: None,
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
fn status_cli_exits_unhealthy_code_when_embedder_missing() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    // No model fabrication — both embedder and reranker report Missing,
    // which classifies as Unhealthy.
    let out = env.cmd().args(["status"]).output().unwrap();
    // Issue #282: Unhealthy keeps its historical exit code (1).
    assert_eq!(out.status.code(), Some(tome::error::EXIT_HEALTH_UNHEALTHY));
}

/// Issue #282: a Degraded verdict (reranker missing — the embedder + index
/// still serve queries) exits with the DISTINCT Degraded code, not the
/// Unhealthy `1`. Both are non-zero so "fail on any non-zero" gates are
/// unaffected; the distinct code lets a "fail on unhealthy only" gate branch.
#[test]
fn status_cli_exits_degraded_code_when_only_reranker_missing() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    // Remove just the reranker dir (DEFAULT profile) → embedder ok, reranker
    // missing → classify() returns Degraded.
    use tome::embedding::profile::{Profile, reranker_for};
    let reranker_name = reranker_for(Profile::DEFAULT).name;
    std::fs::remove_dir_all(paths.models_dir.join(reranker_name)).unwrap();

    let out = env.cmd().args(["status"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(tome::error::EXIT_HEALTH_DEGRADED),
        "degraded status must exit {} (distinct from unhealthy {}); stderr: {}",
        tome::error::EXIT_HEALTH_DEGRADED,
        tome::error::EXIT_HEALTH_UNHEALTHY,
        String::from_utf8_lossy(&out.stderr),
    );
    // The three codes are distinct (defence-in-depth against a future
    // accidental collision that would silently re-merge the verdicts).
    assert_ne!(
        tome::error::EXIT_HEALTH_DEGRADED,
        tome::error::EXIT_HEALTH_UNHEALTHY
    );
    assert_ne!(tome::error::EXIT_HEALTH_DEGRADED, 0);

    // `--json` still exposes the three-state via `overall` — the documented
    // gating field.
    let out_json = env.cmd().args(["--json", "status"]).output().unwrap();
    assert_eq!(
        out_json.status.code(),
        Some(tome::error::EXIT_HEALTH_DEGRADED)
    );
    let v: serde_json::Value = serde_json::from_slice(&out_json.stdout).unwrap();
    assert_eq!(v["overall"], "degraded");
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

/// M3 (US5 closeout): exercise `status::fill_harness_mcp`'s POPULATED path
/// through the real CLI. Bind a project marker to workspace "demo" declaring an
/// effective harness list whose members have known MCP states — crush=ok
/// (correct Tome entry), devin=drift (stale `--workspace` arg), jetbrains-ai=
/// manual (no writable MCP file) — then run `--json status` UNDER that project
/// scope and assert the emitted `harness_mcp` array's states + the human-panel
/// glyphs. The status analogue of `doctor_mcp_states_p11.rs`.
#[test]
fn status_cli_json_populates_harness_mcp_under_project_scope() {
    use tome::harness::lookup;
    use tome::harness::mcp_config::{self, TomeEntry};

    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);
    // The marker binds to "demo"; the workspace row must exist in the DB.
    crate::common::seed_workspace(&paths, "demo");

    // Project marker: workspace + an effective harness list.
    let project = env.home_path().join("project");
    let marker_dir = project.join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(
        marker_dir.join("config.toml"),
        "workspace = \"demo\"\nharnesses = [\"crush\", \"devin\", \"jetbrains-ai\"]\n",
    )
    .unwrap();

    // Seed each harness's MCP entry on disk via its PRODUCTION dialect.
    let home = env.home_path();
    let seed = |harness: &str, ws: &str| {
        let module = lookup(harness).expect("harness");
        let path = module.mcp_config_path(&project, home);
        let entry = TomeEntry::new(
            "tome".to_string(),
            vec![
                "mcp".to_string(),
                "--workspace".to_string(),
                ws.to_string(),
                "--harness".to_string(),
                harness.to_string(),
            ],
        );
        mcp_config::write_entry(&path, &module.mcp_dialect(), &entry).expect("write entry");
    };
    // crush: correct workspace → ok.
    seed("crush", "demo");
    // devin: stale workspace arg → drift.
    seed("devin", "stale");
    // jetbrains-ai: manual-only — deliberately write NO MCP file.

    // Run `--json status` from inside the project dir so the marker resolves.
    let out = env
        .cmd()
        .current_dir(&project)
        .args(["--json", "status"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
        panic!(
            "parse JSON; stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        )
    });

    let mcp = v["harness_mcp"]
        .as_array()
        .expect("harness_mcp array present");
    let state_of = |name: &str| -> &str {
        mcp.iter()
            .find(|h| h["harness"] == name)
            .unwrap_or_else(|| panic!("{name} in harness_mcp; got {mcp:?}"))["state"]
            .as_str()
            .unwrap()
    };
    assert_eq!(state_of("crush"), "ok", "crush correct entry → ok");
    assert_eq!(
        state_of("devin"),
        "drift",
        "devin stale --workspace → drift"
    );
    assert_eq!(
        state_of("jetbrains-ai"),
        "manual",
        "jetbrains-ai has no writable MCP file → manual",
    );

    // Human panel renders the per-harness MCP glyphs (piped ⇒ plain forms).
    let human = env
        .cmd()
        .current_dir(&project)
        .args(["status"])
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&human.stdout);
    assert!(s.contains("MCP:"), "panel shows the MCP row; got:\n{s}");
    assert!(s.contains("crush [ok]"), "crush ok glyph; got:\n{s}");
    assert!(s.contains("devin [drift]"), "devin drift glyph; got:\n{s}",);
    assert!(
        s.contains("jetbrains-ai [manual]"),
        "jetbrains-ai manual glyph; got:\n{s}",
    );
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

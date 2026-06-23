//! Phase 6 / US4 — `tome doctor` library-API + CLI integration tests.
//!
//! Library-API tests target `doctor::assemble_report` + `doctor::fixes::apply`
//! directly so we don't have to spawn the binary for every scenario.
//! CLI-binary smoke tests cover the emit path + exit-code propagation.
//!
//! The catalog re-clone repair gets a real `Fixture` because
//! `Git::clone_shallow` shells out to real `git` — the bridge is
//! straightforward. The model re-download repair is intentionally NOT
//! exercised end-to-end (would download real BGE models from the
//! internet); library-level coverage stays at the cheap_state /
//! check_model level.

use std::path::Path;

use crate::common::{Fixture, ToolEnv, fabricate_all_registry_models, paths_for};
use tempfile::TempDir;
use tome::doctor::{self, CatalogCacheState, DoctorClassification};
use tome::workspace::{ResolvedScope, ScopeSource};

fn global_scope() -> ResolvedScope {
    ResolvedScope {
        scope: tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    }
}

fn empty_home() -> TempDir {
    TempDir::new().unwrap()
}

// ---- Per-subsystem assembly --------------------------------------------

#[test]
fn assemble_with_no_models_reports_unhealthy() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert_eq!(report.embedder.state, "missing");
    assert_eq!(report.reranker.state, "missing");
    assert_eq!(report.overall, DoctorClassification::Unhealthy);
    // Suggested fixes for both models, both auto-fixable.
    let model_fixes: Vec<_> = report
        .suggested_fixes
        .iter()
        .filter(|f| f.subsystem == "embedder" || f.subsystem == "reranker")
        .collect();
    assert_eq!(model_fixes.len(), 2);
    assert!(model_fixes.iter().all(|f| f.auto_fixable));
}

#[test]
fn assemble_with_models_and_no_catalogs_reports_ok() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert_eq!(report.embedder.state, "ok");
    assert_eq!(report.reranker.state, "ok");
    assert_eq!(report.overall, DoctorClassification::Ok);
    assert!(report.suggested_fixes.is_empty());
}

#[test]
fn assemble_with_broken_catalog_cache_reports_degraded() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    // Register a catalog, then remove its `.git/` to simulate corruption.
    let fix = Fixture::build_sample();
    let out = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Find the catalog cache and corrupt it.
    let cache_dir = cache_dir_for(&env, &fix.url);
    assert!(cache_dir.join(".git").exists());
    std::fs::remove_dir_all(cache_dir.join(".git")).unwrap();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert_eq!(report.catalogs.len(), 1);
    assert_eq!(report.catalogs[0].state, CatalogCacheState::NotARepo);
    assert_eq!(report.overall, DoctorClassification::Degraded);
    let cat_fix = report
        .suggested_fixes
        .iter()
        .find(|f| matches!(f.subsystem, tome::doctor::Subsystem::Catalog(_)))
        .expect("catalog suggested fix");
    assert!(cat_fix.auto_fixable);
    assert!(cat_fix.command.starts_with("tome catalog update"));
}

#[test]
fn assemble_with_missing_catalog_cache_reports_degraded() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let fix = Fixture::build_sample();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    let cache_dir = cache_dir_for(&env, &fix.url);
    std::fs::remove_dir_all(&cache_dir).unwrap();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert_eq!(report.catalogs[0].state, CatalogCacheState::Missing);
    assert_eq!(report.overall, DoctorClassification::Degraded);
}

#[test]
fn assemble_with_manifest_invalid_is_not_auto_fixable() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let fix = Fixture::build_sample();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    let cache_dir = cache_dir_for(&env, &fix.url);
    // Corrupt the manifest so it fails to parse but keep `.git/` so the
    // cache classifies as ManifestInvalid (not NotARepo).
    std::fs::write(cache_dir.join("tome-catalog.toml"), "not valid toml = =\n").unwrap();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert_eq!(report.catalogs[0].state, CatalogCacheState::ManifestInvalid);
    let cat_fix = report
        .suggested_fixes
        .iter()
        .find(|f| matches!(f.subsystem, tome::doctor::Subsystem::Catalog(_)))
        .expect("catalog suggested fix");
    assert!(
        !cat_fix.auto_fixable,
        "manifest-invalid should require manual investigation, not --fix",
    );
}

// ---- Phase 3 Polish PR-E — orphan clones + workspace registry --------

#[test]
fn assemble_reports_orphan_clone_in_catalogs_list() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    // Plant a fake catalog clone at `catalogs_dir/<sha>` with `.git/`
    // and a minimal tome-catalog.toml. No config.toml references it —
    // it's an orphan.
    let orphan_cache = paths.catalogs_dir.join("orphan-cache");
    std::fs::create_dir_all(orphan_cache.join(".git")).unwrap();
    std::fs::write(
        orphan_cache.join("tome-catalog.toml"),
        "[catalog]\nname = \"orphan\"\nversion = \"0.1.0\"\n[[plugins]]\nname = \"x\"\nsource = \"./x\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    // The minimal source dir mentioned by the manifest.
    std::fs::create_dir_all(orphan_cache.join("x")).unwrap();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    let orphan = report
        .catalogs
        .iter()
        .find(|c| c.state == CatalogCacheState::Orphan)
        .expect("orphan entry in report.catalogs");
    assert_eq!(orphan.cache_path, orphan_cache);

    // Orphan does NOT trip Degraded — informational per contract.
    assert_eq!(report.overall, DoctorClassification::Ok);

    let orphan_fix = report
        .suggested_fixes
        .iter()
        .find(|f| matches!(f.subsystem, tome::doctor::Subsystem::Catalog(_)))
        .expect("orphan suggested fix");
    assert!(
        !orphan_fix.auto_fixable,
        "orphan removal is NOT auto-fixable per contract",
    );
    assert!(
        orphan_fix.command.starts_with("rm -rf"),
        "orphan fix command names the path: {}",
        orphan_fix.command,
    );
}

#[test]
fn workspace_registry_status_reports_absent_by_default() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(!report.workspace_registry.present);
    assert_eq!(report.workspace_registry.tracked, 0);
}

#[test]
#[ignore = "F11: workspace registry is replaced by the workspace_projects junction table"]
fn workspace_registry_status_reports_present_count_after_opt_in() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    std::fs::create_dir_all(&paths.logs_dir).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    // Opt in by touching the file with two valid absolute paths.
    let temp_a = TempDir::new().unwrap();
    let temp_b = TempDir::new().unwrap();
    std::fs::write(
        &paths.global_config_file,
        format!("{}\n{}\n", temp_a.path().display(), temp_b.path().display(),),
    )
    .unwrap();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(report.workspace_registry.present);
    assert_eq!(report.workspace_registry.tracked, 2);
}

// ---- --fix repairs -----------------------------------------------------

#[test]
fn fix_repairs_broken_catalog_cache_and_re_classifies_ok() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let fix = Fixture::build_sample();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    let cache_dir = cache_dir_for(&env, &fix.url);
    std::fs::remove_dir_all(cache_dir.join(".git")).unwrap();

    let mut report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert_eq!(report.overall, DoctorClassification::Degraded);

    let attempts = doctor::fixes::apply(
        &mut report,
        &doctor::fixes::FixContext {
            paths: &paths,
            scope: &global_scope(),
            home: home.path(),
            force: false,
        },
    );
    assert!(attempts >= 1);
    doctor::fixes::re_assemble(&mut report);

    // Catalog cache should be restored to Ok; overall flips back.
    assert_eq!(report.catalogs[0].state, CatalogCacheState::Ok);
    assert_eq!(report.overall, DoctorClassification::Ok);
    assert!(report.suggested_fixes.is_empty());
    assert!(cache_dir.join(".git").exists());
}

#[test]
fn has_remaining_manual_fixes_detects_unfixable_after_fix_pass() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let fix = Fixture::build_sample();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    let cache_dir = cache_dir_for(&env, &fix.url);
    // Manifest-invalid: NOT auto-fixable, so even after --fix runs,
    // `has_remaining_manual_fixes` should return true.
    std::fs::write(cache_dir.join("tome-catalog.toml"), "garbage\n").unwrap();

    let mut report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    doctor::fixes::apply(
        &mut report,
        &doctor::fixes::FixContext {
            paths: &paths,
            scope: &global_scope(),
            home: home.path(),
            force: false,
        },
    );
    doctor::fixes::re_assemble(&mut report);

    assert!(doctor::fixes::has_remaining_manual_fixes(&report));
}

// ---- Harness detection --------------------------------------------------

#[test]
fn harness_detection_finds_existing_directories() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let home = empty_home();
    std::fs::create_dir_all(home.path().join(".claude")).unwrap();
    std::fs::create_dir_all(home.path().join(".cursor")).unwrap();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    let by_name: std::collections::BTreeMap<_, _> = report
        .harnesses
        .iter()
        .map(|h| (h.name.as_str(), h))
        .collect();
    assert!(by_name["claude-code"].present);
    assert!(by_name["cursor"].present);
    assert!(!by_name["codex"].present);
    assert!(!by_name["gemini"].present);
}

// ---- Workspace context --------------------------------------------------

#[test]
fn global_scope_overrides_workspace_in_report() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    // Workspace exists on disk; doctor under --global scope reports
    // global state. We don't simulate the resolver here — we just
    // construct ResolvedScope::global_fallback directly.
    let global_report =
        doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert_eq!(
        global_report.workspace.scope,
        tome::workspace::ScopeKind::Global
    );
}

// ---- CLI exit codes -----------------------------------------------------

#[test]
fn cli_doctor_with_no_models_exits_1() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let out = env.cmd().args(["doctor"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(1),
        "expected exit 1; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Suggested fixes:"), "{stdout}");
}

#[test]
fn cli_doctor_with_healthy_state_exits_0() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let out = env.cmd().args(["doctor"]).output().unwrap();
    assert!(
        out.status.success(),
        "exit={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("healthy"), "{stdout}");
}

#[test]
fn cli_doctor_fix_with_manifest_invalid_exits_75() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let fix = Fixture::build_sample();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    let cache_dir = cache_dir_for(&env, &fix.url);
    std::fs::write(cache_dir.join("tome-catalog.toml"), "garbage\n").unwrap();

    let out = env.cmd().args(["doctor", "--fix"]).output().unwrap();
    assert_eq!(
        out.status.code(),
        Some(75),
        "expected exit 75 for unfixable manifest; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
}

// ---- --fix schema migration (Phase 4 / F9 — real production migration) -

#[test]
fn fix_runs_forward_schema_migration_end_to_end() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Phase 4 / F9: the production `MIGRATIONS` table now carries
    // `phase_4_v1_to_v2`, so `doctor::build_suggested_fixes` emits a
    // `subsystem: "schema"` SuggestedFix naturally when the on-disk
    // schema is older than `SCHEMA_VERSION`. This test no longer
    // injects a synthetic fix (T085); it relies on the real production
    // trigger end-to-end.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    // Bootstrap the index at `SCHEMA_VERSION` (= 2) using the production
    // registry seeds, then downgrade-stamp `schema_version` to 1 to
    // simulate an existing Phase 2/3 install on disk. The `skills` table
    // bootstrapped here is already v2 shape (no `enabled` column); the
    // migration body's `INSERT INTO skills_new SELECT * FROM skills`
    // copies whatever rows exist (zero in this test) into the rebuilt
    // table. The marker we verify is the presence of the `workspaces`
    // table — which is also already present from the v2 bootstrap, so
    // the assertion shifts to "schema_version row goes from 1 → 2".
    {
        let (embedder, reranker, summariser) = tome::commands::plugin::registry_seeds();
        let conn = tome::index::open(
            &paths.index_db,
            &tome::index::OpenOptions {
                embedder,
                reranker,
                summariser,
                profile: None,
            },
        )
        .expect("bootstrap v2 index");
        // Drop the `workspaces` table content so the migration's seed
        // insert has somewhere to land without a unique-name collision.
        // The migration also creates the table; on a v2 bootstrap this
        // would conflict, so we drop the entire pre-migration shape and
        // let the migration recreate it.
        conn.execute_batch(
            "DROP TABLE IF EXISTS workspace_projects;
             DROP TABLE IF EXISTS workspace_catalogs;
             DROP TABLE IF EXISTS workspace_skills;
             DROP TABLE IF EXISTS workspaces;",
        )
        .expect("strip v2 workspace tables");
        conn.execute(
            "UPDATE meta SET value = '1' WHERE key = 'schema_version'",
            [],
        )
        .expect("downgrade stamp to v1");
    }

    // Assemble — with a real migration registered, the suggested-fix
    // list MUST contain a `subsystem: "schema"` entry without any
    // manual injection.
    let mut report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    let has_schema_fix = report
        .suggested_fixes
        .iter()
        .any(|f| f.subsystem == "schema" && f.auto_fixable);
    assert!(
        has_schema_fix,
        "build_suggested_fixes must auto-emit a schema fix when on-disk < SCHEMA_VERSION; \
         got: {:#?}",
        report.suggested_fixes,
    );

    let attempts = doctor::fixes::apply(
        &mut report,
        &doctor::fixes::FixContext {
            paths: &paths,
            scope: &global_scope(),
            home: home.path(),
            force: false,
        },
    );
    assert!(attempts >= 1, "expected at least one repair attempt");
    doctor::fixes::re_assemble(&mut report);

    assert_eq!(
        report.index.schema_version,
        Some(tome::index::SCHEMA_VERSION),
        "schema_version must be bumped to {} by the migration",
        tome::index::SCHEMA_VERSION,
    );
    assert!(
        report.index.integrity_ok,
        "post-migration index must report integrity_ok",
    );
    assert_eq!(
        report.overall,
        DoctorClassification::Ok,
        "post-fix classification must be Ok; report = {report:#?}",
    );

    // The real migration creates the four workspace tables. Verify the
    // `workspaces` table exists and seeded the privileged `global` row.
    let conn = rusqlite::Connection::open(&paths.index_db).unwrap();
    let global_workspace_present: bool = conn
        .query_row("SELECT 1 FROM workspaces WHERE name = 'global'", [], |_| {
            Ok(true)
        })
        .unwrap_or(false);
    assert!(
        global_workspace_present,
        "phase_4_v1_to_v2 must seed the `global` workspace row",
    );
}

// ---- Drift coverage (Phase 3 Polish — Blocker B3) ----------------------

/// Bootstrap a real v1 index DB at the scope path using **stub seeds**.
/// `check_drift` then compares against the production registry seeds
/// (bge-small-en-v1.5 / bge-reranker-base) — stub vs production names
/// disagree, triggering `EmbedderNameDrift` + `RerankerDrift`.
fn bootstrap_index_with_stub_seeds(paths: &tome::paths::Paths) {
    std::fs::create_dir_all(&paths.root).unwrap();
    let _ = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: tome::index::MetaSeed {
                name: "stub-embedder".into(),
                version: "0".into(),
            },
            reranker: tome::index::MetaSeed {
                name: "stub-reranker".into(),
                version: "0".into(),
            },
            summariser: tome::index::MetaSeed {
                name: "stub-summariser".into(),
                version: "0".into(),
            },
            profile: None,
        },
    )
    .expect("bootstrap v1 index with stub seeds");
}

#[test]
fn embedder_name_drift_classifies_unhealthy() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    bootstrap_index_with_stub_seeds(&paths);
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    // The DB records `stub-embedder` but the configured embedder is
    // bge-small-en-v1.5 — name drift.
    assert!(
        matches!(
            report.drift,
            tome::index::DriftStatus::EmbedderNameDrift { .. }
        ),
        "expected EmbedderNameDrift, got {:?}",
        report.drift,
    );
    assert_eq!(
        report.overall,
        DoctorClassification::Unhealthy,
        "embedder drift must flip overall to Unhealthy; report = {report:#?}",
    );

    // Suggested fix uses subsystem `Drift` (the diagnosis text
    // discriminates between embedder/reranker/summariser drift) and is
    // NOT auto-fixable.
    let drift_fix = report
        .suggested_fixes
        .iter()
        .find(|f| {
            f.subsystem == tome::doctor::Subsystem::Drift && f.diagnosis.starts_with("embedder:")
        })
        .expect("embedder-drift fix entry");
    assert!(
        !drift_fix.auto_fixable,
        "embedder drift requires `tome reindex --force`, not --fix",
    );
    assert!(
        drift_fix.command.starts_with("tome reindex"),
        "drift fix command: {}",
        drift_fix.command,
    );
}

#[test]
fn reranker_drift_alone_classifies_degraded() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Bootstrap with the production embedder seed but a stub reranker
    // seed. Only the reranker drifts, so overall classifies Degraded
    // (not Unhealthy — embedder drift is the load-bearing one).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let (real_embedder_seed, _real_reranker_seed, real_summariser_seed) =
        tome::commands::plugin::registry_seeds();
    let _ = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: real_embedder_seed,
            reranker: tome::index::MetaSeed {
                name: "stub-reranker".into(),
                version: "0".into(),
            },
            summariser: real_summariser_seed,
            profile: None,
        },
    )
    .expect("bootstrap");
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(
        matches!(report.drift, tome::index::DriftStatus::RerankerDrift { .. }),
        "expected RerankerDrift, got {:?}",
        report.drift,
    );
    assert_eq!(
        report.overall,
        DoctorClassification::Degraded,
        "reranker drift alone must classify Degraded; report = {report:#?}",
    );

    let drift_fix = report
        .suggested_fixes
        .iter()
        .find(|f| {
            f.subsystem == tome::doctor::Subsystem::Drift && f.diagnosis.starts_with("reranker")
        })
        .expect("reranker drift fix entry");
    assert!(!drift_fix.auto_fixable);
}

#[test]
fn no_drift_reported_when_seeds_match_registry() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Bootstrap with the production registry seeds — no drift, no
    // suggested fix entry, no classification penalty from drift.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let (embedder, reranker, summariser) = tome::commands::plugin::registry_seeds();
    let _ = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder,
            reranker,
            summariser,
            profile: None,
        },
    )
    .expect("bootstrap");
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(matches!(report.drift, tome::index::DriftStatus::None));
    assert!(
        report
            .suggested_fixes
            .iter()
            .all(|f| f.subsystem != tome::doctor::Subsystem::Drift),
        "no drift suggested-fix entries when seeds match",
    );
    assert_eq!(report.overall, DoctorClassification::Ok);
}

// ---- Helpers -----------------------------------------------------------

fn cache_dir_for(env: &ToolEnv, url: &str) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(url.as_bytes());
    env.catalogs_dir().join(hex::encode(h.finalize()))
}

#[allow(dead_code)]
fn _silence(_: &Path) {}

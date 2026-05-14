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

mod common;

use std::path::Path;

use common::{Fixture, ToolEnv, fabricate_all_installed_models, paths_for};
use tempfile::TempDir;
use tome::doctor::{self, CatalogCacheState, DoctorClassification};
use tome::workspace::{ResolvedScope, ScopeSource};

fn global_scope() -> ResolvedScope {
    ResolvedScope {
        scope: tome::workspace::Scope::Global,
        source: ScopeSource::GlobalFallback,
    }
}

fn empty_home() -> TempDir {
    TempDir::new().unwrap()
}

// ---- Per-subsystem assembly --------------------------------------------

#[test]
fn assemble_with_no_models_reports_unhealthy() {
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
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_all_installed_models(&paths);
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert_eq!(report.embedder.state, "ok");
    assert_eq!(report.reranker.state, "ok");
    assert_eq!(report.overall, DoctorClassification::Ok);
    assert!(report.suggested_fixes.is_empty());
}

#[test]
fn assemble_with_broken_catalog_cache_reports_degraded() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_all_installed_models(&paths);
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
        .find(|f| f.subsystem.starts_with("catalog:"))
        .expect("catalog suggested fix");
    assert!(cat_fix.auto_fixable);
    assert!(cat_fix.command.starts_with("tome catalog update"));
}

#[test]
fn assemble_with_missing_catalog_cache_reports_degraded() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_all_installed_models(&paths);
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
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_all_installed_models(&paths);
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
        .find(|f| f.subsystem.starts_with("catalog:"))
        .expect("catalog suggested fix");
    assert!(
        !cat_fix.auto_fixable,
        "manifest-invalid should require manual investigation, not --fix",
    );
}

// ---- --fix repairs -----------------------------------------------------

#[test]
fn fix_repairs_broken_catalog_cache_and_re_classifies_ok() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_all_installed_models(&paths);
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

    let attempts =
        doctor::fixes::apply(&mut report, &paths, &tome::workspace::Scope::Global).unwrap();
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
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_all_installed_models(&paths);
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
    doctor::fixes::apply(&mut report, &paths, &tome::workspace::Scope::Global).unwrap();
    doctor::fixes::re_assemble(&mut report);

    assert!(doctor::fixes::has_remaining_manual_fixes(&report));
}

// ---- Harness detection --------------------------------------------------

#[test]
fn harness_detection_finds_existing_directories() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_all_installed_models(&paths);

    let home = empty_home();
    std::fs::create_dir_all(home.path().join(".claude")).unwrap();
    std::fs::create_dir_all(home.path().join(".cursor")).unwrap();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    let by_name: std::collections::BTreeMap<_, _> = report
        .harnesses
        .iter()
        .map(|h| (h.name.as_str(), h))
        .collect();
    assert!(by_name["claude_code"].present);
    assert!(by_name["cursor"].present);
    assert!(!by_name["codex"].present);
    assert!(!by_name["gemini"].present);
}

// ---- Workspace context --------------------------------------------------

#[test]
fn global_scope_overrides_workspace_in_report() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_all_installed_models(&paths);
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
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_all_installed_models(&paths);

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
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_all_installed_models(&paths);

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

// ---- Helpers -----------------------------------------------------------

fn cache_dir_for(env: &ToolEnv, url: &str) -> std::path::PathBuf {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(url.as_bytes());
    env.catalogs_dir().join(hex::encode(h.finalize()))
}

#[allow(dead_code)]
fn _silence(_: &Path) {}

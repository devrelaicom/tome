//! Phase 4 / US5.a (T376) — FR-560 informational note coverage.
//!
//! A fixture home dir with `.gemini/` present plus an effective list that
//! does NOT include `gemini` (e.g. `[claude-code]`) must surface `gemini`
//! in `detected_uninstalled_harnesses` without affecting overall
//! classification. The note is purely informational: developers see what
//! they could enable, without doctor pestering them about it.

use crate::common::{ToolEnv, fabricate_all_registry_models, paths_for};
use tempfile::TempDir;
use tome::doctor::{self, DoctorClassification};
use tome::workspace::ResolvedScope;

#[test]
fn gemini_dir_present_but_not_in_effective_list_is_detected_uninstalled() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    // Effective list excludes gemini — only claude-code is declared.
    std::fs::write(
        &paths.global_settings_file,
        "harnesses = [\"claude-code\"]\n",
    )
    .unwrap();

    let home_tmp = TempDir::new().unwrap();
    std::fs::create_dir(home_tmp.path().join(".gemini")).unwrap();

    let report = doctor::assemble_report(
        &ResolvedScope::global_fallback(),
        &paths,
        home_tmp.path(),
        false,
    )
    .unwrap();

    // FR-560: gemini surfaces as detected-but-uninstalled.
    assert!(
        report
            .detected_uninstalled_harnesses
            .iter()
            .any(|n| n == "gemini"),
        "expected gemini in detected_uninstalled_harnesses; got {:?}",
        report.detected_uninstalled_harnesses,
    );
    // claude-code is configured but not detected on this machine, so
    // it should NOT appear in the uninstalled list (the filter is
    // "machine-detected AND not in effective list").
    assert!(
        !report
            .detected_uninstalled_harnesses
            .iter()
            .any(|n| n == "claude-code"),
        "claude-code is in the effective list, must not be in detected_uninstalled",
    );
    // Classification is not affected by detected_uninstalled per
    // FR-560.
    assert!(
        matches!(
            report.overall,
            DoctorClassification::Ok | DoctorClassification::Degraded,
        ),
        "detected_uninstalled must not push to Unhealthy; got {:?}",
        report.overall,
    );
}

#[test]
fn empty_machine_yields_empty_detected_uninstalled() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home_tmp = TempDir::new().unwrap();

    let report = doctor::assemble_report(
        &ResolvedScope::global_fallback(),
        &paths,
        home_tmp.path(),
        false,
    )
    .unwrap();
    assert!(report.detected_uninstalled_harnesses.is_empty());
}

#[test]
fn detected_uninstalled_is_sorted_for_determinism() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home_tmp = TempDir::new().unwrap();
    // Create three detect dirs in a non-sorted order.
    for dir in &[".opencode", ".claude", ".cursor", ".gemini"] {
        std::fs::create_dir(home_tmp.path().join(dir)).unwrap();
    }
    // Empty effective list; expect all four to surface.
    let report = doctor::assemble_report(
        &ResolvedScope::global_fallback(),
        &paths,
        home_tmp.path(),
        false,
    )
    .unwrap();
    let mut sorted = report.detected_uninstalled_harnesses.clone();
    sorted.sort();
    assert_eq!(
        report.detected_uninstalled_harnesses, sorted,
        "detected_uninstalled_harnesses must be deterministically sorted",
    );
    // Must include all four.
    for name in &["claude-code", "cursor", "gemini", "opencode"] {
        assert!(
            report
                .detected_uninstalled_harnesses
                .contains(&(*name).to_owned()),
            "missing {name} in {:?}",
            report.detected_uninstalled_harnesses,
        );
    }
}

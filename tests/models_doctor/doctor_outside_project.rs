//! Phase 4 / US5.b — FR-564 coverage. From outside any project marker,
//! `tome doctor` must:
//!
//! - Resolve to the privileged `global` workspace via `GlobalFallback`.
//! - Leave `project_binding == None`.
//! - Use the global scope's effective harness list for the harness
//!   subsystem checks. With no global declarations, the effective list
//!   is `None` and the per-harness vectors are empty.
//! - Not fail classification on absent project-relative files.
//!
//! Some of this is already covered in `tests/doctor_p4.rs`; this file
//! pins the FR-564 contract specifically + adds a positive assertion
//! on the resolution `source`.

use crate::common::{ToolEnv, fabricate_all_registry_models, paths_for};
use tempfile::TempDir;
use tome::doctor::{self, DoctorClassification};
use tome::workspace::{ResolvedScope, ScopeSource};

#[test]
fn doctor_outside_project_resolves_global_with_no_binding() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = TempDir::new().unwrap();

    let scope = ResolvedScope::global_fallback();
    assert!(matches!(scope.source, ScopeSource::GlobalFallback));
    assert!(scope.project_root.is_none());

    let report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    assert!(
        report.project_binding.is_none(),
        "FR-564: outside any project marker, project_binding must be None",
    );
    assert!(
        report.effective_harness_list.is_none(),
        "no project + no global harness declarations → effective list is None",
    );
    assert!(report.harness_rules.is_empty());
    assert!(report.harness_mcp.is_empty());
    assert_eq!(
        report.overall,
        DoctorClassification::Ok,
        "fresh-install global scope with fabricated models should classify Ok",
    );
}

#[test]
fn doctor_outside_project_with_global_harnesses_uses_global_effective_list() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = TempDir::new().unwrap();
    // Declare a harness in global config.
    // Task 2: global harness settings now live in config.toml [harness].enabled.
    std::fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"claude-code\"]\n",
    )
    .unwrap();

    let scope = ResolvedScope::global_fallback();
    let report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    // Effective list resolves to the global declaration.
    assert!(report.effective_harness_list.is_some());
    // Without a project root, the per-harness file checks can't
    // resolve — C-M1: the vectors now carry `NotApplicable` entries
    // (one per declared harness) so JSON consumers distinguish
    // "no global harnesses" from "globally declared, no project context".
    // Classification stays unaffected per FR-561.
    assert!(report.project_binding.is_none());
    assert_eq!(report.harness_rules.len(), 1);
    assert_eq!(report.harness_rules[0].harness, "claude-code");
    assert_eq!(
        report.harness_rules[0].health,
        tome::doctor::SubsystemHealth::NotApplicable,
    );
    assert_eq!(report.harness_mcp.len(), 1);
    assert_eq!(report.harness_mcp[0].harness, "claude-code");
    assert_eq!(
        report.harness_mcp[0].health,
        tome::doctor::SubsystemHealth::NotApplicable,
    );
}

//! Phase 4 / US5.a (T374) — per-subsystem doctor coverage for the new
//! Phase 4 surfaces: binding, binding-rules-copy, summariser, harness
//! rules + MCP integration, and detected-uninstalled-harnesses
//! information.
//!
//! The harness rules + MCP cases exercise the report-assembly + classifier
//! paths without spinning up a real harness file tree — the production
//! `harness_integration::check_harness_integration` is invoked indirectly
//! via `assemble_report`, but the per-harness file paths are computed
//! against TempDir-rooted project dirs.

mod common;

use common::{ToolEnv, fabricate_all_registry_models, paths_for};
use tempfile::TempDir;
use tome::doctor::{self, DoctorClassification, RulesCopyState, Subsystem, SubsystemHealth};
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

fn global_scope() -> ResolvedScope {
    ResolvedScope::global_fallback()
}

fn empty_home() -> TempDir {
    TempDir::new().unwrap()
}

fn project_scope(project_root: std::path::PathBuf, ws_name: &str) -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::parse(ws_name).unwrap()),
        source: ScopeSource::ProjectMarker,
        project_root: Some(project_root),
    }
}

// ---- Binding subsystem -----------------------------------------------------

#[test]
fn binding_outside_project_is_none() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(
        report.project_binding.is_none(),
        "outside any project marker, project_binding must be None (FR-564)",
    );
}

#[test]
fn binding_healthy_when_marker_and_rules_align() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let ws = WorkspaceName::parse("alpha").unwrap();
    let src_rules = paths.workspace_rules_file(&ws);
    std::fs::create_dir_all(src_rules.parent().unwrap()).unwrap();
    std::fs::write(&src_rules, b"shared rules\n").unwrap();

    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(
        project_root.join(".tome/config.toml"),
        "workspace = \"alpha\"\n",
    )
    .unwrap();
    std::fs::write(project_root.join(".tome/RULES.md"), b"shared rules\n").unwrap();

    let scope = project_scope(project_root, "alpha");
    let report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    let binding = report.project_binding.expect("binding present");
    assert!(binding.config_well_formed);
    assert_eq!(binding.rules_file_drift, RulesCopyState::Match);
}

#[test]
fn binding_marker_malformed_classifies_unhealthy() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    // Marker missing the required `workspace` field.
    std::fs::write(project_root.join(".tome/config.toml"), "extra = 1\n").unwrap();

    let scope = project_scope(project_root, "alpha");
    let report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    let binding = report.project_binding.unwrap();
    assert!(!binding.config_well_formed);
    assert_eq!(report.overall, DoctorClassification::Unhealthy);
    assert!(
        report
            .suggested_fixes
            .iter()
            .any(|f| f.subsystem == Subsystem::Binding && !f.auto_fixable),
        "malformed binding must surface a non-auto-fixable suggestion",
    );
}

/// Polish C-M12: `Binding`-broken emits TWO `SuggestedFix` entries
/// (one per executable remediation command) rather than one entry
/// with a compound prose `command` string. JSON consumers parsing the
/// `command` field as one runnable shell line need this split.
#[test]
fn binding_broken_emits_two_split_suggested_fixes_each_with_single_command() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(project_root.join(".tome/config.toml"), "extra = 1\n").unwrap();

    let scope = project_scope(project_root, "alpha");
    let report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();

    let binding_fixes: Vec<_> = report
        .suggested_fixes
        .iter()
        .filter(|f| f.subsystem == Subsystem::Binding)
        .collect();
    assert_eq!(
        binding_fixes.len(),
        2,
        "expected exactly 2 Binding fixes (rebind + recreate); got {:#?}",
        binding_fixes,
    );
    for f in &binding_fixes {
        assert!(!f.auto_fixable, "binding fixes must be manual");
        // Each `command` line is a single executable invocation, not
        // a compound "X, or Y" string. We check there is no embedded
        // ", or " separator (C-M12 regression marker).
        assert!(
            !f.command.contains(", or "),
            "C-M12: command should be one executable line, not compound: {:?}",
            f.command,
        );
    }
    // The two fixes cover the two remediation paths: rebind to an
    // existing workspace, OR recreate the named workspace via init.
    assert!(
        binding_fixes
            .iter()
            .any(|f| f.command.starts_with("tome workspace use")),
        "expected one rebind suggestion, got: {:#?}",
        binding_fixes,
    );
    assert!(
        binding_fixes
            .iter()
            .any(|f| f.command.starts_with("tome workspace init")),
        "expected one recreate suggestion, got: {:#?}",
        binding_fixes,
    );
}

#[test]
fn binding_rules_copy_drift_classifies_degraded() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let ws = WorkspaceName::parse("beta").unwrap();
    let src_rules = paths.workspace_rules_file(&ws);
    std::fs::create_dir_all(src_rules.parent().unwrap()).unwrap();
    std::fs::write(&src_rules, b"canonical body\n").unwrap();

    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(
        project_root.join(".tome/config.toml"),
        "workspace = \"beta\"\n",
    )
    .unwrap();
    std::fs::write(
        project_root.join(".tome/RULES.md"),
        b"hand-edited divergent\n",
    )
    .unwrap();

    let scope = project_scope(project_root, "beta");
    let report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    let binding = report.project_binding.unwrap();
    assert_eq!(binding.rules_file_drift, RulesCopyState::Drift);
    assert_eq!(report.overall, DoctorClassification::Degraded);
    assert!(
        report
            .suggested_fixes
            .iter()
            .any(|f| f.subsystem == Subsystem::BindingRulesCopy && f.auto_fixable),
        "BindingRulesCopy drift must be auto-fixable",
    );
}

#[test]
fn binding_rules_copy_missing_classifies_degraded() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(
        project_root.join(".tome/config.toml"),
        "workspace = \"gamma\"\n",
    )
    .unwrap();
    // Source RULES.md exists but the project's copy is absent — the
    // pure "Missing" (copy-side) state. R-M5 distinguishes this from
    // SourceMissing (workspace-side absent).
    let ws = tome::workspace::WorkspaceName::parse("gamma").unwrap();
    let src = paths.workspace_rules_file(&ws);
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, b"canonical\n").unwrap();

    let scope = project_scope(project_root, "gamma");
    let report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    let binding = report.project_binding.unwrap();
    assert_eq!(binding.rules_file_drift, RulesCopyState::Missing);
    assert!(
        report
            .suggested_fixes
            .iter()
            .any(|f| f.subsystem == Subsystem::BindingRulesCopy),
    );
}

// ---- Summariser subsystem -------------------------------------------------

#[test]
fn summariser_missing_classifies_unhealthy() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    // Don't fabricate models — summariser will read as `missing`.
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert_eq!(report.summariser.state, "missing");
    assert_eq!(report.overall, DoctorClassification::Unhealthy);
    assert!(
        report
            .suggested_fixes
            .iter()
            .any(|f| f.subsystem == Subsystem::Summariser && f.auto_fixable),
        "summariser missing must surface an auto-fixable suggestion",
    );
}

#[test]
fn summariser_present_classifies_ok() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert_eq!(report.summariser.state, "ok");
}

// ---- detected_uninstalled_harnesses (FR-560) ------------------------------

#[test]
fn detected_uninstalled_lists_machine_harnesses_not_in_effective_list() {
    // Fixture home with .gemini/ present; effective list is empty
    // (no project, no harness declarations in global settings) so
    // Gemini ends up in the uninstalled-but-detected list.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let home_tmp = TempDir::new().unwrap();
    std::fs::create_dir_all(home_tmp.path().join(".gemini")).unwrap();

    let report = doctor::assemble_report(&global_scope(), &paths, home_tmp.path(), false).unwrap();
    // The presence list (Phase 3 surface) keeps reporting Gemini as detected.
    assert!(
        report
            .harnesses
            .iter()
            .any(|h| h.name == "gemini" && h.present)
    );
    // FR-560: gemini should also surface as "detected but not configured"
    // because the effective list is empty.
    assert!(
        report
            .detected_uninstalled_harnesses
            .iter()
            .any(|name| name == "gemini"),
        "gemini directory present but not in effective list → expected in \
         detected_uninstalled_harnesses; got: {:?}",
        report.detected_uninstalled_harnesses,
    );
    // Classification stays unaffected by detected_uninstalled.
    assert!(matches!(
        report.overall,
        DoctorClassification::Ok | DoctorClassification::Degraded,
    ));
}

#[test]
fn outside_any_project_resolves_global_with_no_binding() {
    // FR-564: from outside any project marker, doctor resolves to
    // `global`, `project_binding` is None, and harness subsystems
    // report against the global effective list (empty in this fixture).
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(report.project_binding.is_none());
    assert!(report.harness_rules.is_empty());
    assert!(report.harness_mcp.is_empty());
}

// ---- Effective harness list snapshot --------------------------------------

#[test]
fn effective_list_is_none_when_no_scope_declares_harnesses() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    assert!(report.effective_harness_list.is_none());
}

// ---- Harness rules-file integration (T367 indirect) -----------------------

#[test]
fn harness_subsystems_not_applicable_when_no_project_root() {
    // From outside any project marker, the per-harness file paths can't
    // resolve (they're project-relative). C-M1: the classifier now
    // emits per-harness `NotApplicable` entries (one per declared
    // harness) so JSON consumers can distinguish "no harnesses declared"
    // (empty Vec) from "harnesses declared but no project context"
    // (Vec of NotApplicable). Classification stays unaffected per
    // FR-561.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    // Set the global settings.toml to declare a harness; binding is None
    // so harness subsystems resolve to NotApplicable entries.
    std::fs::write(
        &paths.global_settings_file,
        "harnesses = [\"claude-code\"]\n",
    )
    .unwrap();
    let home = empty_home();

    let report = doctor::assemble_report(&global_scope(), &paths, home.path(), false).unwrap();
    // Effective list resolves to the declared harness.
    assert!(report.effective_harness_list.is_some());
    // Per-harness Vec is populated with NotApplicable entries — one per
    // declared harness — rather than empty.
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

#[test]
fn harness_unsupported_resolves_to_broken_subsystem() {
    // When the effective list contains a harness for which the
    // per-project rules file or MCP config doesn't exist, the harness
    // integration check reports Broken — even when a project IS bound.
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();

    // Project bound to the global workspace; global declares claude-code.
    std::fs::write(
        &paths.global_settings_file,
        "harnesses = [\"claude-code\"]\n",
    )
    .unwrap();

    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(
        project_root.join(".tome/config.toml"),
        "workspace = \"global\"\nharnesses = [\"[global]\"]\n",
    )
    .unwrap();

    let scope = project_scope(project_root, "global");
    let report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    assert!(report.effective_harness_list.is_some());
    assert_eq!(report.harness_rules.len(), 1);
    assert_eq!(report.harness_mcp.len(), 1);
    // Both should be Broken — no harness files on disk.
    assert_eq!(report.harness_rules[0].harness, "claude-code");
    assert_eq!(report.harness_rules[0].health, SubsystemHealth::Broken);
    assert_eq!(report.harness_mcp[0].harness, "claude-code");
    assert_eq!(report.harness_mcp[0].health, SubsystemHealth::Broken);
    // Classification flips to Degraded (harness rules/mcp broken).
    assert_eq!(report.overall, DoctorClassification::Degraded);
}

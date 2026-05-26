//! Phase 4 / US5.b — repair-handler tests for `tome doctor --fix` against
//! the new Phase 4 subsystems (summariser re-download, binding-rules-copy
//! re-copy, harness-rules / harness-mcp re-sync) plus the `--force`
//! override path for user-owned MCP entries.
//!
//! Tests target the library API `doctor::fixes::apply` rather than the
//! CLI binary so they can drive the dispatch lattice without a real
//! summariser download. The summariser-redownload case is gated behind
//! the existing fabricate-models test harness (sparse-file fixtures);
//! the production `download_summariser_model` path is intentionally NOT
//! exercised here — same boundary as `tests/doctor.rs` for the embedder
//! repair.
//!
//! Cross-test serialisation
//! ------------------------
//!
//! Harness override tests share `HARNESS_MODULES_OVERRIDE` with the rest
//! of the test suite. Per `harness_sync_stub.rs`'s convention, the file
//! owns a single `OVERRIDE_MUTEX` held for the lifetime of every test
//! that installs an override.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use common::{
    HarnessModulesGuard, ToolEnv, fabricate_all_registry_models, paths_for, seed_workspace,
};
use tempfile::TempDir;
use tome::doctor::{self, DoctorClassification, RulesCopyState, Subsystem, SubsystemHealth};
use tome::harness::StubHarness;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

fn project_scope(project_root: PathBuf, ws_name: &str) -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::parse(ws_name).unwrap()),
        source: ScopeSource::ProjectMarker,
        project_root: Some(project_root),
    }
}

fn empty_home() -> TempDir {
    TempDir::new().unwrap()
}

/// Insert a `(workspace_name, project_path)` row in `workspace_projects`
/// so `workspace::sync::sync_one` enumerates the project on its walk.
fn bind_project_in_db(paths: &tome::paths::Paths, ws_name: &str, project_root: &Path) {
    let (e, r, s) = (
        common::stub_embedder_seed(),
        common::stub_reranker_seed(),
        common::stub_summariser_seed(),
    );
    let conn = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: e,
            reranker: r,
            summariser: s,
        },
    )
    .expect("open index for project bind");
    let workspace_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![ws_name],
            |row| row.get(0),
        )
        .expect("workspace row");
    let now_unix = time::OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO workspace_projects (workspace_id, project_path, bound_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![workspace_id, project_root.to_str().unwrap(), now_unix],
    )
    .expect("insert workspace_projects");
}

// =====================================================================
// BindingRulesCopy: --fix re-copies the workspace RULES.md
// =====================================================================

#[test]
fn missing_binding_rules_copy_fix_recopies() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();
    seed_workspace(&paths, "alpha");

    // Workspace RULES.md (source of truth).
    let ws = WorkspaceName::parse("alpha").unwrap();
    let src = paths.workspace_rules_file(&ws);
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, b"alpha canonical rules\n").unwrap();

    // Project bound to alpha.
    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(
        project_root.join(".tome/config.toml"),
        "workspace = \"alpha\"\n",
    )
    .unwrap();
    // Intentionally NO project RULES.md — the missing case.

    bind_project_in_db(&paths, "alpha", &project_root);

    let scope = project_scope(project_root.clone(), "alpha");

    let mut report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    assert_eq!(
        report.project_binding.as_ref().unwrap().rules_file_drift,
        RulesCopyState::Missing,
    );
    // Apply --fix.
    let attempts = doctor::fixes::apply(
        &mut report,
        &doctor::fixes::FixContext {
            paths: &paths,
            scope: &scope,
            home: home.path(),
            force: false,
        },
    );
    assert!(attempts >= 1, "expected at least one fix attempt");
    doctor::fixes::re_assemble(&mut report);

    // File restored from workspace's RULES.md.
    let dest = project_root.join(".tome/RULES.md");
    let bytes = std::fs::read(&dest).expect("RULES.md must exist after fix");
    assert_eq!(bytes, b"alpha canonical rules\n");
    assert_eq!(
        report.project_binding.as_ref().unwrap().rules_file_drift,
        RulesCopyState::Match,
    );
}

#[test]
fn drifted_binding_rules_copy_fix_recopies() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();
    seed_workspace(&paths, "beta");

    let ws = WorkspaceName::parse("beta").unwrap();
    let src = paths.workspace_rules_file(&ws);
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, b"canonical v2\n").unwrap();

    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(
        project_root.join(".tome/config.toml"),
        "workspace = \"beta\"\n",
    )
    .unwrap();
    // Project copy hand-edited.
    std::fs::write(
        project_root.join(".tome/RULES.md"),
        b"hand edited divergent\n",
    )
    .unwrap();

    bind_project_in_db(&paths, "beta", &project_root);

    let scope = project_scope(project_root.clone(), "beta");
    let mut report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    assert_eq!(
        report.project_binding.as_ref().unwrap().rules_file_drift,
        RulesCopyState::Drift,
    );

    doctor::fixes::apply(
        &mut report,
        &doctor::fixes::FixContext {
            paths: &paths,
            scope: &scope,
            home: home.path(),
            force: false,
        },
    );
    doctor::fixes::re_assemble(&mut report);

    let bytes = std::fs::read(project_root.join(".tome/RULES.md")).unwrap();
    assert_eq!(bytes, b"canonical v2\n");
    assert_eq!(
        report.project_binding.as_ref().unwrap().rules_file_drift,
        RulesCopyState::Match,
    );
}

// =====================================================================
// Binding broken (orphan workspace): NOT auto-fixable, even with --force
// =====================================================================

#[test]
fn binding_broken_orphan_workspace_is_not_auto_fixable_with_force() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();
    // Only the privileged `global` workspace is seeded — the project
    // points at `not-registered` which doesn't exist in the DB.
    // Stamp meta so the DB is reachable but the workspace is missing.
    common::write_config_for_cli(&paths, &tome::config::Config::default());

    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(
        project_root.join(".tome/config.toml"),
        "workspace = \"not-registered\"\n",
    )
    .unwrap();

    let scope = project_scope(project_root.clone(), "not-registered");
    let mut report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    assert!(!report.project_binding.as_ref().unwrap().config_well_formed);
    assert_eq!(report.overall, DoctorClassification::Unhealthy);

    doctor::fixes::apply(
        &mut report,
        &doctor::fixes::FixContext {
            paths: &paths,
            scope: &scope,
            home: home.path(),
            // Even with `--force`, we don't auto-rebind.
            force: true,
        },
    );
    doctor::fixes::re_assemble(&mut report);

    // The binding subsystem stays broken; the suggested fix is still
    // present + non-auto-fixable.
    assert!(!report.project_binding.as_ref().unwrap().config_well_formed);
    let still_broken = report
        .suggested_fixes
        .iter()
        .any(|f| f.subsystem == Subsystem::Binding && !f.auto_fixable);
    assert!(
        still_broken,
        "binding-broken suggestion must persist post-fix; got: {:#?}",
        report.suggested_fixes,
    );
    assert!(doctor::fixes::has_remaining_manual_fixes(&report));
}

// =====================================================================
// HarnessRules drift: --fix re-runs sync
// =====================================================================

#[test]
fn drifted_harness_rules_fix_resyncs() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness)]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();
    seed_workspace(&paths, "drift-ws");

    let ws = WorkspaceName::parse("drift-ws").unwrap();
    let src = paths.workspace_rules_file(&ws);
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, b"canonical rules\n").unwrap();

    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(
        project_root.join(".tome/config.toml"),
        "workspace = \"drift-ws\"\nharnesses = [\"stub\"]\n",
    )
    .unwrap();
    std::fs::write(project_root.join(".tome/RULES.md"), b"canonical rules\n").unwrap();
    bind_project_in_db(&paths, "drift-ws", &project_root);

    // First sync to populate the stub rules + mcp files.
    let sync_deps = tome::harness::sync::SyncDeps {
        paths: &paths,
        home_root: home.path(),
        workspace_name: &ws,
        force: false,
    };
    tome::harness::sync::sync_project(&project_root, &sync_deps).expect("initial sync");

    // Corrupt the stub rules block body.
    let stub_rules = project_root.join("STUB_RULES.md");
    let contents = std::fs::read_to_string(&stub_rules).unwrap();
    let corrupted = contents.replace("canonical rules", "MANGLED");
    std::fs::write(&stub_rules, corrupted).unwrap();

    let scope = project_scope(project_root.clone(), "drift-ws");
    let mut report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    let rules_state = &report.harness_rules;
    assert!(rules_state.iter().any(|h| h.harness == "stub"
        && matches!(h.health, SubsystemHealth::Drift | SubsystemHealth::Broken)));

    doctor::fixes::apply(
        &mut report,
        &doctor::fixes::FixContext {
            paths: &paths,
            scope: &scope,
            home: home.path(),
            force: false,
        },
    );
    doctor::fixes::re_assemble(&mut report);

    // Stub rules block restored.
    let after = std::fs::read_to_string(&stub_rules).unwrap();
    assert!(
        after.contains("canonical rules"),
        "post-fix STUB_RULES.md must contain the canonical body; got: {after}",
    );
    let stub_rules_health = report
        .harness_rules
        .iter()
        .find(|h| h.harness == "stub")
        .expect("stub harness in report");
    assert_eq!(stub_rules_health.health, SubsystemHealth::Ok);
}

// =====================================================================
// HarnessMcp broken: --fix re-syncs
// =====================================================================

#[test]
fn missing_harness_mcp_fix_resyncs() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness)]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();
    seed_workspace(&paths, "mcp-ws");
    let ws = WorkspaceName::parse("mcp-ws").unwrap();
    let src = paths.workspace_rules_file(&ws);
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, b"x\n").unwrap();

    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(
        project_root.join(".tome/config.toml"),
        "workspace = \"mcp-ws\"\nharnesses = [\"stub\"]\n",
    )
    .unwrap();
    bind_project_in_db(&paths, "mcp-ws", &project_root);

    let sync_deps = tome::harness::sync::SyncDeps {
        paths: &paths,
        home_root: home.path(),
        workspace_name: &ws,
        force: false,
    };
    tome::harness::sync::sync_project(&project_root, &sync_deps).expect("initial sync");

    // Delete the stub.mcp.json to put the mcp subsystem into Broken.
    let mcp_path = project_root.join("stub.mcp.json");
    std::fs::remove_file(&mcp_path).unwrap();

    let scope = project_scope(project_root.clone(), "mcp-ws");
    let mut report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    assert!(
        report
            .harness_mcp
            .iter()
            .any(|h| h.harness == "stub" && h.health == SubsystemHealth::Broken)
    );

    doctor::fixes::apply(
        &mut report,
        &doctor::fixes::FixContext {
            paths: &paths,
            scope: &scope,
            home: home.path(),
            force: false,
        },
    );
    doctor::fixes::re_assemble(&mut report);

    assert!(mcp_path.is_file(), "stub.mcp.json must be re-created");
    let stub_mcp_health = report
        .harness_mcp
        .iter()
        .find(|h| h.harness == "stub")
        .unwrap();
    assert_eq!(stub_mcp_health.health, SubsystemHealth::Ok);
}

// =====================================================================
// User-owned HarnessMcp: --fix without --force refuses; --fix --force
// rewrites.
// =====================================================================

#[test]
fn user_owned_harness_mcp_fix_without_force_leaves_user_entry() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness)]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();
    seed_workspace(&paths, "user-mcp");
    let ws = WorkspaceName::parse("user-mcp").unwrap();
    let src = paths.workspace_rules_file(&ws);
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, b"x\n").unwrap();

    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(
        project_root.join(".tome/config.toml"),
        "workspace = \"user-mcp\"\nharnesses = [\"stub\"]\n",
    )
    .unwrap();
    bind_project_in_db(&paths, "user-mcp", &project_root);

    // Pre-populate a user-owned `tome` entry (command != "tome" → not
    // Tome-owned per mcp_config::is_tome_owned). Sync would normally
    // refuse to overwrite this without --force.
    let mcp_path = project_root.join("stub.mcp.json");
    std::fs::write(
        &mcp_path,
        r#"{
  "mcpServers": {
    "tome": {
      "command": "evil",
      "args": ["custom"]
    }
  }
}"#,
    )
    .unwrap();
    // We must also create STUB_RULES.md so the rules subsystem doesn't
    // independently fail — keep the failure surface scoped to MCP.
    std::fs::write(
        project_root.join("STUB_RULES.md"),
        "<!-- tome:begin -->\n<!-- tome:end -->\n",
    )
    .unwrap();

    let scope = project_scope(project_root.clone(), "user-mcp");
    let mut report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();
    assert!(
        report
            .harness_mcp
            .iter()
            .any(|h| h.harness == "stub" && h.health == SubsystemHealth::UserOwned),
        "expected UserOwned for the stub harness; got: {:#?}",
        report.harness_mcp,
    );

    // --fix without --force: the user-owned entry must NOT be rewritten.
    doctor::fixes::apply(
        &mut report,
        &doctor::fixes::FixContext {
            paths: &paths,
            scope: &scope,
            home: home.path(),
            force: false,
        },
    );
    doctor::fixes::re_assemble(&mut report);

    let after = std::fs::read_to_string(&mcp_path).unwrap();
    assert!(
        after.contains("\"evil\""),
        "user-owned entry must survive a non-forced --fix; got: {after}",
    );
    assert!(
        doctor::fixes::has_remaining_manual_fixes(&report),
        "user-owned MCP must remain in the residual fix list",
    );
}

#[test]
fn user_owned_harness_mcp_fix_force_rewrites_to_tome_owned() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness)]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);
    let home = empty_home();
    seed_workspace(&paths, "force-mcp");
    let ws = WorkspaceName::parse("force-mcp").unwrap();
    let src = paths.workspace_rules_file(&ws);
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, b"x\n").unwrap();

    let project_tmp = TempDir::new().unwrap();
    let project_root = project_tmp.path().to_path_buf();
    std::fs::create_dir_all(project_root.join(".tome")).unwrap();
    std::fs::write(
        project_root.join(".tome/config.toml"),
        "workspace = \"force-mcp\"\nharnesses = [\"stub\"]\n",
    )
    .unwrap();
    bind_project_in_db(&paths, "force-mcp", &project_root);

    let mcp_path = project_root.join("stub.mcp.json");
    std::fs::write(
        &mcp_path,
        r#"{
  "mcpServers": {
    "tome": {
      "command": "evil",
      "args": ["custom"]
    }
  }
}"#,
    )
    .unwrap();
    std::fs::write(
        project_root.join("STUB_RULES.md"),
        "<!-- tome:begin -->\n<!-- tome:end -->\n",
    )
    .unwrap();

    let scope = project_scope(project_root.clone(), "force-mcp");
    let mut report = doctor::assemble_report(&scope, &paths, home.path(), false).unwrap();

    // --fix --force MUST rewrite the entry to the Tome-owned shape.
    doctor::fixes::apply(
        &mut report,
        &doctor::fixes::FixContext {
            paths: &paths,
            scope: &scope,
            home: home.path(),
            force: true,
        },
    );
    doctor::fixes::re_assemble(&mut report);

    let after = std::fs::read_to_string(&mcp_path).unwrap();
    assert!(
        !after.contains("\"evil\""),
        "user-owned `evil` command must be replaced; got: {after}",
    );
    assert!(
        after.contains("\"tome\""),
        "rewrite must install the Tome-owned `command = tome`; got: {after}",
    );
    assert!(
        after.contains("--workspace") && after.contains("force-mcp"),
        "rewrite must include the bound workspace name in args; got: {after}",
    );

    let stub_mcp_health = report
        .harness_mcp
        .iter()
        .find(|h| h.harness == "stub")
        .unwrap();
    assert_eq!(stub_mcp_health.health, SubsystemHealth::Ok);
    // The mcp subsystem in particular must NOT still appear in the
    // residual fix list — the force path repaired it. Other subsystems
    // (e.g. the rules subsystem, given the synthetic empty-block setup)
    // may still have non-auto-fixable suggestions; we don't assert
    // against the global `has_remaining_manual_fixes` for that reason.
    assert!(
        !report
            .suggested_fixes
            .iter()
            .any(|f| matches!(&f.subsystem, Subsystem::HarnessMcp(n) if n == "stub")),
        "post-force-rewrite, the stub MCP fix must be gone from the suggested list; \
         got: {:#?}",
        report.suggested_fixes,
    );
}

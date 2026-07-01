//! Library-API tests for `tome harness use` — across all three scopes.
//! Each test installs a single-entry stub override (named `"stub"` so
//! the sync orchestrator's effective list recognises it) and asserts
//! that the correct settings file was edited.

use crate::common::{HarnessModulesGuard, HomeGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::cli::{HarnessScopeArg, HarnessUseArgs};
use tome::commands::harness::use_;
use tome::harness::StubHarness;
use tome::output::Mode;
use tome::paths::Paths;
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

fn make_resolved_scope(name: &str, project_root: Option<std::path::PathBuf>) -> ResolvedScope {
    let scope = Scope(WorkspaceName::parse(name).unwrap());
    let source = if project_root.is_some() {
        ScopeSource::ProjectMarker
    } else {
        ScopeSource::GlobalFallback
    };
    ResolvedScope {
        scope,
        source,
        project_root,
        overridden_project_marker: None,
    }
}

#[test]
fn use_unknown_harness_errors_with_exit_18() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessUseArgs {
        names: vec!["totally-not-a-harness".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Global),
        force: false,
    };
    let scope = make_resolved_scope("global", None);
    let err = use_::run(args, &scope, &paths, Mode::Json).expect_err("unknown");
    assert_eq!(err.exit_code(), 18, "want HarnessNotSupported; got {err:?}");
}

#[test]
fn use_project_scope_without_project_errors_with_usage() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessUseArgs {
        names: vec!["stub".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Project),
        force: false,
    };
    let scope = make_resolved_scope("global", None);
    let err = use_::run(args, &scope, &paths, Mode::Json).expect_err("usage");
    assert_eq!(err.exit_code(), 2, "want Usage; got {err:?}");
}

#[test]
fn use_global_scope_writes_global_settings_file() {
    // Task 2: global scope now writes to config.toml [harness].enabled,
    // not settings.toml. This test is updated accordingly.
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessUseArgs {
        names: vec!["stub".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Global),
        force: false,
    };
    let scope = make_resolved_scope("global", None);
    use_::run(args, &scope, &paths, Mode::Json).expect("use ok");

    let body = std::fs::read_to_string(&paths.global_config_file).expect("global config");
    assert!(
        body.contains("stub"),
        "global config must include stub: {body}"
    );
}

/// Phase 11 / US4 (M4): `tome harness use generic-op` (global scope) is ACCEPTED
/// — the opt-in target lives in `OPT_IN_TARGETS`, not `SUPPORTED_HARNESSES`, so
/// `run` must resolve it via the alias+opt-in-aware `lookup` rather than
/// erroring exit 18. Driven against the REAL registry (NO `HarnessModulesGuard`
/// override, since opt-in targets are not in the override slot) at global scope,
/// so no project sync runs — only the name validation + settings write.
#[test]
fn use_generic_op_global_scope_is_accepted() {
    // No `HarnessModulesGuard` — opt-in targets are resolved through the real
    // `OPT_IN_TARGETS` registry, which `lookup` consults. Still serialise on the
    // override mutex so a co-resident test's installed override can't leak in and
    // shadow the real registry mid-run.
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    let args = HarnessUseArgs {
        names: vec!["generic-op".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Global),
        force: false,
    };
    let scope = make_resolved_scope("global", None);
    use_::run(args, &scope, &paths, Mode::Json).expect("use generic-op ok (not exit 18)");

    // Task 2: global scope now writes to config.toml [harness].enabled.
    let body = std::fs::read_to_string(&paths.global_config_file).expect("global config");
    assert!(
        body.contains("generic-op"),
        "global config must include generic-op: {body}",
    );
}

#[test]
fn use_workspace_scope_writes_workspace_settings_file() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "demo");

    let args = HarnessUseArgs {
        names: vec!["stub".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Workspace),
        force: false,
    };
    let scope = make_resolved_scope("demo", None);
    use_::run(args, &scope, &paths, Mode::Json).expect("use ok");

    let ws_settings = paths.workspaces_dir.join("demo/settings.toml");
    let body = std::fs::read_to_string(&ws_settings).expect("workspace settings");
    assert!(
        body.contains("stub"),
        "workspace settings must include stub: {body}"
    );
}

#[test]
fn use_project_scope_writes_project_marker() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    // Build a project marker pointing at `global`.
    let project_dir = TempDir::new().unwrap();
    let marker_dir = project_dir.path().join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(marker_dir.join("config.toml"), "workspace = \"global\"\n").unwrap();

    // Pre-populate $HOME for any harness detect() calls during sync.
    let _home = HomeGuard::install(env.home_path());

    let args = HarnessUseArgs {
        names: vec!["stub".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Project),
        force: false,
    };
    let scope = make_resolved_scope("global", Some(project_dir.path().to_path_buf()));
    let result = use_::run(args, &scope, &paths, Mode::Json);

    result.expect("use ok");

    let marker_body =
        std::fs::read_to_string(Paths::project_marker_config(project_dir.path())).unwrap();
    assert!(
        marker_body.contains("stub"),
        "project marker must include stub: {marker_body}"
    );
}

/// T-M3 (US3 review) — `tome harness use --force` exercises the FR-502
/// wiring through to `sync::build_deps(..., force: true)`.
///
/// The other six tests in this file pass `force: false`. Add one that
/// runs against a project marker and a synthetic stub harness with
/// `force: true`; we don't assert on the sync orchestrator's clash
/// behaviour (the stub harness produces no MCP entries with a clash),
/// only that the flag round-trips through use_::run without surfacing
/// an unexpected error.
#[test]
fn use_with_force_true_propagates_to_sync_deps() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    // Project marker bound to global.
    let project_dir = TempDir::new().unwrap();
    let marker_dir = project_dir.path().join(".tome");
    std::fs::create_dir_all(&marker_dir).unwrap();
    std::fs::write(marker_dir.join("config.toml"), "workspace = \"global\"\n").unwrap();

    let _home = HomeGuard::install(env.home_path());

    let args = HarnessUseArgs {
        names: vec!["stub".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Project),
        force: true,
    };
    let scope = make_resolved_scope("global", Some(project_dir.path().to_path_buf()));
    use_::run(args, &scope, &paths, Mode::Json).expect("use --force ok");

    // The settings file must have the entry; the sync path ran under
    // `force: true` so any pre-existing user-owned MCP entry would have
    // been overwritten without exit 19.
    let marker_body =
        std::fs::read_to_string(Paths::project_marker_config(project_dir.path())).unwrap();
    assert!(
        marker_body.contains("stub"),
        "project marker must include stub: {marker_body}",
    );
}

#[test]
fn use_idempotent_when_name_already_present_does_not_invoke_sync() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    // `global` is auto-seeded by index bootstrap; no manual seed needed.

    // Task 2: global scope now writes to config.toml [harness].enabled.
    // Pre-write global config with stub already present.
    std::fs::write(
        &paths.global_config_file,
        "[harness]\nenabled = [\"stub\"]\n",
    )
    .unwrap();
    let mtime_before = std::fs::metadata(&paths.global_config_file)
        .unwrap()
        .modified()
        .unwrap();

    // Sleep so mtime granularity can advance if a write happens.
    std::thread::sleep(std::time::Duration::from_millis(1100));

    let args = HarnessUseArgs {
        names: vec!["stub".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Global),
        force: false,
    };
    let scope = make_resolved_scope("global", None);
    use_::run(args, &scope, &paths, Mode::Json).expect("use ok");

    let mtime_after = std::fs::metadata(&paths.global_config_file)
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(
        mtime_before, mtime_after,
        "config file must not be rewritten"
    );
}

/// Task 2: global scope must write `[harness] enabled` in `config.toml`,
/// not a top-level `harnesses` key in `settings.toml`.
#[test]
fn use_global_scope_writes_config_harness_table() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(StubHarness::default())]);

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let args = HarnessUseArgs {
        names: vec!["stub".to_string()],
        all: false,
        scope: Some(HarnessScopeArg::Global),
        force: false,
    };
    let scope = make_resolved_scope("global", None);
    use_::run(args, &scope, &paths, Mode::Json).expect("use ok");

    let body = std::fs::read_to_string(&paths.global_config_file).unwrap();
    assert!(
        body.contains("[harness]"),
        "config.toml must have [harness]: {body}"
    );
    assert!(
        body.contains("stub"),
        "config.toml must include stub: {body}"
    );
    // settings.toml must NOT be created anymore
    assert!(
        !paths.root.join("settings.toml").exists(),
        "settings.toml must not exist after global harness use"
    );
    // Round-trip assertion: config::load must parse back the written file
    // and return the harness name in the enabled list.
    let parsed = tome::config::load(&paths).expect("round-trip load must succeed");
    assert_eq!(
        parsed.harness.enabled.as_deref(),
        Some(&["stub".to_string()][..]),
        "round-trip: harness.enabled must contain exactly [stub]"
    );
}

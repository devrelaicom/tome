//! Phase 4 / US2 slice a — `tome workspace info`.
//!
//! Library-API tests for `commands::workspace::info::assemble`. The
//! happy-path counts depend on enabling a plugin against the
//! `sample-plugin-catalog` fixture (mirrors `tests/status.rs`'s setup);
//! bootstrap-not-yet is tested directly with no index file at all.
//!
//! `tests/workspace_info_cli.rs`-style binary coverage is deferred —
//! the assembly logic is pure compute and the CLI wrapper is a thin
//! emit. The exit-code contract is enforced by `tests/exit_codes.rs`.

use crate::common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, lifecycle_paths, write_config_for_cli,
};
use tempfile::TempDir;
use tome::commands::plugin::registry_seeds;
use tome::commands::workspace::info::assemble;
use tome::embedding::stub::StubEmbedder;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, Scope, ScopeKind, ScopeSource};

fn global_scope() -> ResolvedScope {
    ResolvedScope::global_fallback()
}

fn workspace_scope(path: &std::path::Path) -> ResolvedScope {
    // F10: Scope is a name; the project_root carries the bound path.
    let name = tome::workspace::WorkspaceName::parse("test-ws").unwrap();
    ResolvedScope {
        scope: Scope(name),
        source: ScopeSource::Flag,
        project_root: Some(path.to_path_buf()),
    }
}

fn enable_alpha(paths: &tome::paths::Paths, config: &tome::config::Config) {
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let embedder = StubEmbedder::new();
    let scope = Scope(tome::workspace::WorkspaceName::global());
    let deps = LifecycleDeps {
        paths,
        scope: &scope,
        config,
        embedder: &embedder,
        embedder_seed,
        reranker_seed,
        summariser_seed,
        allow_model_download: false,
    };
    lifecycle::enable(&id, &deps).expect("enable alpha");
}

#[test]
fn info_global_with_no_state_reports_zero_counts() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());

    let info = assemble(&global_scope(), &paths).expect("assemble");
    assert_eq!(info.scope, ScopeKind::Global);
    assert!(info.path.is_none());
    assert_eq!(info.source, ScopeSource::GlobalFallback);
    assert_eq!(info.catalogs, 0);
    assert_eq!(info.plugins_total, 0);
    assert_eq!(info.plugins_enabled, 0);
    assert_eq!(info.skills_indexed, 0);
    assert!(info.schema_version.is_none());
    assert!(info.embedder.is_none());
}

#[test]
fn info_global_with_enabled_plugin_reports_populated_counts() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);
    enable_alpha(&paths, &config);

    let info = assemble(&global_scope(), &paths).expect("assemble");
    assert_eq!(info.scope, ScopeKind::Global);
    assert_eq!(info.catalogs, 1);
    assert_eq!(info.plugins_total, 1);
    assert_eq!(info.plugins_enabled, 1);
    assert_eq!(info.skills_indexed, 4);
    assert_eq!(info.schema_version, Some(tome::index::SCHEMA_VERSION));
    let embedder = info.embedder.as_ref().expect("embedder identity present");
    let (expected, _, _) = registry_seeds();
    assert_eq!(embedder.name, expected.name);
    assert_eq!(embedder.version, expected.version);
}

#[test]
#[ignore = "F11: workspace-scoped reads target the central DB via workspace_catalogs; F2a routes every read to the global config"]
fn info_workspace_scope_reads_workspace_paths() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();

    // Create a workspace directory containing a `.tome/config.toml` with
    // one catalog. The index DB is NOT bootstrapped — exercise the
    // bootstrap-not-yet read path under a workspace scope.
    let workspace_root = tmp.path().join("project");
    std::fs::create_dir_all(workspace_root.join(".tome")).unwrap();
    let catalog_root = copy_sample_plugin_catalog(&tmp, "ws-catalog");
    let config = config_with_catalog("ws-catalog", &catalog_root);
    let config_path = workspace_root.join(".tome/config.toml");
    let body = toml::to_string_pretty(&config).unwrap();
    std::fs::write(&config_path, body).unwrap();

    let scope = workspace_scope(&workspace_root);
    let info = assemble(&scope, &paths).expect("assemble");
    assert_eq!(info.scope, ScopeKind::Workspace);
    assert_eq!(info.path.as_deref(), Some(workspace_root.as_path()));
    assert_eq!(info.source, ScopeSource::Flag);
    assert_eq!(info.catalogs, 1);
    assert_eq!(info.plugins_total, 0);
    assert_eq!(info.plugins_enabled, 0);
    assert_eq!(info.skills_indexed, 0);
    assert!(info.schema_version.is_none());
    assert!(info.embedder.is_none());
}

#[test]
fn info_json_shape_is_byte_stable_for_bootstrap_not_yet() {
    // Pin the JSON wire format for the not-yet-bootstrapped global case.
    // If a field order or rename drifts, this test breaks loudly — the
    // schema is consumed by `tome doctor` (data-model §5) and by users
    // piping into `jq`.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let info = assemble(&global_scope(), &paths).expect("assemble");
    let json = serde_json::to_string(&info).expect("serialise");
    assert_eq!(
        json,
        r#"{"scope":"global","path":null,"source":"global_fallback","catalogs":0,"plugins_total":0,"plugins_enabled":0,"skills_indexed":0,"schema_version":null,"embedder":null,"enrolled_catalogs":[],"enabled_plugins":[],"bound_projects":[],"summary_cache":null}"#,
    );
}

#[test]
fn info_json_includes_workspace_path_under_workspace_scope() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let workspace_root = tmp.path().join("ws");
    std::fs::create_dir_all(workspace_root.join(".tome")).unwrap();
    let info = assemble(&workspace_scope(&workspace_root), &paths).expect("assemble");
    let json = serde_json::to_string(&info).expect("serialise");
    assert!(json.contains(r#""scope":"workspace""#));
    assert!(json.contains(r#""source":"flag""#));
    let expected_path = serde_json::to_string(&workspace_root).unwrap();
    assert!(
        json.contains(&format!(r#""path":{expected_path}"#)),
        "json={json} expected_path={expected_path}",
    );
}

#[test]
#[ignore = "F11: workspace-scoped reads target the central DB; the malformed-config gate moves to workspace_projects validation"]
fn info_workspace_scope_with_malformed_config_returns_workspace_malformed() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    let workspace_root = tmp.path().join("broken");
    std::fs::create_dir_all(workspace_root.join(".tome")).unwrap();
    let config_path = workspace_root.join(".tome/config.toml");
    std::fs::write(&config_path, "this is = not = valid = toml").unwrap();

    let err = assemble(&workspace_scope(&workspace_root), &paths).unwrap_err();
    assert!(
        matches!(err, tome::error::TomeError::WorkspaceMalformed { .. }),
        "expected WorkspaceMalformed, got {err:?}",
    );
    assert_eq!(err.exit_code(), 70);
}

// CLI-binary smoke test: bare `tome workspace info` (no flag, no env,
// no marker) falls back to global and exits 0. Exercises the dispatcher
// + emit path that the library tests bypass. Phase 4 / F10 removed the
// `--global` flag; the fallback IS the privileged-default surface now.
#[test]
fn cli_workspace_info_prints_and_exits_zero() {
    let env = ToolEnv::new();
    // Run the CLI inside a directory with no `.tome/config.toml` so the
    // resolver's project-marker walk falls through cleanly.
    let scratch = TempDir::new().unwrap();
    let output = env
        .cmd()
        .current_dir(scratch.path())
        .args(["workspace", "info"])
        .output()
        .expect("spawn tome");
    assert!(
        output.status.success(),
        "exit={:?} stdout={} stderr={}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Workspace:"), "stdout={stdout}");
    assert!(stdout.contains("(global)"), "stdout={stdout}");
}

#[test]
fn cli_workspace_info_json_emits_single_line() {
    let env = ToolEnv::new();
    let scratch = TempDir::new().unwrap();
    let output = env
        .cmd()
        .current_dir(scratch.path())
        .args(["--json", "workspace", "info"])
        .output()
        .expect("spawn tome");
    assert!(output.status.success(), "exit={:?}", output.status.code());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid json");
    assert_eq!(parsed["scope"], "global");
    assert_eq!(parsed["path"], serde_json::Value::Null);
    assert_eq!(parsed["source"], "global_fallback");
}

// =============================================================
// Phase 4 / US2.a-1 — enrolled catalogs, enabled plugins, bound
// projects, summary cache state, and the optional <name> positional.
// =============================================================

#[test]
fn info_includes_enrolled_catalogs() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);

    let info = assemble(&global_scope(), &paths).expect("assemble");
    assert_eq!(info.enrolled_catalogs.len(), 1);
    assert_eq!(info.enrolled_catalogs[0].name, "sample-plugin-catalog");
    assert!(info.enrolled_catalogs[0].pinned_ref == "main");
}

#[test]
fn info_includes_enabled_plugins() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);
    enable_alpha(&paths, &config);

    let info = assemble(&global_scope(), &paths).expect("assemble");
    assert_eq!(info.enabled_plugins.len(), 1);
    let plugin = &info.enabled_plugins[0];
    assert_eq!(plugin.catalog, "sample-plugin-catalog");
    assert_eq!(plugin.plugin, "plugin-alpha");
    assert!(plugin.skill_count > 0);
}

#[test]
fn info_bound_projects_empty_when_no_bindings() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);
    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);

    let info = assemble(&global_scope(), &paths).expect("assemble");
    assert!(info.bound_projects.is_empty());
}

#[test]
fn info_summary_cache_none_when_settings_absent() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);
    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);

    let info = assemble(&global_scope(), &paths).expect("assemble");
    assert!(info.summary_cache.is_none());
}

#[test]
fn info_with_name_argument_targets_other_workspace() {
    use tome::commands::workspace::info::assemble as _assemble;
    // assemble takes the resolved scope, but the CLI's `info <name>`
    // goes through `run` which routes to `assemble_for_name`. That
    // helper is private to the module; we exercise it indirectly via
    // the public `WorkspaceInfoArgs` shape through the run entry. For
    // a library-API test, we mirror the same code path: open the DB,
    // seed two workspaces, then assert `assemble` on each scope.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // Seed central DB with `extra` workspace.
    crate::common::seed_workspace(&paths, "extra");

    // Resolved as the extra workspace via the Flag source.
    let name = tome::workspace::WorkspaceName::parse("extra").unwrap();
    let resolved = tome::workspace::ResolvedScope {
        scope: tome::workspace::Scope(name.clone()),
        source: tome::workspace::ScopeSource::Flag,
        project_root: None,
    };
    let info = _assemble(&resolved, &paths).expect("assemble extra");
    assert_eq!(info.scope, ScopeKind::Workspace);
    assert_eq!(info.source, tome::workspace::ScopeSource::Flag);
}

#[test]
fn info_missing_workspace_returns_workspace_not_found() {
    // DB exists, workspace_catalogs table exists, but the named
    // workspace was never created → exit 13.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::seed_workspace(&paths, "real"); // bootstraps DB with v2 schema
    let name = tome::workspace::WorkspaceName::parse("not-real").unwrap();
    let resolved = tome::workspace::ResolvedScope {
        scope: tome::workspace::Scope(name),
        source: tome::workspace::ScopeSource::Flag,
        project_root: None,
    };
    let err = assemble(&resolved, &paths).unwrap_err();
    assert!(
        matches!(err, tome::error::TomeError::WorkspaceNotFound { .. }),
        "expected WorkspaceNotFound, got {err:?}",
    );
    assert_eq!(err.exit_code(), 13);
}

// =============================================================
// Tiered-skill-routing — `tome workspace info --details`.
// =============================================================

#[test]
fn workspace_info_details_lists_entries_with_tiers() {
    use tome::commands::workspace::info::assemble_with_details;

    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    crate::common::fabricate_all_registry_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);
    enable_alpha(&paths, &config);

    // With `details = true`, the report gains a `plugin_details` array
    // with per-entry routing tiers.
    let info = assemble_with_details(&global_scope(), &paths, true).expect("assemble details");
    let details = info
        .plugin_details
        .as_ref()
        .expect("plugin_details present with details=true");
    assert!(!details.is_empty(), "expected at least one plugin");
    let pd = &details[0];
    assert_eq!(pd.plugin, "plugin-alpha");
    assert!(!pd.skills.is_empty(), "expected at least one skill");
    // Every skill/command entry carries a routing tier.
    for s in &pd.skills {
        assert!(s.tier.is_some(), "skill entry missing tier: {s:?}");
    }
    // Freshly-enabled entries carry the default routing tier (3, per the
    // schema-v5 `workspace_skills.tier DEFAULT 3`).
    let has_default_tier = details
        .iter()
        .flat_map(|pd| pd.skills.iter().chain(pd.commands.iter()))
        .any(|e| e.tier == Some(3));
    assert!(has_default_tier, "expected at least one tier-3 entry");

    // The JSON wire surfaces the field + numeric tiers under `--details`.
    let json = serde_json::to_string(&info).expect("serialise");
    assert!(json.contains(r#""plugin_details""#), "json={json}");
    assert!(json.contains(r#""tier":3"#), "json={json}");

    // WITHOUT details, the field is None → absent from the JSON wire
    // (byte-shape unchanged).
    let info_plain = assemble_with_details(&global_scope(), &paths, false).expect("assemble plain");
    assert!(info_plain.plugin_details.is_none());
    let json_plain = serde_json::to_string(&info_plain).expect("serialise");
    assert!(
        !json_plain.contains("plugin_details"),
        "plugin_details must be absent without details: {json_plain}",
    );
}

// =============================================================
// T-M6 (Polish PR-D): byte-stable JSON wire-shape pin for
// `WorkspaceCatalogEntry`. The triple `(name, url, pinned_ref)` is the
// public contract for the enrolled-catalogs array under
// `WorkspaceInfo.enrolled_catalogs`; a future field rename or addition
// would break downstream `jq` consumers silently if not pinned here.
// =============================================================

#[test]
fn workspace_catalog_entry_json_wire_shape_is_byte_stable() {
    use tome::workspace::info::WorkspaceCatalogEntry;
    let entry = WorkspaceCatalogEntry {
        name: "sample-plugin-catalog".to_owned(),
        url: "https://github.com/example/catalog.git".to_owned(),
        pinned_ref: "main".to_owned(),
    };
    let json = serde_json::to_string(&entry).expect("serialise");
    assert_eq!(
        json,
        r#"{"name":"sample-plugin-catalog","url":"https://github.com/example/catalog.git","pinned_ref":"main"}"#,
    );
}

#[test]
fn workspace_catalog_entry_field_order_is_pinned() {
    use tome::workspace::info::WorkspaceCatalogEntry;
    let entry = WorkspaceCatalogEntry {
        name: "x".to_owned(),
        url: "y".to_owned(),
        pinned_ref: "z".to_owned(),
    };
    let json = serde_json::to_string(&entry).expect("serialise");
    let name_idx = json.find("\"name\"").expect("name field present");
    let url_idx = json.find("\"url\"").expect("url field present");
    let pinned_idx = json.find("\"pinned_ref\"").expect("pinned_ref present");
    assert!(name_idx < url_idx, "name must come before url");
    assert!(url_idx < pinned_idx, "url must come before pinned_ref");
}

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

mod common;

use common::{
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
    ResolvedScope {
        scope: Scope::Workspace(path.to_path_buf()),
        source: ScopeSource::Flag,
    }
}

fn enable_alpha(paths: &tome::paths::Paths, config: &tome::config::Config) {
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths,
        scope: &Scope::Global,
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
    common::fabricate_all_registry_models(&paths);

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
        r#"{"scope":"global","path":null,"source":"global_fallback","catalogs":0,"plugins_total":0,"plugins_enabled":0,"skills_indexed":0,"schema_version":null,"embedder":null}"#,
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

// CLI-binary smoke test: bare `tome workspace info` with default `--global`
// (no fixture) prints a human report and exits 0. Exercises the dispatcher
// + emit path that the library tests bypass.
#[test]
fn cli_workspace_info_prints_and_exits_zero() {
    let env = ToolEnv::new();
    let output = env
        .cmd()
        .args(["--global", "workspace", "info"])
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
    let output = env
        .cmd()
        .args(["--json", "--global", "workspace", "info"])
        .output()
        .expect("spawn tome");
    assert!(output.status.success(), "exit={:?}", output.status.code());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(stdout.trim()).expect("valid json");
    assert_eq!(parsed["scope"], "global");
    assert_eq!(parsed["path"], serde_json::Value::Null);
    assert_eq!(parsed["source"], "global_flag");
}

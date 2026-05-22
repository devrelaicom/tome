//! Integration tests for `tome plugin list` against the CLI binary.
//!
//! Unlike `plugin enable`, `plugin list` does not load embedders or
//! rerankers — it only reads the catalog manifest and the index DB. That
//! makes the CLI binary safe to drive directly: no model artefacts needed.
//!
//! Setup pattern:
//!   1. `ToolEnv` provides an isolated `HOME` + XDG layout.
//!   2. Copy the `sample-plugin-catalog` fixture into a TempDir.
//!   3. Write `config.toml` directly to the isolated config dir (bypasses
//!      `catalog add`, which would require the fixture to be a git repo).
//!   4. Pre-enable `plugin-alpha` via the library API so the index has at
//!      least one enabled plugin row.
//!   5. Invoke `tome plugin list --json`.
//!
//! Spec: `contracts/plugin-commands.md` §3.

mod common;

use std::path::Path;

use common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_models, paths_for,
    sample_plugin_catalog_fixture, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
    write_config_for_cli,
};
use serde_json::Value;
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::paths::Paths;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

/// End-to-end setup: register the sample-plugin-catalog under `catalog_name`
/// in the supplied env's config and pre-enable `plugin-alpha`.
fn setup_with_alpha_enabled(env: &ToolEnv, fixture_tmp: &TempDir, catalog_name: &str) -> Paths {
    let paths = paths_for(env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(fixture_tmp, "catalog");

    // Mirror what `tome catalog add` would record: catalog path = the
    // directory containing `tome-catalog.toml`. The fixture lays out plugin
    // directories as immediate children of the catalog root, so the same
    // `Config` works for both the lifecycle library API (`<path>/<plugin>`)
    // and the CLI list path (manifest walk → `<path>/<source>`).
    let cli_config = config_with_catalog(catalog_name, &catalog_root);
    write_config_for_cli(&paths, &cli_config);

    let lib_config = cli_config.clone();
    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &tome::workspace::Scope::Global,
        config: &lib_config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = format!("{catalog_name}/plugin-alpha").parse().unwrap();
    lifecycle::enable(&id, &deps).expect("pre-enable plugin-alpha");

    paths
}

/// Parse NDJSON stdout into a `Vec<Value>`.
fn parse_ndjson(stdout: &[u8]) -> Vec<Value> {
    stdout
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_slice::<Value>(l).expect("parse json line"))
        .collect()
}

#[test]
fn list_emits_both_plugins_with_correct_status() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["plugin", "list", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let records = parse_ndjson(&out.stdout);
    assert_eq!(
        records.len(),
        2,
        "expected two plugin rows, got: {}",
        String::from_utf8_lossy(&out.stdout),
    );

    let by_plugin: std::collections::HashMap<String, &Value> = records
        .iter()
        .map(|r| {
            let id = r["id"]
                .as_object()
                .and_then(|o| o.get("plugin"))
                .and_then(|v| v.as_str())
                .expect("plugin field")
                .to_owned();
            (id, r)
        })
        .collect();

    let alpha = by_plugin
        .get("plugin-alpha")
        .expect("plugin-alpha row missing");
    assert_eq!(alpha["status"], "enabled");
    assert_eq!(alpha["version"], "1.2.3");

    let beta = by_plugin
        .get("plugin-beta")
        .expect("plugin-beta row missing");
    assert_eq!(beta["status"], "disabled");
    assert_eq!(beta["version"], "0.9.0");
}

#[test]
fn list_enabled_only_hides_disabled_plugins() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["plugin", "list", "--enabled-only", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let records = parse_ndjson(&out.stdout);
    assert_eq!(records.len(), 1, "expected one enabled plugin row");
    assert_eq!(records[0]["status"], "enabled");
    assert_eq!(records[0]["id"]["plugin"], "plugin-alpha");
}

#[test]
fn list_catalog_filter_narrows_results() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    // Filter to the catalog we registered — same content as the default
    // list, but exercises the `--catalog` code path explicitly.
    let out = env
        .cmd()
        .args([
            "plugin",
            "list",
            "--catalog",
            "sample-plugin-catalog",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let records = parse_ndjson(&out.stdout);
    assert_eq!(records.len(), 2);
    for r in &records {
        assert_eq!(r["id"]["catalog"], "sample-plugin-catalog");
    }
}

#[test]
fn list_unknown_catalog_filter_exits_with_catalog_not_found_code() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["plugin", "list", "--catalog", "does-not-exist", "--json"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "expected failure for unknown catalog filter",
    );
    // Exit code 3 corresponds to CatalogNotFound (see src/error.rs).
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn list_with_no_catalogs_registered_emits_nothing_in_json_mode() {
    // Sanity check: the empty-config baseline is well-behaved even without
    // a fabricated index DB. Establishes the "zero state" the other tests
    // build on top of.
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["plugin", "list", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        out.stdout.is_empty(),
        "expected empty JSON stream, got {:?}",
        String::from_utf8_lossy(&out.stdout),
    );

    // Cross-check: the fixture itself must exist so future contributors
    // know they should not delete it.
    let fixture = sample_plugin_catalog_fixture();
    assert!(
        fixture.is_dir(),
        "expected sample-plugin-catalog fixture at {}",
        fixture.display(),
    );
    let manifest: &Path = &fixture.join("tome-catalog.toml");
    assert!(manifest.is_file(), "missing tome-catalog.toml in fixture");
}

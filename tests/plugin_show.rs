//! Integration tests for `tome plugin show <id>` against the CLI binary.
//!
//! `plugin show` walks the same plugin tree as `plugin list` but emits a
//! single-record view. The interesting axes are:
//!   * Metadata fields surface (version, author, description).
//!   * Component counts include the skills directory.
//!   * Lenient parsing tolerates extra fields in `plugin.json` (FR-013a).
//!   * Unknown plugin → exit 20 (PluginNotFound).
//!   * Invalid id shape → exit 2 (Usage).
//!
//! Spec: `contracts/plugin-commands.md` §4.

mod common;

use common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_models, paths_for,
    stub_embedder_seed, stub_reranker_seed, stub_summariser_seed, write_config_for_cli,
};
use serde_json::Value;
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::paths::Paths;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

/// Register `sample-plugin-catalog` and enable plugin-alpha, mirroring the
/// `plugin_list.rs` harness. Returns the resolved `Paths` for follow-up
/// assertions.
fn setup(env: &ToolEnv, fixture_tmp: &TempDir) -> Paths {
    let paths = paths_for(env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(fixture_tmp, "catalog");
    let cli_config = config_with_catalog("sample-plugin-catalog", &catalog_root);
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
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("pre-enable plugin-alpha for show tests");

    paths
}

#[test]
fn show_emits_full_metadata_and_component_counts_for_enabled_plugin() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup(&env, &fixture_tmp);

    let out = env
        .cmd()
        .args([
            "plugin",
            "show",
            "sample-plugin-catalog/plugin-alpha",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let record: Value = serde_json::from_slice(&out.stdout)
        .expect("plugin show --json must emit a single JSON record");

    assert_eq!(record["id"]["catalog"], "sample-plugin-catalog");
    assert_eq!(record["id"]["plugin"], "plugin-alpha");
    assert_eq!(record["version"], "1.2.3");
    assert_eq!(record["status"], "enabled");
    assert!(
        record["description"]
            .as_str()
            .map(|s| s.contains("Alpha plugin"))
            .unwrap_or(false),
        "description must come from plugin.json, got {}",
        record["description"],
    );
    assert!(
        record["author"]
            .as_str()
            .map(|s| s.contains("Tome Test Harness"))
            .unwrap_or(false),
        "author display must combine name + email, got {}",
        record["author"],
    );

    // Component counts — the fixture lays out five skill directories under
    // plugin-alpha/skills (one of which carries a malformed YAML body).
    // `count_components` counts directories under `skills/` containing a
    // SKILL.md, regardless of frontmatter validity, so all five count here.
    let counts = &record["component_counts"];
    assert!(
        counts["skills"].as_u64().unwrap() >= 4,
        "expected >=4 skills, got {counts}",
    );
}

#[test]
fn show_tolerates_extra_unknown_fields_in_plugin_json() {
    // The fixture's plugin.json deliberately includes `keywords`,
    // `homepage`, and `unknown_extra_field` — none of which are part of
    // Tome's lenient PluginManifest schema. FR-013a requires the parser to
    // ignore them silently. If this test fails, the manifest parser is no
    // longer lenient.
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup(&env, &fixture_tmp);

    let out = env
        .cmd()
        .args([
            "plugin",
            "show",
            "sample-plugin-catalog/plugin-alpha",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "lenient parser regressed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn show_unknown_plugin_exits_with_plugin_not_found_code() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup(&env, &fixture_tmp);

    let out = env
        .cmd()
        .args(["plugin", "show", "sample-plugin-catalog/ghost"])
        .output()
        .unwrap();
    assert!(!out.status.success(), "expected unknown plugin to fail");
    // PluginNotFound → exit 20 (see src/error.rs).
    assert_eq!(out.status.code(), Some(20));
}

#[test]
fn show_unknown_catalog_exits_with_catalog_not_found_code() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup(&env, &fixture_tmp);

    let out = env
        .cmd()
        .args(["plugin", "show", "ghost-catalog/plugin-alpha"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    // CatalogNotFound → exit 3.
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn show_malformed_id_exits_with_usage_code() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup(&env, &fixture_tmp);

    // Missing the `<catalog>/` half of the address.
    let out = env
        .cmd()
        .args(["plugin", "show", "just-a-plugin-name"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    // Usage → exit 2.
    assert_eq!(out.status.code(), Some(2));
}

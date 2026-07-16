//! Task C2 — `tome plugin show` surfaces the new manifest metadata
//! (`keywords`/`homepage`/`repository`/`bugs`/`documentation`/`display_name`/
//! `contributors`) added to `TomePluginManifest` in Task A3.
//!
//! Mirrors the `plugin_show_p6*` fixture style: one plugin whose manifest
//! carries the new fields (asserts human + JSON render them), and one
//! minimal plugin whose manifest carries none of them (asserts JSON omits
//! them entirely rather than emitting `null`/empty placeholders).

use std::fs;
use std::path::Path;

use serde_json::Value;
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

use crate::common::{
    ToolEnv, config_with_catalog, fabricate_all_registry_models, global_scope, paths_for,
    stub_embedder_seed, stub_reranker_seed, stub_summariser_seed, write_config_for_cli,
};

struct Fixture {
    env: ToolEnv,
    _catalog_tmp: TempDir,
}

/// Stage a plugin enabled in the global workspace, with `manifest_toml`
/// written verbatim as its `tome-plugin.toml`.
fn setup(manifest_toml: &str) -> Fixture {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let catalog_tmp = TempDir::new().unwrap();
    let catalog_root = catalog_tmp.path().join("catalog");
    let plugin_dir = catalog_root.join("plug");
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    fs::write(plugin_dir.join("tome-plugin.toml"), manifest_toml).unwrap();
    fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        r#"{"name": "plug", "version": "1.0.0"}"#,
    )
    .unwrap();
    write_catalog_manifest(&catalog_root, "plug");

    let cli_config = config_with_catalog("acme", &catalog_root);
    write_config_for_cli(&paths, &cli_config);

    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &global_scope(),
        config: &cli_config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "acme/plug".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("enable plugin");

    Fixture {
        env,
        _catalog_tmp: catalog_tmp,
    }
}

fn write_catalog_manifest(catalog_root: &Path, plugin_name: &str) {
    fs::write(
        catalog_root.join("tome-catalog.toml"),
        format!("[[plugins]]\nname = \"{plugin_name}\"\nsource = \"{plugin_name}\"\n"),
    )
    .unwrap();
}

fn show_json(fx: &Fixture) -> Value {
    let out = fx
        .env
        .cmd()
        .args(["plugin", "show", "acme/plug", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    serde_json::from_slice(&out.stdout).expect("valid JSON")
}

fn show_human(fx: &Fixture) -> String {
    let out = fx
        .env
        .cmd()
        .args(["plugin", "show", "acme/plug"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const FULL_MANIFEST: &str = r#"
name = "plug"
version = "1.0.0"
keywords = ["auth", "security"]
homepage = "https://example.com"
repository = "https://example.com/repo"
bugs = "https://example.com/issues"
documentation = "https://example.com/docs"
display_name = "Plug Plugin"

[[contributors]]
name = "Sam"
email = "sam@example.com"

[[contributors]]
name = "Ada"
"#;

#[test]
fn json_includes_present_metadata_and_contributors() {
    let fx = setup(FULL_MANIFEST);
    let json = show_json(&fx);

    assert_eq!(json["display_name"], "Plug Plugin");
    assert_eq!(json["keywords"], serde_json::json!(["auth", "security"]));
    assert_eq!(json["homepage"], "https://example.com");
    assert_eq!(json["repository"], "https://example.com/repo");
    assert_eq!(json["bugs"], "https://example.com/issues");
    assert_eq!(json["documentation"], "https://example.com/docs");
    assert_eq!(
        json["contributors"],
        serde_json::json!(["Sam <sam@example.com>", "Ada"])
    );
}

#[test]
fn human_renders_present_metadata_and_contributors() {
    let fx = setup(FULL_MANIFEST);
    let text = show_human(&fx);

    assert!(text.contains("Display name: Plug Plugin"), "got:\n{text}");
    assert!(
        text.contains("Keywords:     auth, security"),
        "got:\n{text}"
    );
    assert!(
        text.contains("Homepage:     https://example.com"),
        "got:\n{text}"
    );
    assert!(
        text.contains("Repository:   https://example.com/repo"),
        "got:\n{text}"
    );
    assert!(
        text.contains("Bugs:         https://example.com/issues"),
        "got:\n{text}"
    );
    assert!(
        text.contains("Documentation: https://example.com/docs"),
        "got:\n{text}"
    );
    assert!(text.contains("Contributors:"), "got:\n{text}");
    assert!(text.contains("  - Sam <sam@example.com>"), "got:\n{text}");
    assert!(text.contains("  - Ada"), "got:\n{text}");
}

const MINIMAL_MANIFEST: &str = "name = \"plug\"\nversion = \"1.0.0\"\n";

#[test]
fn json_omits_absent_metadata_entirely() {
    let fx = setup(MINIMAL_MANIFEST);
    let json = show_json(&fx);

    for field in [
        "display_name",
        "homepage",
        "repository",
        "bugs",
        "documentation",
    ] {
        assert!(
            json.get(field).is_none(),
            "`{field}` should be absent from JSON when the manifest doesn't set it; got {json}",
        );
    }
    assert!(
        json.get("keywords").is_none() || json["keywords"] == serde_json::json!([]),
        "keywords should be absent or empty when unset; got {json}",
    );
    assert!(
        json.get("contributors").is_none() || json["contributors"] == serde_json::json!([]),
        "contributors should be absent or empty when unset; got {json}",
    );
}

#[test]
fn human_omits_absent_metadata_entirely() {
    let fx = setup(MINIMAL_MANIFEST);
    let text = show_human(&fx);

    for label in [
        "Display name:",
        "Keywords:",
        "Homepage:",
        "Repository:",
        "Bugs:",
        "Documentation:",
        "Contributors:",
    ] {
        assert!(
            !text.contains(label),
            "`{label}` should not render when the manifest doesn't set it; got:\n{text}",
        );
    }
}

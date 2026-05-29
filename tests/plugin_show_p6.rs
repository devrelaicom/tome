//! Phase 6 / US5 — `tome plugin show` extensions smoke test (FIRST cut).
//!
//! Asserts the FR-083 additions to the `--json` envelope: the
//! `ships_hooks_json` / `ships_guardrails_md` presence booleans, and that an
//! agent entry appears in the `agents` array. The full matrix (persona-name
//! surfacing under the toggle, human-mode rendering, byte-stable JSON pin) is
//! the next chunk.

mod common;

use std::fs;
use std::path::Path;

use serde_json::Value;
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

use common::{
    ToolEnv, config_with_catalog, fabricate_all_registry_models, global_scope, paths_for,
    stub_embedder_seed, stub_reranker_seed, stub_summariser_seed, write_config_for_cli,
};

struct Fixture {
    env: ToolEnv,
    _catalog_tmp: TempDir,
}

/// Stage a plugin shipping one agent + `hooks/hooks.json` +
/// `hooks/GUARDRAILS.md`, enabled in the global workspace.
fn setup() -> Fixture {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let catalog_tmp = TempDir::new().unwrap();
    let catalog_root = catalog_tmp.path().join("catalog");
    let plugin_dir = catalog_root.join("plug");
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    fs::create_dir_all(plugin_dir.join("agents")).unwrap();
    fs::create_dir_all(plugin_dir.join("hooks")).unwrap();
    fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        r#"{"name": "plug", "version": "1.0.0"}"#,
    )
    .unwrap();
    fs::write(
        plugin_dir.join("agents").join("reviewer.md"),
        "---\nname: reviewer\ndescription: Reviews code.\n---\nReview carefully.\n",
    )
    .unwrap();
    fs::write(plugin_dir.join("hooks").join("hooks.json"), "{}").unwrap();
    fs::write(
        plugin_dir.join("hooks").join("GUARDRAILS.md"),
        "Be careful.\n",
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

#[test]
fn shows_agents_and_hooks_guardrails_presence_booleans() {
    let fx = setup();
    let json = show_json(&fx);

    assert_eq!(
        json["ships_hooks_json"], true,
        "plugin ships hooks/hooks.json"
    );
    assert_eq!(
        json["ships_guardrails_md"], true,
        "plugin ships hooks/GUARDRAILS.md"
    );

    let agents = json["agents"].as_array().expect("agents array present");
    assert!(
        agents.iter().any(|a| a["name"] == "reviewer"),
        "reviewer agent listed; got {agents:?}"
    );
    // Personas are off (no settings toggle) → no persona_name on the agent.
    let reviewer = agents
        .iter()
        .find(|a| a["name"] == "reviewer")
        .expect("reviewer present");
    assert!(
        reviewer.get("persona_name").is_none(),
        "persona_name absent when expose_agents_as_personas is off; got {reviewer:?}"
    );
}

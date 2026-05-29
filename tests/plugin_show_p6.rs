//! Phase 6 / US5 — `tome plugin show` extensions (FR-083, T133).
//!
//! Asserts the additions to both output paths: the `ships_hooks_json` /
//! `ships_guardrails_md` presence booleans, the per-plugin `agents` array
//! (displayed name), and the per-agent `persona_name` — present only when
//! `expose_agents_as_personas` resolves true at the scope, absent otherwise.
//! Human-mode rendering of the same surfaces is exercised too. The byte-stable
//! JSON wire-pin lives in `tests/plugin_show_p6_json_shape.rs`.

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

/// Turn on `expose_agents_as_personas` at the global scope so the bare-CLI
/// `plugin show` (which resolves to global fallback) surfaces persona names.
fn enable_personas(fx: &Fixture) {
    let paths = paths_for(&fx.env);
    fs::write(
        &paths.global_settings_file,
        "expose_agents_as_personas = true\n",
    )
    .expect("write global settings");
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

#[test]
fn agent_persona_name_surfaces_when_personas_on() {
    let fx = setup();
    enable_personas(&fx);

    let json = show_json(&fx);
    let agents = json["agents"].as_array().expect("agents array present");
    let reviewer = agents
        .iter()
        .find(|a| a["name"] == "reviewer")
        .expect("reviewer present");
    // The single non-clashing agent derives `<name>-persona`.
    assert_eq!(
        reviewer["persona_name"], "reviewer-persona",
        "persona_name present + derived when toggle on; got {reviewer:?}",
    );
}

#[test]
fn human_mode_renders_ship_booleans_agents_and_persona() {
    let fx = setup();
    enable_personas(&fx);

    let text = show_human(&fx);
    assert!(
        text.contains("Ships hooks/hooks.json:    yes"),
        "human mode shows hooks presence; got:\n{text}",
    );
    assert!(
        text.contains("Ships hooks/GUARDRAILS.md: yes"),
        "human mode shows guardrails presence; got:\n{text}",
    );
    assert!(
        text.contains("Agents (1):"),
        "human mode lists the Agents section; got:\n{text}",
    );
    assert!(
        text.contains("reviewer"),
        "the agent's displayed name appears; got:\n{text}",
    );
    assert!(
        text.contains("persona: reviewer-persona"),
        "human mode shows the resolved persona name when the toggle is on; got:\n{text}",
    );
}

#[test]
fn human_mode_omits_persona_when_off() {
    let fx = setup();
    // No personas toggle.

    let text = show_human(&fx);
    assert!(
        text.contains("Agents (1):"),
        "the Agents section still renders; got:\n{text}",
    );
    assert!(
        !text.contains("persona:"),
        "no persona line when the toggle is off; got:\n{text}",
    );
}

//! Phase 6 / US5 — byte-stable JSON wire-shape pin for the `tome plugin show
//! --json` Phase 6 additions (T133) per `doctor-extensions-p6.md` § `tome
//! plugin show` (FR-083).
//!
//! Mirrors the Phase 5 pin style (`plugin_show_p5_json_shape.rs`): the full
//! envelope carries dynamic fields (timestamps) that have their own tests in
//! `plugin_show.rs`, so this file pins the Phase 6 ADDITIONS by field
//! presence + value:
//! - `ships_hooks_json` / `ships_guardrails_md` envelope booleans.
//! - the `agents` array of per-entry projections (displayed `name`).
//! - the per-agent `persona_name` — present + derived when
//!   `expose_agents_as_personas` resolves true, absent (skip-if-none) when off.

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

/// Stage a plugin shipping one agent + `hooks/hooks.json` +
/// `hooks/GUARDRAILS.md`, enabled in the global workspace. `ships_hooks` /
/// `ships_guardrails` toggle whether the two `hooks/` files are written.
fn setup(ships_hooks: bool, ships_guardrails: bool) -> Fixture {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let catalog_tmp = TempDir::new().unwrap();
    let catalog_root = catalog_tmp.path().join("catalog");
    let plugin_dir = catalog_root.join("plug");
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    std::fs::write(
        plugin_dir.join("tome-plugin.toml"),
        format!(
            "name = \"{}\"\nversion = \"1.0.0\"\n",
            plugin_dir.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
    fs::create_dir_all(plugin_dir.join("agents")).unwrap();
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
    if ships_hooks || ships_guardrails {
        fs::create_dir_all(plugin_dir.join("hooks")).unwrap();
    }
    if ships_hooks {
        fs::write(plugin_dir.join("hooks").join("hooks.json"), "{}").unwrap();
    }
    if ships_guardrails {
        fs::write(
            plugin_dir.join("hooks").join("GUARDRAILS.md"),
            "Be careful.\n",
        )
        .unwrap();
    }
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
fn agents_entry_field_set_and_ship_booleans_pinned() {
    let fx = setup(true, true);
    let v = show_json(&fx);

    // Envelope ship-presence booleans.
    assert_eq!(v["ships_hooks_json"], true);
    assert_eq!(v["ships_guardrails_md"], true);

    // The `agents` array carries one per-entry projection.
    let agents = v["agents"].as_array().expect("agents array");
    assert_eq!(agents.len(), 1, "one agent entry");
    let a = &agents[0];
    for field in [
        "name",
        "description",
        "when_to_use",
        "searchable",
        "user_invocable",
        "prompt_name",
        "arguments",
    ] {
        assert!(a.get(field).is_some(), "agent entry missing `{field}`: {a}");
    }
    assert_eq!(a["name"], "reviewer", "displayed name");
    // Agents are never searchable, never user-invocable, never a prompt.
    assert_eq!(a["searchable"], false);
    assert_eq!(a["user_invocable"], false);
    assert_eq!(a["prompt_name"], Value::Null);
    // Personas off (no global settings toggle) → persona_name skipped entirely.
    assert!(
        a.get("persona_name").is_none(),
        "persona_name absent (skip-if-none) when toggle off; got {a}",
    );
}

#[test]
fn ship_booleans_false_when_not_shipped() {
    let fx = setup(false, false);
    let v = show_json(&fx);
    assert_eq!(v["ships_hooks_json"], false);
    assert_eq!(v["ships_guardrails_md"], false);
}

#[test]
fn persona_name_present_and_derived_when_toggle_on() {
    let fx = setup(true, true);
    // Turn on personas at the global scope (the bare CLI resolves to global
    // fallback, whose resolver reads the global settings file).
    let paths = paths_for(&fx.env);
    fs::write(
        &paths.global_settings_file,
        "expose_agents_as_personas = true\n",
    )
    .expect("write global settings");

    let v = show_json(&fx);
    let a = &v["agents"][0];
    assert_eq!(
        a["persona_name"], "reviewer-persona",
        "persona_name present + derived `<name>-persona` when toggle on; got {a}",
    );
}

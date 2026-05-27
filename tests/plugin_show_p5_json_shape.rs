//! Phase 5 / US5.b — byte-stable JSON wire-shape pin for
//! `tome plugin show --json` per
//! `contracts/catalog-and-plugin-extensions-p5.md` § `tome plugin show`
//! § JSON-mode output.
//!
//! The pin asserts on field presence + values for each per-entry
//! projection; the full envelope contains many existing fields (`id`,
//! `version`, etc.) that already have their own tests in
//! `plugin_show.rs`. This file pins the Phase 5 ADDITIONS:
//! - `skills` array of per-entry projections.
//! - `commands` array of per-entry projections.
//! - Each entry: `name`, `description`, `when_to_use`, `searchable`,
//!   `user_invocable`, `prompt_name`, `arguments`, optional
//!   `argument_hint`.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

use common::{
    ToolEnv, config_with_catalog, fabricate_all_registry_models, global_scope, paths_for,
    stub_embedder_seed, stub_reranker_seed, stub_summariser_seed, write_config_for_cli,
};

fn write_plugin_with(
    catalog_root: &Path,
    plugin_name: &str,
    skills: &[(&str, &str)],
    commands: &[(&str, &str)],
) -> PathBuf {
    let plugin_dir = catalog_root.join(plugin_name);
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        format!(r#"{{"name": "{plugin_name}", "version": "1.0.0"}}"#),
    )
    .unwrap();
    for (name, body) in skills {
        let dir = plugin_dir.join("skills").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
    }
    if !commands.is_empty() {
        let cmd_dir = plugin_dir.join("commands");
        fs::create_dir_all(&cmd_dir).unwrap();
        for (name, body) in commands {
            fs::write(cmd_dir.join(format!("{name}.md")), body).unwrap();
        }
    }
    fs::write(
        catalog_root.join("tome-catalog.toml"),
        format!("[[plugins]]\nname = \"{plugin_name}\"\nsource = \"{plugin_name}\"\n"),
    )
    .unwrap();
    plugin_dir
}

#[test]
fn plugin_show_json_pins_per_entry_field_set() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let catalog_tmp = TempDir::new().unwrap();
    let catalog_root = catalog_tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();

    let skill_body = "---
name: pinned-skill
description: pinned skill description
---
body
";
    let command_body = "---
name: pinned-command
description: pinned command description
arguments: [one, two]
argument-hint: \"<one> <two>\"
---
body
";
    write_plugin_with(
        &catalog_root,
        "pinned",
        &[("pinned-skill", skill_body)],
        &[("pinned-command", command_body)],
    );

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
    let id: PluginId = "acme/pinned".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("enable pinned plugin");

    let out = env
        .cmd()
        .args(["plugin", "show", "acme/pinned", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("parse JSON");

    // Top-level Phase 5 additions: skills + commands arrays.
    let skills = v["skills"].as_array().expect("skills array");
    assert_eq!(skills.len(), 1, "expected one skill entry");
    let s = &skills[0];
    for field in [
        "name",
        "description",
        "when_to_use",
        "searchable",
        "user_invocable",
        "prompt_name",
        "arguments",
    ] {
        assert!(
            s.get(field).is_some(),
            "skill entry missing field `{field}`: {s}"
        );
    }
    assert_eq!(s["name"], "pinned-skill");
    assert_eq!(s["description"], "pinned skill description");
    assert_eq!(s["when_to_use"], Value::Null);
    assert_eq!(s["searchable"], true);
    assert_eq!(s["user_invocable"], false);
    assert_eq!(s["prompt_name"], Value::Null);
    assert_eq!(s["arguments"], serde_json::json!([]));

    let commands = v["commands"].as_array().expect("commands array");
    assert_eq!(commands.len(), 1, "expected one command entry");
    let c = &commands[0];
    for field in [
        "name",
        "description",
        "when_to_use",
        "searchable",
        "user_invocable",
        "prompt_name",
        "arguments",
    ] {
        assert!(
            c.get(field).is_some(),
            "command entry missing field `{field}`: {c}"
        );
    }
    assert_eq!(c["name"], "pinned-command");
    assert_eq!(c["description"], "pinned command description");
    assert_eq!(c["when_to_use"], Value::Null);
    assert_eq!(c["searchable"], true);
    assert_eq!(c["user_invocable"], true);
    let derived_name = c["prompt_name"].as_str().expect("derived prompt_name");
    assert!(
        derived_name.contains("pinned") && derived_name.contains("__"),
        "prompt_name `{derived_name}` should be `<plugin>__<entry>` form",
    );
    let args = c["arguments"].as_array().expect("arguments array");
    let names: Vec<&str> = args.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(names, vec!["one", "two"]);
    // argument_hint present when set in frontmatter.
    assert_eq!(c["argument_hint"], "<one> <two>");
}

#[test]
fn plugin_show_json_omits_argument_hint_when_absent() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let catalog_tmp = TempDir::new().unwrap();
    let catalog_root = catalog_tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();

    let body = "---
name: no-hint
description: command without an argument-hint
---
body
";
    write_plugin_with(&catalog_root, "noh", &[], &[("no-hint", body)]);

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
    let id: PluginId = "acme/noh".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("enable noh plugin");

    let out = env
        .cmd()
        .args(["plugin", "show", "acme/noh", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let c = &v["commands"][0];
    // `argument_hint` is `skip_serializing_if = "Option::is_none"`.
    assert!(
        c.get("argument_hint").is_none(),
        "argument_hint must be absent when None; got: {c}",
    );
}

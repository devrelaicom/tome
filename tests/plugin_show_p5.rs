//! Phase 5 / US5.b — `tome plugin show` per-entry annotations.
//!
//! Exercises the post-Phase-5 output shape per
//! `contracts/catalog-and-plugin-extensions-p5.md` § `tome plugin show`:
//! Skills + Commands sections, per-entry `searchable=` /
//! `user_invocable=` / `prompt=` annotations, and the `[dormant]`
//! marker.
//!
//! All tests construct a hand-rolled plugin fixture in a TempDir so the
//! entry frontmatter can carry the Phase 5 fields (`user_invocable`,
//! `prompt_name`, `argument-hint`, `arguments`) needed to exercise the
//! annotation matrix. `lifecycle::enable` runs against the
//! `StubEmbedder` so no ONNX model is loaded.

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

// ---- fixture construction -------------------------------------------------

/// Write `plugin.json` + the supplied `skills` and `commands` under
/// `<catalog_root>/<plugin_name>/`. Both lists are `(file_name,
/// frontmatter_body)`. Skills go under `skills/<name>/SKILL.md`,
/// commands under `commands/<name>.md`.
fn write_plugin(
    catalog_root: &Path,
    plugin_name: &str,
    skills: &[(&str, &str)],
    commands: &[(&str, &str)],
) -> PathBuf {
    let plugin_dir = catalog_root.join(plugin_name);
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    std::fs::write(
        plugin_dir.join("tome-plugin.toml"),
        format!(
            "name = \"{}\"\nversion = \"1.0.0\"\n",
            plugin_dir.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
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

    plugin_dir
}

/// Write `tome-catalog.toml` referencing one plugin.
fn write_catalog_manifest(catalog_root: &Path, plugin_name: &str) {
    fs::write(
        catalog_root.join("tome-catalog.toml"),
        format!(
            "\
[[plugins]]
name = \"{plugin_name}\"
source = \"{plugin_name}\"
"
        ),
    )
    .unwrap();
}

/// Bootstrap one ToolEnv with a hand-rolled catalog + plugin enabled.
/// Returns the env (keep alive for the test) and the catalog TempDir
/// (also keep alive — the on-disk plugin tree is rooted under it).
struct Fixture {
    env: ToolEnv,
    _catalog_tmp: TempDir,
}

fn setup_with_entries(
    plugin_name: &str,
    skills: &[(&str, &str)],
    commands: &[(&str, &str)],
) -> Fixture {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let catalog_tmp = TempDir::new().unwrap();
    let catalog_root = catalog_tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();
    write_plugin(&catalog_root, plugin_name, skills, commands);
    write_catalog_manifest(&catalog_root, plugin_name);

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
    let id: PluginId = format!("acme/{plugin_name}").parse().unwrap();
    lifecycle::enable(&id, &deps).expect("enable hand-rolled plugin");

    Fixture {
        env,
        _catalog_tmp: catalog_tmp,
    }
}

/// Run `tome plugin show --json <id>` and parse stdout.
fn show_json(fx: &Fixture, plugin_name: &str) -> Value {
    let out = fx
        .env
        .cmd()
        .args(["plugin", "show", &format!("acme/{plugin_name}"), "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    serde_json::from_slice(&out.stdout).expect("plugin show --json must emit valid JSON")
}

// ---- tests ----------------------------------------------------------------

const SKILL_DEFAULT: &str = "---
name: s1
description: A regular skill, searchable but not user-invocable.
---
body
";

const SKILL_DORMANT: &str = "---
name: dormant-thing
description: Internal scaffolding referenced by other entries.
disable-model-invocation: true
user_invocable: false
---
body
";

const COMMAND_DEFAULT: &str = "---
name: fix-issue
description: Fix a GitHub issue
---
do the thing
";

const COMMAND_WITH_ARGS: &str = "---
name: migrate-component
description: Migrate a component from one framework to another
arguments: [component, from, to]
---
migrate body
";

const COMMAND_WITH_OVERRIDE: &str = "---
name: rename-me
description: Command with explicit prompt_name override
prompt_name: my_renamed_prompt
---
override body
";

#[test]
fn skills_and_commands_render_in_separate_sections() {
    let fx = setup_with_entries(
        "plug",
        &[("s1", SKILL_DEFAULT)],
        &[("fix-issue", COMMAND_DEFAULT)],
    );

    let record = show_json(&fx, "plug");

    let skills = record["skills"].as_array().expect("skills array");
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0]["name"], "s1");

    let commands = record["commands"].as_array().expect("commands array");
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0]["name"], "fix-issue");
}

#[test]
fn skill_default_flags_in_json() {
    let fx = setup_with_entries("plug", &[("s1", SKILL_DEFAULT)], &[]);
    let record = show_json(&fx, "plug");
    let s = &record["skills"][0];
    // Skills default: searchable=true, user_invocable=false.
    assert_eq!(s["searchable"], true);
    assert_eq!(s["user_invocable"], false);
    // user_invocable=false → no derived prompt_name.
    assert_eq!(s["prompt_name"], Value::Null);
    // T-G2 (US5.c): contract specifies both arrays always present in
    // JSON output. A skills-only plugin must still ship an empty
    // `commands` array — never omit + never null.
    let commands = record["commands"]
        .as_array()
        .expect("commands array must be present even when empty");
    assert!(
        commands.is_empty(),
        "skills-only plugin must have empty commands array; got {commands:?}",
    );
}

#[test]
fn command_default_flags_and_derived_prompt_name() {
    let fx = setup_with_entries("plug", &[], &[("fix-issue", COMMAND_DEFAULT)]);
    let record = show_json(&fx, "plug");
    let c = &record["commands"][0];
    // Commands default: searchable=true, user_invocable=true.
    assert_eq!(c["searchable"], true);
    assert_eq!(c["user_invocable"], true);
    // prompt_name is derived: `<plugin>__<entry>`, sanitised.
    let derived_name = c["prompt_name"].as_str().expect("derived prompt_name");
    assert!(
        derived_name.contains("plug"),
        "prompt_name `{derived_name}` should include plugin name"
    );
    assert!(
        derived_name.contains("fix"),
        "prompt_name `{derived_name}` should include entry name"
    );
    // T-G2 (US5.c): contract specifies both arrays always present in
    // JSON output. A commands-only plugin must still ship an empty
    // `skills` array.
    let skills = record["skills"]
        .as_array()
        .expect("skills array must be present even when empty");
    assert!(
        skills.is_empty(),
        "commands-only plugin must have empty skills array; got {skills:?}",
    );
}

#[test]
fn dormant_not_annotated_when_searchable_true() {
    // T-G1 (US5.c): negative case for the [dormant] annotation.
    // SKILL_DEFAULT resolves to searchable=true, user_invocable=false;
    // [dormant] requires BOTH flags false, so this entry must not be
    // annotated. Companion to dormant_entry_annotated which exercises
    // the positive case.
    let fx = setup_with_entries("plug", &[("s1", SKILL_DEFAULT)], &[]);
    let out = fx
        .env
        .cmd()
        .args(["plugin", "show", "acme/plug"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("[dormant]"),
        "skill with searchable=true must NOT be annotated [dormant]; got:\n{stdout}",
    );
}

#[test]
fn dormant_entry_annotated() {
    let fx = setup_with_entries("plug", &[("dormant-thing", SKILL_DORMANT)], &[]);

    // Human-mode output: the `[dormant]` annotation is rendered when
    // both flags resolve to false.
    let out = fx
        .env
        .cmd()
        .args(["plugin", "show", "acme/plug"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("[dormant]"),
        "dormant skill must be annotated [dormant]; got:\n{stdout}",
    );

    // JSON-mode: both flags false.
    let record = show_json(&fx, "plug");
    let s = &record["skills"][0];
    assert_eq!(s["searchable"], false);
    assert_eq!(s["user_invocable"], false);
}

#[test]
fn override_prompt_name_surfaces_in_json() {
    let fx = setup_with_entries("plug", &[], &[("rename-me", COMMAND_WITH_OVERRIDE)]);
    let record = show_json(&fx, "plug");
    let c = &record["commands"][0];
    // user_invocable=true → derived name uses the override.
    assert_eq!(c["user_invocable"], true);
    let derived_name = c["prompt_name"].as_str().expect("override prompt_name");
    // The contract's prompt_name override is sanitised but otherwise
    // preserved; the original `my_renamed_prompt` survives the
    // sanitiser unchanged (lowercase + alphanumerics/underscores only).
    assert_eq!(derived_name, "my_renamed_prompt");
}

#[test]
fn declared_arguments_surface_in_json() {
    let fx = setup_with_entries("plug", &[], &[("migrate-component", COMMAND_WITH_ARGS)]);
    let record = show_json(&fx, "plug");
    let c = &record["commands"][0];
    let args = c["arguments"].as_array().expect("arguments array");
    let names: Vec<&str> = args.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(names, vec!["component", "from", "to"]);
}

#[test]
fn human_mode_shows_per_kind_section_headers() {
    let fx = setup_with_entries(
        "plug",
        &[("s1", SKILL_DEFAULT)],
        &[("fix-issue", COMMAND_DEFAULT)],
    );
    let out = fx
        .env
        .cmd()
        .args(["plugin", "show", "acme/plug"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Skills ("),
        "expected `Skills (N):` section header in:\n{stdout}",
    );
    assert!(
        stdout.contains("Commands ("),
        "expected `Commands (N):` section header in:\n{stdout}",
    );
    // searchable= / user_invocable= rendered on the per-entry line.
    assert!(
        stdout.contains("searchable=") && stdout.contains("user_invocable="),
        "per-entry flag annotations missing from:\n{stdout}",
    );
}

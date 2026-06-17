//! `tome tier {set,list,clear}` integration tests.
//!
//! Setup goes through the library API + StubEmbedder so we don't load ONNX
//! models in CI: `lifecycle::enable` stamps the central DB (registry-seeded by
//! `write_config_for_cli`, so the spawned binary's registry-seed opens match)
//! with the sample plugin's skills enrolled in the `global` workspace. The
//! `tier` subcommands are then driven via the compiled binary — they only open
//! the index (`index::open`/`open_read_only`), never the embedder, so they are
//! cheap to spawn.

mod common;

use common::{
    Fixture, ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_models, paths_for,
    stub_embedder_seed, stub_reranker_seed, stub_summariser_seed, write_config_for_cli,
};
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

/// Enable `sample-plugin-catalog/plugin-alpha` in the `global` workspace via the
/// library API (StubEmbedder). `_fixture` keeps the git fixture alive only for
/// parity with sibling suites — resolution reads the symlinked clone dir.
fn setup_enabled(env: &ToolEnv, tmp: &TempDir) {
    let paths = paths_for(env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);

    let embedder = StubEmbedder::new();
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    lifecycle::enable(&id, &deps).expect("enable plugin-alpha");
}

/// Parse `tome tier list --json` NDJSON stdout into a vec of JSON objects.
fn tier_list_json(env: &ToolEnv) -> Vec<serde_json::Value> {
    let out = env
        .cmd()
        .args(["--json", "tier", "list"])
        .output()
        .expect("spawn tome tier list");
    assert!(
        out.status.success(),
        "tier list exit {:?}; stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("each stdout line is a JSON object"))
        .collect()
}

#[test]
fn tier_set_then_list_roundtrip() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled(&env, &tmp);

    // Default tier is 3 for every entry.
    let before = tier_list_json(&env);
    let skill_a = before
        .iter()
        .find(|e| e["plugin"] == "plugin-alpha" && e["name"] == "skill-a")
        .expect("skill-a listed");
    assert_eq!(skill_a["tier"], 3, "default tier is 3");

    // Set it to tier 1.
    let set = env
        .cmd()
        .args(["tier", "set", "plugin-alpha/skill-a", "1"])
        .output()
        .expect("spawn tier set");
    assert!(
        set.status.success(),
        "tier set exit {:?}; stderr={}",
        set.status.code(),
        String::from_utf8_lossy(&set.stderr),
    );

    // List now shows skill-a at tier 1.
    let after = tier_list_json(&env);
    let skill_a = after
        .iter()
        .find(|e| e["plugin"] == "plugin-alpha" && e["name"] == "skill-a")
        .expect("skill-a still listed");
    assert_eq!(skill_a["tier"], 1, "skill-a retiered to 1");
    assert_eq!(skill_a["kind"], "skill");
    assert_eq!(skill_a["catalog"], "sample-plugin-catalog");
}

#[test]
fn tier_set_unknown_entry_exits_27() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled(&env, &tmp);

    let out = env
        .cmd()
        .args(["tier", "set", "plugin-alpha/ghost", "1"])
        .output()
        .expect("spawn tier set ghost");
    assert_eq!(
        out.status.code(),
        Some(27),
        "unknown entry → EntryNotFound (27); stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn tier_clear_resets_to_3() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled(&env, &tmp);

    // Bump to tier 2 first.
    let set = env
        .cmd()
        .args(["tier", "set", "plugin-alpha/skill-b", "2"])
        .output()
        .expect("spawn tier set");
    assert!(set.status.success());
    let after_set = tier_list_json(&env);
    assert_eq!(
        after_set
            .iter()
            .find(|e| e["name"] == "skill-b")
            .expect("skill-b")["tier"],
        2,
    );

    // Clear → back to 3.
    let clear = env
        .cmd()
        .args(["tier", "clear", "plugin-alpha/skill-b"])
        .output()
        .expect("spawn tier clear");
    assert!(
        clear.status.success(),
        "tier clear exit {:?}; stderr={}",
        clear.status.code(),
        String::from_utf8_lossy(&clear.stderr),
    );
    let after_clear = tier_list_json(&env);
    assert_eq!(
        after_clear
            .iter()
            .find(|e| e["name"] == "skill-b")
            .expect("skill-b")["tier"],
        3,
        "clear reset skill-b to the default tier 3",
    );
}

#[test]
fn tier_set_rejects_out_of_range() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled(&env, &tmp);

    // Tier 4 is outside the clap value_parser range 1..=3 → usage error (2).
    let out = env
        .cmd()
        .args(["tier", "set", "plugin-alpha/skill-a", "4"])
        .output()
        .expect("spawn tier set 4");
    assert_eq!(
        out.status.code(),
        Some(2),
        "out-of-range tier → usage exit 2"
    );
}

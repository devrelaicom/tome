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
use tome::index::skills::set_tier_for_plugin;
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

/// `tome harness session-start` regenerates the routing directive fresh from
/// live state. With a tier-1 skill present, the directive must carry the
/// "Load now (Tier 1)" section and a `get_skill(` call for it. The command
/// prints plain text to stdout (the Claude Code SessionStart hook target).
#[test]
fn harness_session_start_prints_directive() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled(&env, &tmp);

    // Promote skill-a to tier 1 so the directive has a "Load now (Tier 1)"
    // section with a get_skill call.
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

    let out = env
        .cmd()
        .args(["harness", "session-start"])
        .output()
        .expect("spawn harness session-start");
    assert!(
        out.status.success(),
        "session-start exit {:?}; stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let directive = String::from_utf8_lossy(&out.stdout);
    assert!(
        directive.contains("## Load now (Tier 1)"),
        "directive must carry the Tier 1 section; got:\n{directive}"
    );
    assert!(
        directive.contains("get_skill("),
        "directive must carry a get_skill call for the tier-1 skill; got:\n{directive}"
    );
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

/// Verify that `set_tier_for_plugin` (the function called by `plugin enable
/// --tier`) bulk-sets ALL skills/commands for a plugin and that the result is
/// visible via `tome tier list --json`.
///
/// This test exercises the underlying DB helper that `enable --tier` invokes.
/// The command-layer flag wiring is covered by
/// `plugin_enable_with_tier_flag_binary` (marked `#[ignore]` because the
/// `plugin enable` binary path loads FastembedEmbedder/ONNX models).
#[test]
fn plugin_enable_with_tier_flag_bulk_sets() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();

    // Enable the plugin in-process via the stub embedder (no ONNX load).
    setup_enabled(&env, &tmp);

    // Default tier for all entries is 3.
    let before = tier_list_json(&env);
    for entry in &before {
        assert_eq!(
            entry["tier"], 3,
            "all entries start at tier 3, but {:?} has tier {}",
            entry["name"], entry["tier"]
        );
    }
    assert!(
        !before.is_empty(),
        "plugin-alpha must have at least one tierable entry after enable"
    );

    // Simulate what `plugin enable --tier 1` does: call set_tier_for_plugin
    // under a writable connection + advisory lock, exactly as enable.rs does.
    {
        let paths = paths_for(&env);
        let (e, r, s) = (
            stub_embedder_seed(),
            stub_reranker_seed(),
            stub_summariser_seed(),
        );
        let conn = tome::index::open(
            &paths.index_db,
            &tome::index::OpenOptions {
                embedder: e,
                reranker: r,
                summariser: s,
                profile: None,
            },
        )
        .expect("open writable index");
        let lock = tome::index::acquire_lock(&paths.index_lock).expect("acquire lock");
        let affected =
            set_tier_for_plugin(&conn, "global", "sample-plugin-catalog", "plugin-alpha", 1)
                .expect("set_tier_for_plugin");
        lock.release().expect("release lock");

        assert!(
            affected > 0,
            "set_tier_for_plugin must update at least one row; got 0 affected"
        );
    }

    // Every skill/command for plugin-alpha must now show tier 1.
    let after = tier_list_json(&env);
    assert_eq!(
        after.len(),
        before.len(),
        "entry count must be unchanged after tier update"
    );
    for entry in &after {
        assert_eq!(
            entry["tier"], 1,
            "after bulk-set all entries must be tier 1, but {:?} shows tier {}",
            entry["name"], entry["tier"]
        );
    }
}

/// Drive `tome plugin enable --tier 1` via the compiled binary.
///
/// # Why `#[ignore]`
///
/// The `plugin enable` command path unconditionally calls
/// `FastembedEmbedder::load`, which requires the real ONNX model files to be
/// present on disk (not just the `manifest.toml` stubs used by the cheap
/// integration tests). Running this test without the models results in a
/// `ModelEmbedInitFailed` error. This test is therefore gated behind the
/// real-model release gate (`cargo test --ignored`), exactly like
/// `model_download_complete`, `reranker_cpu_inference`, and
/// `search_knn_recall_realmodel`.
///
/// The underlying behaviour (DB tier update + visibility via `tier list`) is
/// covered without models by `plugin_enable_with_tier_flag_bulk_sets` above.
#[test]
#[ignore = "requires real ONNX model files on disk; run with `cargo test --ignored`"]
fn plugin_enable_with_tier_flag_binary() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();

    // For this binary-driven path we need the catalog enrolled and models
    // actually installed (the binary checks model presence before the embedder
    // loads). setup_enabled uses the stub embedder; we re-stage the catalog so
    // the binary can resolve it, then let `plugin enable --yes --tier 1` do
    // everything.
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);
    // Real model files must be present; if they aren't the binary exits with
    // ModelMissing (exit 30) and the test assertion below will surface a
    // descriptive failure.

    let out = env
        .cmd()
        .args([
            "--json",
            "plugin",
            "enable",
            "--yes",
            "--tier",
            "1",
            "sample-plugin-catalog/plugin-alpha",
        ])
        .output()
        .expect("spawn plugin enable");

    assert!(
        out.status.success(),
        "plugin enable --tier 1 failed (exit {:?}); stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    // Every skill/command for plugin-alpha must show tier 1 immediately after.
    let after = tier_list_json(&env);
    assert!(
        !after.is_empty(),
        "tier list must be non-empty after enable with --tier"
    );
    for entry in &after {
        assert_eq!(
            entry["tier"], 1,
            "all entries must be tier 1 after `plugin enable --tier 1`, but {:?} shows {}",
            entry["name"], entry["tier"]
        );
    }
}

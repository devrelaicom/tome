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

/// `--kind agent` is INERT for the tier commands. `skill-a` is a real, existing
/// entry (see `tier_set_then_list_roundtrip`), but `tome tier set ... --kind
/// agent` filters against `tiered_entries_for_workspace`, which hard-filters to
/// `kind IN ('skill','command')` — so the agent-kind filter matches zero rows
/// and resolves to `EntryNotFound` (exit 27). This documents that the `Agent`
/// variant added to `TierKindArg` for `tome query --kind` never tiers anything.
#[test]
fn tier_set_kind_agent_is_inert_and_exits_27() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled(&env, &tmp);

    let out = env
        .cmd()
        .args([
            "tier",
            "set",
            "plugin-alpha/skill-a",
            "1",
            "--kind",
            "agent",
        ])
        .output()
        .expect("spawn tier set --kind agent");
    assert_eq!(
        out.status.code(),
        Some(27),
        "--kind agent must be inert for tier (skill/command only) → EntryNotFound (27); stderr={}",
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

// ---- issue #317: bulk retiering (--plugin, name-globs, --all) -------------

/// Enable BOTH plugin-alpha and plugin-beta in `global` via the library API
/// (StubEmbedder) so the `--plugin` fan-out and workspace-wide `--all` tests
/// operate over more than one plugin.
fn setup_enabled_both(env: &ToolEnv, tmp: &TempDir) {
    let paths = paths_for(env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);

    let embedder = StubEmbedder::new();
    for plugin in ["plugin-alpha", "plugin-beta"] {
        let id: PluginId = format!("sample-plugin-catalog/{plugin}").parse().unwrap();
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
        lifecycle::enable(&id, &deps).unwrap_or_else(|e| panic!("enable {plugin}: {e}"));
    }
}

/// The tier of a named entry, or `None` when it isn't listed.
fn tier_of(entries: &[serde_json::Value], name: &str) -> Option<i64> {
    entries
        .iter()
        .find(|e| e["name"] == name)
        .and_then(|e| e["tier"].as_i64())
}

/// A single literal `<plugin>/<name>` id remains BYTE-IDENTICAL: `--json` emits
/// exactly one object with the pinned field order. This is the back-compat
/// guarantee for the pre-#317 shape (mirrors `tier_record_json_shape_is_pinned`
/// end-to-end through the binary).
#[test]
fn tier_set_single_literal_id_json_is_one_pinned_object() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled(&env, &tmp);

    let out = env
        .cmd()
        .args(["--json", "tier", "set", "plugin-alpha/skill-a", "2"])
        .output()
        .expect("spawn tier set --json");
    assert!(
        out.status.success(),
        "tier set exit {:?}; stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "exactly one JSON object; got: {stdout}");
    assert_eq!(
        lines[0],
        r#"{"catalog":"sample-plugin-catalog","plugin":"plugin-alpha","name":"skill-a","kind":"skill","tier":2}"#,
        "single-id JSON must be the byte-identical pinned shape",
    );
}

/// A `<plugin>/*` name-glob retiers EVERY enabled entry of the plugin.
#[test]
fn tier_set_name_glob_retiers_every_entry() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled(&env, &tmp);

    let before = tier_list_json(&env);
    assert!(before.len() >= 2, "plugin-alpha has multiple entries");
    assert!(
        before.iter().all(|e| e["tier"] == 3),
        "all entries start at default tier 3"
    );

    let out = env
        .cmd()
        .args(["--json", "tier", "set", "plugin-alpha/*", "2"])
        .output()
        .expect("spawn tier set glob");
    assert!(
        out.status.success(),
        "tier set glob exit {:?}; stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    // One NDJSON record per affected entry.
    let records = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();
    assert_eq!(records, before.len(), "one record per retiered entry");

    let after = tier_list_json(&env);
    assert!(
        after.iter().all(|e| e["tier"] == 2),
        "every entry now tier 2: {after:?}"
    );
}

/// A `<plugin>/foo-*` name-glob retiers only the matching SUBSET; non-matching
/// siblings keep their tier.
#[test]
fn tier_set_name_glob_subset_leaves_others() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled(&env, &tmp);

    // `skill-*` matches every plugin-alpha entry (all named skill-…), so use a
    // narrower glob that excludes at least one: `skill-a*` matches only skill-a.
    let out = env
        .cmd()
        .args(["tier", "set", "plugin-alpha/skill-a*", "1"])
        .output()
        .expect("spawn tier set subset glob");
    assert!(
        out.status.success(),
        "tier set subset exit {:?}; stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let after = tier_list_json(&env);
    assert_eq!(tier_of(&after, "skill-a"), Some(1), "skill-a retiered");
    // A sibling that does NOT match `skill-a*` stays at the default.
    assert_eq!(
        tier_of(&after, "skill-c"),
        Some(3),
        "non-matching sibling skill-c unchanged"
    );
}

/// A name-glob that matches nothing is `entry_not_found` (exit 27), never a
/// silent success.
#[test]
fn tier_set_name_glob_zero_match_exits_27() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled(&env, &tmp);

    let out = env
        .cmd()
        .args(["tier", "set", "plugin-alpha/nope-*", "1"])
        .output()
        .expect("spawn tier set zero-match glob");
    assert_eq!(
        out.status.code(),
        Some(27),
        "zero-match glob → EntryNotFound (27); stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// `--plugin <catalog>/<plugin>` fans out over every enabled entry of the named
/// plugin, leaving OTHER plugins untouched.
#[test]
fn tier_set_plugin_selector_fans_out() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled_both(&env, &tmp);

    let out = env
        .cmd()
        .args([
            "tier",
            "set",
            "--plugin",
            "sample-plugin-catalog/plugin-alpha",
            "2",
        ])
        .output()
        .expect("spawn tier set --plugin");
    assert!(
        out.status.success(),
        "tier set --plugin exit {:?}; stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let after = tier_list_json(&env);
    // Every plugin-alpha entry is tier 2; plugin-beta's skill-x is untouched (3).
    for e in &after {
        if e["plugin"] == "plugin-alpha" {
            assert_eq!(e["tier"], 2, "plugin-alpha entry retiered: {e:?}");
        }
    }
    assert_eq!(
        tier_of(&after, "skill-x"),
        Some(3),
        "plugin-beta/skill-x untouched by --plugin plugin-alpha"
    );
}

/// A bare `--plugin <plugin>` (no catalog) resolves via the enabled-plugin
/// candidate set when the name is unique across enrolled catalogs.
#[test]
fn tier_set_plugin_bare_unique_catalog() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled_both(&env, &tmp);

    let out = env
        .cmd()
        .args(["tier", "set", "--plugin", "plugin-beta", "1"])
        .output()
        .expect("spawn tier set --plugin bare");
    assert!(
        out.status.success(),
        "bare --plugin exit {:?}; stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let after = tier_list_json(&env);
    assert_eq!(
        tier_of(&after, "skill-x"),
        Some(1),
        "plugin-beta/skill-x retiered via bare --plugin"
    );
    // plugin-alpha entries stay at the default.
    assert_eq!(
        tier_of(&after, "skill-a"),
        Some(3),
        "plugin-alpha untouched"
    );
}

/// `--plugin` naming a plugin with no tierable entries is `entry_not_found`
/// (exit 27). `ghost` is not an enabled plugin.
#[test]
fn tier_set_plugin_no_entries_exits_27() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled(&env, &tmp);

    let out = env
        .cmd()
        .args([
            "tier",
            "set",
            "--plugin",
            "sample-plugin-catalog/ghost",
            "1",
        ])
        .output()
        .expect("spawn tier set --plugin ghost");
    // A slash-qualified `--plugin` that isn't an enabled plugin resolves to a
    // valid PluginId (selector defers existence downstream), then collects zero
    // tierable entries → EntryNotFound (27).
    assert_eq!(
        out.status.code(),
        Some(27),
        "--plugin with no tierable entries → EntryNotFound (27); stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// `tier clear --all` resets EVERY enabled tierable entry in the workspace back
/// to the default (3), across multiple plugins.
#[test]
fn tier_clear_all_resets_workspace() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled_both(&env, &tmp);

    // Bump a couple of entries off the default first.
    for id in ["plugin-alpha/skill-a", "plugin-beta/skill-x"] {
        let set = env
            .cmd()
            .args(["tier", "set", id, "1"])
            .output()
            .expect("spawn tier set");
        assert!(set.status.success(), "tier set {id}");
    }
    let bumped = tier_list_json(&env);
    assert_eq!(tier_of(&bumped, "skill-a"), Some(1));
    assert_eq!(tier_of(&bumped, "skill-x"), Some(1));

    // Reset the whole workspace.
    let clear = env
        .cmd()
        .args(["tier", "clear", "--all"])
        .output()
        .expect("spawn tier clear --all");
    assert!(
        clear.status.success(),
        "tier clear --all exit {:?}; stderr={}",
        clear.status.code(),
        String::from_utf8_lossy(&clear.stderr),
    );

    let after = tier_list_json(&env);
    assert!(
        after.iter().all(|e| e["tier"] == 3),
        "every entry back to default tier 3: {after:?}"
    );
}

/// `tier clear --plugin <sel>` resets a whole plugin, leaving others untouched.
#[test]
fn tier_clear_plugin_selector_resets_one_plugin() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled_both(&env, &tmp);

    // Bump both plugins to tier 1.
    for sel in ["plugin-alpha", "plugin-beta"] {
        let set = env
            .cmd()
            .args(["tier", "set", "--plugin", sel, "1"])
            .output()
            .expect("spawn tier set --plugin");
        assert!(set.status.success(), "tier set --plugin {sel}");
    }

    // Clear only plugin-alpha.
    let clear = env
        .cmd()
        .args(["tier", "clear", "--plugin", "plugin-alpha"])
        .output()
        .expect("spawn tier clear --plugin");
    assert!(
        clear.status.success(),
        "tier clear --plugin exit {:?}; stderr={}",
        clear.status.code(),
        String::from_utf8_lossy(&clear.stderr),
    );

    let after = tier_list_json(&env);
    assert_eq!(
        tier_of(&after, "skill-a"),
        Some(3),
        "plugin-alpha reset to default"
    );
    assert_eq!(
        tier_of(&after, "skill-x"),
        Some(1),
        "plugin-beta still tier 1 (untouched)"
    );
}

/// CLI XOR parse errors are exit 2, independent of any DB state: neither
/// selection source, or more than one, is a usage error.
#[test]
fn tier_selection_xor_is_usage_error() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled(&env, &tmp);

    // set: neither id nor --plugin (only a tier).
    let neither = env
        .cmd()
        .args(["tier", "set", "2"])
        .output()
        .expect("spawn tier set neither");
    assert_eq!(
        neither.status.code(),
        Some(2),
        "set with no selection source → usage 2; stderr={}",
        String::from_utf8_lossy(&neither.stderr),
    );

    // set: both id and --plugin.
    let both = env
        .cmd()
        .args([
            "tier",
            "set",
            "plugin-alpha/skill-a",
            "2",
            "--plugin",
            "plugin-alpha",
        ])
        .output()
        .expect("spawn tier set both");
    assert_eq!(
        both.status.code(),
        Some(2),
        "set with both id and --plugin → usage 2; stderr={}",
        String::from_utf8_lossy(&both.stderr),
    );

    // clear: neither id, --plugin, nor --all.
    let clear_neither = env
        .cmd()
        .args(["tier", "clear"])
        .output()
        .expect("spawn tier clear neither");
    assert_eq!(
        clear_neither.status.code(),
        Some(2),
        "clear with no selection source → usage 2",
    );

    // clear: both id and --all.
    let clear_both = env
        .cmd()
        .args(["tier", "clear", "plugin-alpha/skill-a", "--all"])
        .output()
        .expect("spawn tier clear both");
    assert_eq!(
        clear_both.status.code(),
        Some(2),
        "clear with both id and --all → usage 2",
    );
}

/// Forward-progress across a bulk `--plugin` batch where every entry is a benign
/// success: all entries are applied and one record emitted per entry.
#[test]
fn tier_set_plugin_batch_all_succeed() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled_both(&env, &tmp);

    // Two --plugin selectors in one batch → union of both plugins' entries.
    let out = env
        .cmd()
        .args([
            "--json",
            "tier",
            "set",
            "--plugin",
            "plugin-alpha",
            "--plugin",
            "plugin-beta",
            "2",
        ])
        .output()
        .expect("spawn tier set two --plugin");
    assert!(
        out.status.success(),
        "batch --plugin exit {:?}; stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let records = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count();

    let after = tier_list_json(&env);
    assert_eq!(
        records,
        after.len(),
        "one record per entry across both plugins"
    );
    assert!(
        after.iter().all(|e| e["tier"] == 2),
        "every entry across both plugins retiered to 2: {after:?}"
    );
}

/// FAIL-CLOSED (deliberate, intentional-by-test): a bulk `--plugin` batch hard
/// fails on the FIRST selector error even when OTHER `--plugin` tokens matched —
/// the mutation target set must be unambiguous before any write. This diverges
/// from `plugin enable`'s forward-progress: `tier set` MUST NOT partially retier
/// the good plugin and then error, because that would leave the batch in a state
/// the surfaced error contradicts. This test locks that as intentional.
#[test]
fn tier_set_plugin_batch_fails_closed_on_selector_error() {
    let _fixture = Fixture::build_sample();
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    setup_enabled_both(&env, &tmp);

    // A good `--plugin` (plugin-alpha) alongside a `--plugin` glob that matches
    // nothing (`nonexistent-*`). The selector yields matched=[plugin-alpha] AND a
    // NoGlobMatch error → the whole batch aborts with Usage (exit 2) BEFORE any
    // tier write lands.
    let out = env
        .cmd()
        .args([
            "tier",
            "set",
            "--plugin",
            "plugin-alpha",
            "--plugin",
            "nonexistent-*",
            "2",
        ])
        .output()
        .expect("spawn tier set fail-closed batch");
    assert_eq!(
        out.status.code(),
        Some(2),
        "a zero-match --plugin glob in the batch → Usage (2), fail-closed; stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );

    // The good plugin's entries must be UNCHANGED — no partial write landed.
    let after = tier_list_json(&env);
    for e in &after {
        if e["plugin"] == "plugin-alpha" {
            assert_eq!(
                e["tier"], 3,
                "fail-closed: plugin-alpha must stay at the default tier, no partial write: {e:?}"
            );
        }
    }
}

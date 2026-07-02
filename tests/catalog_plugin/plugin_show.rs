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

use crate::common::{
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
        scope: &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
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
fn show_reads_native_manifest_ignoring_legacy_plugin_json() {
    // Post-cutover (US1) `plugin show` reads ONLY `tome-plugin.toml`. The
    // fixture's legacy `.claude-plugin/plugin.json` carries `keywords`,
    // `homepage`, and `unknown_extra_field` — strict-rejected by the native
    // schema, but never read by `show`, so they are simply irrelevant.
    // `show` succeeds against the clean native manifest, and the surfaced
    // `version` comes from `tome-plugin.toml` (US1 closeout TEST-M3: this
    // previously claimed to test "lenient plugin.json parsing", which the
    // cutover made tautological).
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
        "show must read the native manifest: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(
        v["version"], "1.2.3",
        "version must come from the native tome-plugin.toml, got {v}"
    );
}

#[test]
fn show_without_details_omits_tier_from_json_and_human() {
    // #330: the DEFAULT `plugin show` output — human AND json — must be
    // byte-identical to before the flag existed. `tier` is
    // `skip_serializing_if = "Option::is_none"` and stays `None` without
    // `--details`, so it must be ABSENT from the serialized entry JSON, and no
    // ` tier=` annotation appears on the human entry line.
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
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let skills = v["skills"].as_array().expect("skills array");
    assert!(!skills.is_empty(), "fixture has skills to inspect");
    for s in skills {
        assert!(
            s.get("tier").is_none(),
            "without --details the entry JSON must NOT carry `tier`: {s}",
        );
    }

    // Human mode: no ` tier=` annotation on any entry line.
    let human = env
        .cmd()
        .args(["plugin", "show", "sample-plugin-catalog/plugin-alpha"])
        .output()
        .unwrap();
    assert!(human.status.success());
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        !text.contains(" tier="),
        "without --details the human output must not annotate tiers: {text}",
    );
}

#[test]
fn show_details_annotates_each_entry_with_its_tier() {
    // #330: `--details` populates `tier` on each per-entry projection (json)
    // and appends ` tier=<n>` to the human entry line. Enabled entries default
    // to tier 3; promote skill-a to tier 1 to prove the value is per-entry,
    // not a constant.
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup(&env, &fixture_tmp);

    let set = env
        .cmd()
        .args(["tier", "set", "plugin-alpha/skill-a", "1"])
        .output()
        .unwrap();
    assert!(
        set.status.success(),
        "tier set failed: {}",
        String::from_utf8_lossy(&set.stderr),
    );

    // JSON: every enabled skill carries a `tier`; skill-a is 1, the rest are 3.
    let out = env
        .cmd()
        .args([
            "plugin",
            "show",
            "sample-plugin-catalog/plugin-alpha",
            "--details",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("json");
    let skills = v["skills"].as_array().expect("skills array");
    assert!(!skills.is_empty(), "fixture has skills");
    let skill_a = skills
        .iter()
        .find(|s| s["name"] == "skill-a")
        .expect("skill-a present");
    assert_eq!(
        skill_a["tier"], 1,
        "skill-a was promoted to tier 1: {skill_a}",
    );
    // Every enabled entry must carry a tier under --details.
    for s in skills {
        assert!(
            s.get("tier").is_some(),
            "--details must annotate every enabled entry with a tier: {s}",
        );
    }

    // Human: the ` tier=1` annotation appears on skill-a's line, and `tier=3`
    // on the default-tier lines.
    let human = env
        .cmd()
        .args([
            "plugin",
            "show",
            "sample-plugin-catalog/plugin-alpha",
            "--details",
        ])
        .output()
        .unwrap();
    assert!(human.status.success());
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("tier=1"),
        "the promoted skill's line must carry `tier=1`: {text}",
    );
    assert!(
        text.contains("tier=3"),
        "default-tier entries must carry `tier=3`: {text}",
    );
    // The annotation must be on the skill-a entry line specifically.
    let skill_a_line = text
        .lines()
        .find(|l| l.contains("skill-a "))
        .expect("skill-a line present");
    assert!(
        skill_a_line.contains("tier=1"),
        "skill-a's own line must carry tier=1: {skill_a_line}",
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

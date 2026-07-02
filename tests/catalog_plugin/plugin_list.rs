//! Integration tests for `tome plugin list` against the CLI binary.
//!
//! Unlike `plugin enable`, `plugin list` does not load embedders or
//! rerankers — it only reads the catalog manifest and the index DB. That
//! makes the CLI binary safe to drive directly: no model artefacts needed.
//!
//! Setup pattern:
//!   1. `ToolEnv` provides an isolated `HOME` + XDG layout.
//!   2. Copy the `sample-plugin-catalog` fixture into a TempDir.
//!   3. Write `config.toml` directly to the isolated config dir (bypasses
//!      `catalog add`, which would require the fixture to be a git repo).
//!   4. Pre-enable `plugin-alpha` via the library API so the index has at
//!      least one enabled plugin row.
//!   5. Invoke `tome plugin list --json`.
//!
//! Spec: `contracts/plugin-commands.md` §3.

use std::path::Path;

use crate::common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_models, paths_for,
    sample_plugin_catalog_fixture, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
    write_config_for_cli,
};
use serde_json::Value;
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::paths::Paths;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

/// End-to-end setup: register the sample-plugin-catalog under `catalog_name`
/// in the supplied env's config and pre-enable `plugin-alpha`.
fn setup_with_alpha_enabled(env: &ToolEnv, fixture_tmp: &TempDir, catalog_name: &str) -> Paths {
    let paths = paths_for(env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(fixture_tmp, "catalog");

    // Mirror what `tome catalog add` would record: catalog path = the
    // directory containing `tome-catalog.toml`. The fixture lays out plugin
    // directories as immediate children of the catalog root, so the same
    // `Config` works for both the lifecycle library API (`<path>/<plugin>`)
    // and the CLI list path (manifest walk → `<path>/<source>`).
    let cli_config = config_with_catalog(catalog_name, &catalog_root);
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
    let id: PluginId = format!("{catalog_name}/plugin-alpha").parse().unwrap();
    lifecycle::enable(&id, &deps).expect("pre-enable plugin-alpha");

    paths
}

/// Parse NDJSON stdout into a `Vec<Value>`.
fn parse_ndjson(stdout: &[u8]) -> Vec<Value> {
    stdout
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_slice::<Value>(l).expect("parse json line"))
        .collect()
}

#[test]
fn list_emits_both_plugins_with_correct_status() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["plugin", "list", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let records = parse_ndjson(&out.stdout);
    assert_eq!(
        records.len(),
        2,
        "expected two plugin rows, got: {}",
        String::from_utf8_lossy(&out.stdout),
    );

    let by_plugin: std::collections::HashMap<String, &Value> = records
        .iter()
        .map(|r| {
            let id = r["id"]
                .as_object()
                .and_then(|o| o.get("plugin"))
                .and_then(|v| v.as_str())
                .expect("plugin field")
                .to_owned();
            (id, r)
        })
        .collect();

    let alpha = by_plugin
        .get("plugin-alpha")
        .expect("plugin-alpha row missing");
    assert_eq!(alpha["status"], "enabled");
    assert_eq!(alpha["version"], "1.2.3");

    let beta = by_plugin
        .get("plugin-beta")
        .expect("plugin-beta row missing");
    assert_eq!(beta["status"], "disabled");
    assert_eq!(beta["version"], "0.9.0");
}

#[test]
fn list_enabled_only_hides_disabled_plugins() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["plugin", "list", "--enabled-only", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let records = parse_ndjson(&out.stdout);
    assert_eq!(records.len(), 1, "expected one enabled plugin row");
    assert_eq!(records[0]["status"], "enabled");
    assert_eq!(records[0]["id"]["plugin"], "plugin-alpha");
}

#[test]
fn list_filter_matches_name_case_insensitively() {
    // #330: `--filter` is a case-insensitive substring match against the
    // plugin name. Both fixture plugins are `plugin-alpha` / `plugin-beta`;
    // `ALPHA` (upper-cased) must match only alpha in BOTH human and json.
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["plugin", "list", "--filter", "ALPHA", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let records = parse_ndjson(&out.stdout);
    assert_eq!(records.len(), 1, "only plugin-alpha matches `ALPHA`");
    assert_eq!(records[0]["id"]["plugin"], "plugin-alpha");

    // Human mode must be narrowed identically: beta is absent, alpha present.
    let human = env
        .cmd()
        .args(["plugin", "list", "--filter", "ALPHA"])
        .output()
        .unwrap();
    assert!(human.status.success());
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains("plugin-alpha"),
        "alpha must be listed: {text}"
    );
    assert!(
        !text.contains("plugin-beta"),
        "beta must be filtered out of the human table: {text}",
    );
}

#[test]
fn list_filter_matches_description_case_insensitively() {
    // #330: `--filter` also searches the DESCRIPTION. Only plugin-beta's
    // description contains the word "show" ("... plugin list / show tests."),
    // so `SHOW` must select beta alone even though neither plugin NAME
    // contains it.
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["plugin", "list", "--filter", "SHOW", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let records = parse_ndjson(&out.stdout);
    assert_eq!(
        records.len(),
        1,
        "only plugin-beta's description mentions `show`, got: {}",
        String::from_utf8_lossy(&out.stdout),
    );
    assert_eq!(records[0]["id"]["plugin"], "plugin-beta");
    assert!(
        records[0]["description"]
            .as_str()
            .map(|d| d.to_lowercase().contains("show"))
            .unwrap_or(false),
        "matched row's description must contain the needle: {}",
        records[0]["description"],
    );
}

#[test]
fn list_filter_non_match_yields_no_rows() {
    // A needle present in neither name nor description filters everything out.
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["plugin", "list", "--filter", "zzz-nonexistent", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        out.stdout.is_empty(),
        "no plugin matches the needle, so the json stream must be empty: {:?}",
        String::from_utf8_lossy(&out.stdout),
    );
}

#[test]
fn list_tier_filter_includes_only_plugins_with_an_entry_at_that_tier() {
    // #330: `--tier N` keeps a plugin only if it has an enabled entry at tier
    // N. plugin-alpha is enabled (all entries default to tier 3); promote one
    // of its skills to tier 1. Then `--tier 1` must include alpha, `--tier 2`
    // must exclude it (no entry at tier 2).
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

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

    // --tier 1 → alpha included (skill-a is tier 1).
    let out = env
        .cmd()
        .args(["plugin", "list", "--tier", "1", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let records = parse_ndjson(&out.stdout);
    assert_eq!(records.len(), 1, "only plugin-alpha has a tier-1 entry");
    assert_eq!(records[0]["id"]["plugin"], "plugin-alpha");

    // --tier 2 → nothing (no entry at tier 2; beta is disabled entirely).
    let out2 = env
        .cmd()
        .args(["plugin", "list", "--tier", "2", "--json"])
        .output()
        .unwrap();
    assert!(out2.status.success());
    assert!(
        out2.stdout.is_empty(),
        "no plugin has a tier-2 entry: {:?}",
        String::from_utf8_lossy(&out2.stdout),
    );
}

#[test]
fn list_tier_filter_excludes_disabled_plugin() {
    // A disabled plugin (plugin-beta) has no `workspace_skills` rows, so it can
    // never satisfy any `--tier` filter — even tier 3, the default.
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["plugin", "list", "--tier", "3", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let records = parse_ndjson(&out.stdout);
    // Only alpha is enabled; its entries default to tier 3.
    assert_eq!(records.len(), 1, "only the enabled plugin can match a tier");
    assert_eq!(records[0]["id"]["plugin"], "plugin-alpha");
}

#[test]
fn list_filter_and_tier_compose_with_and() {
    // #330: filters compose with logical AND. `--filter beta --tier 1` must
    // yield nothing: plugin-beta matches the name filter but is disabled (no
    // tier-1 entry), and plugin-alpha has a tier-1 entry but its name/desc do
    // not contain "beta"... except beta appears in NEITHER alpha's name nor its
    // description, so the AND is empty.
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    // Promote skill-a to tier 1 so alpha has a tier-1 entry.
    let set = env
        .cmd()
        .args(["tier", "set", "plugin-alpha/skill-a", "1"])
        .output()
        .unwrap();
    assert!(set.status.success());

    // `--filter alpha --tier 1` → alpha matches both → one row.
    let out = env
        .cmd()
        .args([
            "plugin", "list", "--filter", "alpha", "--tier", "1", "--json",
        ])
        .output()
        .unwrap();
    assert!(out.status.success());
    let records = parse_ndjson(&out.stdout);
    assert_eq!(records.len(), 1, "alpha matches both name and tier");
    assert_eq!(records[0]["id"]["plugin"], "plugin-alpha");

    // `--filter beta --tier 1` → beta matches the name but has no tier-1 entry
    // (it's disabled); AND is empty.
    let out2 = env
        .cmd()
        .args([
            "plugin", "list", "--filter", "beta", "--tier", "1", "--json",
        ])
        .output()
        .unwrap();
    assert!(out2.status.success());
    assert!(
        out2.stdout.is_empty(),
        "beta matches the name filter but has no tier-1 entry: {:?}",
        String::from_utf8_lossy(&out2.stdout),
    );
}

#[test]
fn list_tier_out_of_range_is_usage_error() {
    // Clap's `value_parser!(u8).range(1..=3)` rejects 0 / 4 with a usage error
    // (exit 2), never reaching the command body.
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    for bad in ["0", "4", "9"] {
        let out = env
            .cmd()
            .args(["plugin", "list", "--tier", bad])
            .output()
            .unwrap();
        assert_eq!(
            out.status.code(),
            Some(2),
            "`--tier {bad}` must be a clap usage error (exit 2)",
        );
    }
}

#[test]
fn list_catalog_filter_narrows_results() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    // Filter to the catalog we registered — same content as the default
    // list, but exercises the `--catalog` code path explicitly.
    let out = env
        .cmd()
        .args([
            "plugin",
            "list",
            "--catalog",
            "sample-plugin-catalog",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let records = parse_ndjson(&out.stdout);
    assert_eq!(records.len(), 2);
    for r in &records {
        assert_eq!(r["id"]["catalog"], "sample-plugin-catalog");
    }
}

#[test]
fn list_unknown_catalog_filter_exits_with_catalog_not_found_code() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["plugin", "list", "--catalog", "does-not-exist", "--json"])
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "expected failure for unknown catalog filter",
    );
    // Exit code 3 corresponds to CatalogNotFound (see src/error.rs).
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn list_with_no_catalogs_human_mode_nudges_to_add_a_catalog() {
    // #293: catalog-aware empty state. No catalogs enrolled → the fix is to
    // add one, mirroring the `catalog list` nudge.
    let env = ToolEnv::new();
    let out = env.cmd().args(["plugin", "list"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tome catalog add"),
        "expected an add-catalog nudge, got: {stdout}",
    );
}

#[test]
fn list_enabled_only_with_nothing_enabled_nudges_to_enable_a_plugin() {
    // #293: a catalog IS enrolled (its plugins show up in the default list),
    // but `--enabled-only` filters everything out because nothing is enabled.
    // With catalogs present, the empty state must nudge toward enabling a
    // plugin, NOT adding a catalog.
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = setup_with_alpha_enabled(&env, &fixture_tmp, "sample-plugin-catalog");

    // Disable the pre-enabled plugin so `--enabled-only` yields zero rows
    // while the catalog stays enrolled.
    let out = env
        .cmd()
        .args([
            "plugin",
            "disable",
            "sample-plugin-catalog/plugin-alpha",
            "--force",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "disable failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let _ = &paths; // keep the isolated root alive for the second invocation.

    let out = env
        .cmd()
        .args(["plugin", "list", "--enabled-only"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("tome plugin enable"),
        "expected an enable-plugin nudge, got: {stdout}",
    );
    assert!(
        !stdout.contains("tome catalog add"),
        "with a catalog enrolled the nudge must not suggest adding a catalog, got: {stdout}",
    );
}

#[test]
fn list_with_no_catalogs_registered_emits_nothing_in_json_mode() {
    // Sanity check: the empty-config baseline is well-behaved even without
    // a fabricated index DB. Establishes the "zero state" the other tests
    // build on top of.
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["plugin", "list", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(
        out.stdout.is_empty(),
        "expected empty JSON stream, got {:?}",
        String::from_utf8_lossy(&out.stdout),
    );

    // Cross-check: the fixture itself must exist so future contributors
    // know they should not delete it.
    let fixture = sample_plugin_catalog_fixture();
    assert!(
        fixture.is_dir(),
        "expected sample-plugin-catalog fixture at {}",
        fixture.display(),
    );
    let manifest: &Path = &fixture.join("tome-catalog.toml");
    assert!(manifest.is_file(), "missing tome-catalog.toml in fixture");
}

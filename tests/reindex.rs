//! Phase 7 / US5 slice 3 — `tome reindex [<scope>] [--force]`.
//!
//! Library-API tests for the explicit reindex subcommand. The aggregate
//! output / NDJSON record is asserted via the `run_with_deps` entry point
//! (which is a thin wrapper around `execute` + `emit`). The CLI binary
//! path is exercised only for the parse-error / unknown-catalog cases
//! that don't need an embedder — same boundary as plugin enable.

mod common;

use common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_models, lifecycle_paths,
    stub_embedder_seed, stub_reranker_seed,
};
use tempfile::TempDir;
use tome::commands::reindex::{Scope, run_with_deps};
use tome::embedding::stub::StubEmbedder;
use tome::output::Mode;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

fn enable_alpha_and_beta(
    paths: &tome::paths::Paths,
    config: &tome::config::Config,
    embedder: &StubEmbedder,
) {
    let alpha: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let beta: PluginId = "sample-plugin-catalog/plugin-beta".parse().unwrap();
    let deps = LifecycleDeps {
        paths,
        scope: &tome::workspace::Scope::Global,
        config,
        embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        allow_model_download: false,
    };
    lifecycle::enable(&alpha, &deps).expect("enable alpha");
    lifecycle::enable(&beta, &deps).expect("enable beta");
}

fn build_deps<'a>(
    paths: &'a tome::paths::Paths,
    config: &'a tome::config::Config,
    embedder: &'a StubEmbedder,
) -> LifecycleDeps<'a> {
    LifecycleDeps {
        paths,
        scope: &tome::workspace::Scope::Global,
        config,
        embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        allow_model_download: false,
    }
}

#[test]
fn reindex_all_visits_every_enabled_plugin_zero_changes() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha_and_beta(&paths, &config, &embedder);
    let baseline = embedder.call_count();

    let plugins = vec![
        "sample-plugin-catalog/plugin-alpha".parse().unwrap(),
        "sample-plugin-catalog/plugin-beta".parse().unwrap(),
    ];
    let agg = run_with_deps(
        Scope::All,
        &plugins,
        &build_deps(&paths, &config, &embedder),
        false,
        Mode::Json,
    )
    .expect("reindex all");

    assert_eq!(agg.plugins_visited, 2);
    // plugin-alpha has 4 valid skills, plugin-beta has 3 valid skills.
    assert_eq!(agg.skills_checked, 5);
    assert_eq!(agg.skills_re_embedded, 0);
    assert_eq!(agg.skills_unchanged, 5);
    assert_eq!(
        embedder.call_count(),
        baseline,
        "no skill changed — no embed call should fire",
    );
}

#[test]
fn reindex_one_plugin_re_embeds_only_modified_skill() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha_and_beta(&paths, &config, &embedder);
    let baseline = embedder.call_count();

    // Mutate skill-a in plugin-alpha.
    let skill_a = catalog_root
        .join("plugin-alpha")
        .join("skills")
        .join("skill-a")
        .join("SKILL.md");
    std::fs::write(
        &skill_a,
        "---\nname: skill-a\ndescription: rewritten by slice 3\n---\nbody\n",
    )
    .unwrap();

    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let agg = run_with_deps(
        Scope::Plugin(id.clone()),
        &[id],
        &build_deps(&paths, &config, &embedder),
        false,
        Mode::Json,
    )
    .expect("reindex one plugin");

    assert_eq!(agg.plugins_visited, 1);
    assert_eq!(agg.skills_checked, 4);
    assert_eq!(agg.skills_re_embedded, 1);
    assert_eq!(agg.skills_unchanged, 3);
    assert_eq!(
        embedder.call_count() - baseline,
        1,
        "exactly one embed call for the modified skill",
    );
}

#[test]
fn reindex_force_re_embeds_every_skill_in_scope() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha_and_beta(&paths, &config, &embedder);
    let baseline = embedder.call_count();

    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let agg = run_with_deps(
        Scope::Plugin(id.clone()),
        &[id],
        &build_deps(&paths, &config, &embedder),
        true,
        Mode::Json,
    )
    .expect("force reindex");

    assert_eq!(agg.plugins_visited, 1);
    assert_eq!(agg.skills_re_embedded, 4);
    assert_eq!(agg.skills_unchanged, 0);
    assert_eq!(embedder.call_count() - baseline, 4);
}

#[test]
fn reindex_catalog_scope_visits_every_plugin_in_that_catalog() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha_and_beta(&paths, &config, &embedder);

    let plugins = vec![
        "sample-plugin-catalog/plugin-alpha".parse().unwrap(),
        "sample-plugin-catalog/plugin-beta".parse().unwrap(),
    ];
    let agg = run_with_deps(
        Scope::Catalog("sample-plugin-catalog".to_owned()),
        &plugins,
        &build_deps(&paths, &config, &embedder),
        false,
        Mode::Json,
    )
    .expect("reindex catalog");
    assert_eq!(agg.plugins_visited, 2);
    assert_eq!(agg.skills_checked, 5);
    assert_eq!(agg.skills_unchanged, 5);
}

// ---- CLI binary tests for the easy error paths --------------------------
//
// These don't need an embedder — they only hit the scope-parse path.

#[test]
fn reindex_unknown_catalog_exits_3() {
    let env = ToolEnv::new();
    let out = env.cmd().args(["reindex", "ghost"]).output().unwrap();
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn reindex_invalid_scope_format_exits_2() {
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["reindex", "bad/id/extra"])
        .output()
        .unwrap();
    // Two slashes — invalid plugin id format → Usage (exit 2).
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn reindex_empty_scope_with_no_enabled_plugins_is_a_clean_zero() {
    let env = ToolEnv::new();
    let out = env.cmd().args(["reindex"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Nothing to reindex"),
        "expected 'Nothing to reindex' message, got: {stdout}",
    );
}

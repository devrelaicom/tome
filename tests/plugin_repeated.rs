//! FR-008 — repeated-state idempotency for enable / disable.
//!
//! Both `tome plugin enable` and `tome plugin disable` must exit with code
//! 21 (`PluginAlreadyInState`) when invoked against a plugin that is already
//! in the requested state. This file consolidates that contract clause in
//! one place.
//!
//! The enable case is driven through the lifecycle library API because the
//! `tome plugin enable` CLI path loads `FastembedEmbedder` (real ONNX); the
//! handover gotcha #10 boundary applies. The disable case is driven through
//! the CLI binary, which gives us a real `Some(21)` process exit code —
//! the lifecycle's `TomeError::PluginAlreadyInState → 21` mapping is
//! already locked in by `tests/exit_codes.rs`.
//!
//! Spec: `contracts/plugin-commands.md` §"`tome plugin enable`" step 3,
//! §"`tome plugin disable`" step 2.

mod common;

use common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, enrol_catalog_symlinked,
    fabricate_models, paths_for, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
    write_config_for_cli,
};
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::error::{PluginState, TomeError};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

#[test]
fn enable_of_already_enabled_returns_plugin_already_in_state_via_library() {
    let tmp = TempDir::new().unwrap();
    let paths = common::lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    // FF1: `lifecycle::enable` resolves the plugin dir from the DB enrolment,
    // not `config.toml`, so enrol the catalog + symlink the cache dir onto the
    // on-disk fixture before enabling. The in-memory `config` is kept for the
    // `LifecycleDeps` shape.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
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
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();

    lifecycle::enable(&id, &deps).expect("first enable");

    let err = lifecycle::enable(&id, &deps).expect_err("re-enable must error");
    match err {
        TomeError::PluginAlreadyInState { state, plugin } => {
            assert_eq!(state, PluginState::Enabled);
            assert_eq!(plugin, "sample-plugin-catalog/plugin-alpha");
        }
        other => panic!("expected PluginAlreadyInState (exit 21), got {other:?}"),
    }
}

#[test]
fn disable_of_already_disabled_via_cli_exits_21() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&fixture_tmp, "catalog");
    let cli_config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &cli_config);

    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        config: &cli_config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("pre-enable plugin-alpha");

    // First disable: succeeds.
    let first = env
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
        first.status.success(),
        "first disable must succeed; stderr: {}",
        String::from_utf8_lossy(&first.stderr),
    );

    // Second disable: must exit 21.
    let second = env
        .cmd()
        .args([
            "plugin",
            "disable",
            "sample-plugin-catalog/plugin-alpha",
            "--force",
        ])
        .output()
        .unwrap();
    assert_eq!(
        second.status.code(),
        Some(21),
        "expected exit 21 (PluginAlreadyInState), got {:?}; stderr: {}",
        second.status.code(),
        String::from_utf8_lossy(&second.stderr),
    );
}

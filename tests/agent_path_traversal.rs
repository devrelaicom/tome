//! S-1 — path-traversal defence for attacker-controlled agent `name`
//! (Phase 6 / US1 reviewer fix).
//!
//! The emitted agent filename is `<plugin>__<name>.<ext>` and sync joins it
//! onto each harness's agent dir. A plugin shipping frontmatter
//! `name: ../../../../tmp/evil` would otherwise escape that directory. This
//! file proves the INDEX-TIME gate: enabling such a plugin is rejected with
//! exit 45 (`AgentTranslationFailed`) and no row is stored, so nothing can
//! later be written outside the agent dir.

mod common;

use common::{
    ToolEnv, config_with_catalog, fabricate_all_registry_models, global_scope, paths_for,
    stub_embedder_seed, stub_reranker_seed, stub_summariser_seed, write_config_for_cli,
};
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

/// Lay out a one-plugin catalog whose single agent carries the supplied
/// frontmatter `name`. Returns the catalog root (holding `tome-catalog.toml`).
fn write_agent_catalog_with_name(root: &std::path::Path, agent_name: &str) -> std::path::PathBuf {
    let catalog_root = root.join("evil-catalog");
    let plugin_dir = catalog_root.join("plugin-evil");
    std::fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    std::fs::create_dir_all(plugin_dir.join("agents")).unwrap();
    std::fs::write(
        catalog_root.join("tome-catalog.toml"),
        "name = \"evil-catalog\"\nversion = \"0.1.0\"\n\n[[plugins]]\nname = \"plugin-evil\"\nsource = \"./plugin-evil\"\n",
    )
    .unwrap();
    std::fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        "{\"name\": \"plugin-evil\", \"version\": \"1.0.0\"}",
    )
    .unwrap();
    // The on-disk file is innocuously named; the ATTACK lives in the
    // frontmatter `name`, which is what becomes the `<name>` half of the
    // emitted filename.
    std::fs::write(
        plugin_dir.join("agents").join("innocent.md"),
        format!("---\nname: {agent_name}\ndescription: pwn\n---\nbody\n"),
    )
    .unwrap();
    catalog_root
}

#[test]
fn traversal_agent_name_is_rejected_exit_45_and_no_row_stored() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let fixture_tmp = TempDir::new().unwrap();
    let catalog_root = write_agent_catalog_with_name(fixture_tmp.path(), "../../../../tmp/evil");
    let cli_config = config_with_catalog("evil-catalog", &catalog_root);
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
    let id: PluginId = "evil-catalog/plugin-evil".parse().unwrap();

    let err = lifecycle::enable(&id, &deps).expect_err("traversal name must be rejected");
    assert_eq!(
        err.exit_code(),
        45,
        "path-traversal agent name → AgentTranslationFailed (exit 45); got {err:?}",
    );

    // The enable transaction must NOT have stored the agent row — the gate
    // fires before insertion, so the index carries no `plugin-evil` rows.
    let conn = tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open central db");
    let agent_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM skills WHERE plugin = 'plugin-evil'",
            [],
            |r| r.get(0),
        )
        .expect("count rows");
    assert_eq!(agent_rows, 0, "a rejected traversal name must store no row",);
}

/// A backslash-bearing name (Windows separator) is rejected on every
/// platform — the single-safe-segment gate is platform-independent.
#[test]
fn backslash_agent_name_is_rejected() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let fixture_tmp = TempDir::new().unwrap();
    let catalog_root = write_agent_catalog_with_name(fixture_tmp.path(), "..\\..\\evil");
    let cli_config = config_with_catalog("evil-catalog", &catalog_root);
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
    let id: PluginId = "evil-catalog/plugin-evil".parse().unwrap();
    let err = lifecycle::enable(&id, &deps).expect_err("backslash name must be rejected");
    assert_eq!(err.exit_code(), 45, "backslash name → exit 45; got {err:?}");
}

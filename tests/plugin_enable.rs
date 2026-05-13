//! Integration tests for `tome::plugin::lifecycle::enable`.
//!
//! Driven via the library API rather than the CLI binary because the
//! `tome plugin enable` command path loads `FastembedEmbedder` (real ONNX
//! model files). The stub embedder under `tome::embedding::stub` is
//! deterministic and lets these tests run without any model artefacts.
//!
//! Covers FR-004 (atomicity), FR-006 (no-op refresh), FR-011 (name
//! fallback), FR-012 (description fallback), FR-013c (skip a single skill
//! when only the YAML body is invalid), and the contract idempotency clause
//! from `contracts/plugin-commands.md` §1.

mod common;

use common::{
    config_with_catalog, copy_sample_plugin_catalog, fabricate_models, lifecycle_paths,
    stub_embedder_seed, stub_reranker_seed,
};
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::error::{PluginState, TomeError};
use tome::index::{self, OpenOptions};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

#[test]
fn enable_inserts_skill_rows_with_content_hash_and_enabled_flag() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "catalog");
    // The lifecycle resolver looks for `<catalog.path>/<plugin>`. The
    // fixture lays out plugin directories as immediate children of the
    // catalog root, matching the on-disk convention `tome catalog add`
    // records (entry.path = directory containing tome-catalog.toml).
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();

    let outcome = lifecycle::enable(&id, &deps).expect("enable should succeed");

    // skill-a, skill-b (name fallback), skill-c (description fallback), and
    // skill-d (extra frontmatter fields) all land. skill-malformed-yaml-body
    // is skipped per FR-013c.
    assert_eq!(outcome.summary.total_skills, 4);
    assert_eq!(outcome.summary.newly_embedded, 4);
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.contains("frontmatter YAML invalid")
                && w.contains("skill-malformed-yaml-body")),
        "expected FR-013c skip warning, got {:?}",
        outcome.warnings,
    );

    // Each row should be enabled with a non-empty content_hash.
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
        },
    )
    .unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT name, enabled, content_hash, plugin_version FROM skills
             WHERE catalog = ?1 AND plugin = ?2 ORDER BY name",
        )
        .unwrap();
    let rows: Vec<(String, i64, String, String)> = stmt
        .query_map(["sample-plugin-catalog", "plugin-alpha"], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .map(|r| r.unwrap())
        .collect();

    assert_eq!(rows.len(), 4);
    for (name, enabled, content_hash, plugin_version) in &rows {
        assert_eq!(*enabled, 1, "row {name} should be enabled");
        assert!(
            !content_hash.is_empty(),
            "row {name} must record a content_hash",
        );
        assert_eq!(content_hash.len(), 64, "content_hash must be hex SHA-256");
        assert_eq!(plugin_version, "1.2.3", "plugin version must propagate");
    }

    // Sanity-check the names — confirm the FR-011 directory-name fallback
    // landed for skill-b (no `name` field) and the others resolved as-is.
    let names: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(names, &["skill-a", "skill-b", "skill-c", "skill-d"]);
}

#[test]
fn enable_emits_fallback_warnings_for_missing_name_and_description() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();

    let outcome = lifecycle::enable(&id, &deps).expect("enable");

    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.contains("name fallback applied") && w.contains("skill-b")),
        "expected name-fallback warning for skill-b, got {:?}",
        outcome.warnings,
    );
    assert!(
        outcome
            .warnings
            .iter()
            .any(|w| w.contains("description fallback applied") && w.contains("skill-c")),
        "expected description-fallback warning for skill-c, got {:?}",
        outcome.warnings,
    );
}

#[test]
fn enable_is_idempotent_rejecting_re_enable_with_plugin_already_in_state() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();

    lifecycle::enable(&id, &deps).expect("first enable");
    let err = lifecycle::enable(&id, &deps).expect_err("second enable rejected");
    match err {
        TomeError::PluginAlreadyInState { state, plugin } => {
            assert_eq!(state, PluginState::Enabled);
            assert_eq!(plugin, "sample-plugin-catalog/plugin-alpha");
        }
        other => panic!("expected PluginAlreadyInState, got {other:?}"),
    }
}

#[test]
fn enable_returns_model_missing_when_no_models_on_disk_and_download_disallowed() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    // Deliberately skip fabricate_models — the model-presence gate must fire.

    let catalog_root = copy_sample_plugin_catalog(&tmp, "catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();

    let err = lifecycle::enable(&id, &deps).expect_err("model missing");
    assert!(
        matches!(err, TomeError::ModelMissing { .. }),
        "expected ModelMissing, got {err:?}",
    );
}

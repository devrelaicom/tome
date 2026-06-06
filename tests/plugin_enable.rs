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
    fabricate_models, lifecycle_paths, stage_catalog_dir_in_db, stage_sample_catalog_in_db,
    stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
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
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // FF1: enrol via the DB + stage the clone in the content-addressed cache,
    // the on-disk shape `tome catalog add` produces. `resolve_plugin_dir`
    // reads the enrolment URL → `cache_dir_for(url)`; the fixture lays plugin
    // directories out as immediate children of that catalog root.
    stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");
    let config = tome::config::Config::default();

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
            summariser: stub_summariser_seed(),
        },
    )
    .unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT s.name,
                    CASE WHEN ws.skill_id IS NOT NULL THEN 1 ELSE 0 END AS enabled,
                    s.content_hash, s.plugin_version
             FROM skills AS s
             LEFT JOIN workspace_skills AS ws
                    ON ws.skill_id = s.id
                   AND ws.workspace_id = (SELECT id FROM workspaces WHERE name = 'global')
             WHERE s.catalog = ?1 AND s.plugin = ?2 ORDER BY s.name",
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
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");
    let config = tome::config::Config::default();

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
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");
    let config = tome::config::Config::default();

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
fn enable_resolves_nested_plugin_source_from_catalog_manifest() {
    // Regression: prior to making the resolver manifest-first, the lifecycle
    // joined `entry.path` with `id.plugin` literally. A catalog declaring
    // `plugins[].source = "./vendor/wrapped-alpha"` therefore failed with
    // `PluginNotFound` because lifecycle looked under
    // `<catalog>/alpha-plugin` instead of `<catalog>/vendor/wrapped-alpha`,
    // while `tome plugin list` (which already walks tome-catalog.toml) saw
    // the plugin and succeeded. Both surfaces now go through the manifest.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // FF1: stage a bare clone at the content-addressed cache dir + enrol it,
    // then build the nested tree there so `resolve_plugin_dir` reads it.
    let catalog_root = stage_catalog_dir_in_db(
        &paths,
        "global",
        "nested-test",
        &tmp.path().join("nonexistent"),
    );
    let plugin_dir = catalog_root.join("vendor/wrapped-alpha");
    std::fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    std::fs::write(
        plugin_dir.join("tome-plugin.toml"),
        format!(
            "name = \"{}\"\nversion = \"1.0.0\"\n",
            plugin_dir.file_name().unwrap().to_string_lossy()
        ),
    )
    .unwrap();
    std::fs::write(
        plugin_dir.join(".claude-plugin/plugin.json"),
        r#"{"name":"alpha-plugin","version":"1.0.0"}"#,
    )
    .unwrap();
    let skill_dir = plugin_dir.join("skills/alpha-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: alpha\ndescription: an alpha skill\n---\nbody",
    )
    .unwrap();

    // Catalog manifest declares the plugin under a nested vendor path.
    // `parse_and_validate` canonicalises `source` against `catalog_root`, so
    // the directory above must exist before this point — it does.
    let toml = r#"name = "nested-test"
description = "Regression fixture for the manifest-driven resolver"
version = "0.1.0"

[owner]
name = "Tome Test"
email = "tests@tome.invalid"

[[plugins]]
name = "alpha-plugin"
source = "./vendor/wrapped-alpha"
"#;
    std::fs::write(catalog_root.join("tome-catalog.toml"), toml).unwrap();

    let config = tome::config::Config::default();

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
    let id: PluginId = "nested-test/alpha-plugin".parse().unwrap();

    let outcome = lifecycle::enable(&id, &deps)
        .expect("enable should resolve the plugin via the catalog manifest");
    assert_eq!(outcome.summary.total_skills, 1);
    assert_eq!(outcome.summary.newly_embedded, 1);
}

#[test]
fn cheap_reenable_after_disable_invokes_embedder_zero_times() {
    // FR-006: re-enable of a plugin whose skill content is unchanged must
    // skip the embedder and merely flip `enabled = 1`. The hash comparison
    // in `index::skills::enable_plugin_atomic` is the load-bearing branch.
    // We instrument the stub embedder via `call_count()` to assert that the
    // closure was not invoked a second time.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // FF1: enrol via the DB + stage the clone in the content-addressed cache,
    // mirroring `tome catalog add`. The catalog is NOT written to config.toml.
    stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");
    let config = tome::config::Config::default();

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

    let first = lifecycle::enable(&id, &deps).expect("first enable");
    // Four skills land (skill-malformed-yaml-body is skipped). All freshly
    // embedded — the embedder closure ran 4 times.
    assert_eq!(first.summary.total_skills, 4);
    assert_eq!(first.summary.newly_embedded, 4);
    let calls_after_first_enable = embedder.call_count();
    assert_eq!(
        calls_after_first_enable, 4,
        "first enable must invoke embedder once per skill",
    );

    // Disable, then re-enable with the same StubEmbedder so the call counter
    // persists across the round-trip.
    lifecycle::disable(
        &id,
        &paths,
        &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        stub_embedder_seed(),
        stub_reranker_seed(),
        stub_summariser_seed(),
    )
    .expect("disable");

    let second = lifecycle::enable(&id, &deps).expect("cheap re-enable");
    // Cheap path: same 4 skills surface, but `newly_embedded` collapses to 0
    // and the embedder closure is not invoked.
    assert_eq!(second.summary.total_skills, 4);
    assert_eq!(
        second.summary.newly_embedded, 0,
        "cheap re-enable must report zero newly-embedded skills",
    );
    assert_eq!(
        embedder.call_count(),
        calls_after_first_enable,
        "cheap re-enable must not invoke the embedder",
    );
}

#[test]
fn enable_returns_model_missing_when_no_models_on_disk_and_download_disallowed() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    // Deliberately skip fabricate_models — the model-presence gate must fire.

    stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");
    let config = tome::config::Config::default();

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

    let err = lifecycle::enable(&id, &deps).expect_err("model missing");
    assert!(
        matches!(err, TomeError::ModelMissing { .. }),
        "expected ModelMissing, got {err:?}",
    );
}

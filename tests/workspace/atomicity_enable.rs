//! Atomicity (FR-004) integration test for `tome::plugin::lifecycle::enable`.
//!
//! Uses `StubEmbedder::with_force_fail_after(n)` to inject a mid-pipeline
//! embedder failure. The contract: when any step inside the enable
//! transaction fails, the on-disk index must be indistinguishable from its
//! pre-call state — zero rows for the affected plugin in both `skills` and
//! `skill_embeddings`.
//!
//! Process-global SIGINT simulation (`catalog::git::CANCELLED`) was
//! deliberately avoided here; flipping it races every other test in the
//! binary because cargo runs tests inside the same process. The
//! `force_fail_after` knob is the atomicity dial.

use crate::common::{
    config_with_catalog, copy_sample_plugin_catalog, enrol_catalog_symlinked, fabricate_models,
    lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::error::TomeError;
use tome::index::{self, OpenOptions};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

#[test]
fn enable_rolls_back_when_embedder_fails_mid_pipeline() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog + symlink the cache dir onto the on-disk tree so
    // `resolve_plugin_dir` (DB-backed) resolves the fixture.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    // plugin-alpha exposes 5 SKILL.md files (one of which is the
    // FR-013c-skipped malformed-yaml-body). The valid count is 4. Failing
    // after 2 successful embeds means embeds 3 and 4 will error — well
    // inside the transaction, before commit.
    let embedder = StubEmbedder::with_force_fail_after(2);
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

    let err = lifecycle::enable(&id, &deps).expect_err("forced failure");
    match err {
        TomeError::EmbeddingGenerationFailure { input_desc, detail } => {
            assert_eq!(input_desc, "stub-forced-failure");
            assert!(
                detail.contains("forced failure after 2"),
                "unexpected detail: {detail}",
            );
        }
        other => panic!("expected EmbeddingGenerationFailure, got {other:?}"),
    }

    // Failure injection must have actually fired — i.e. the embedder was
    // called more than `force_fail_after` times. This guards against a
    // future change that bails out before any embedding work happens.
    assert!(
        embedder.call_count() > 2,
        "embedder call count ({}) should exceed force_fail_after threshold (2)",
        embedder.call_count(),
    );

    // Rollback invariant: zero rows for this plugin in `skills`.
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .unwrap();
    let total_skills: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM skills WHERE catalog = ?1 AND plugin = ?2",
            ["sample-plugin-catalog", "plugin-alpha"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        total_skills, 0,
        "skills table must be empty for this plugin after rollback",
    );

    // Rollback invariant: zero embedding rows joined to this plugin.
    let total_embeddings: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM skill_embeddings e
             JOIN skills s ON s.id = e.skill_id
             WHERE s.catalog = ?1 AND s.plugin = ?2",
            ["sample-plugin-catalog", "plugin-alpha"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        total_embeddings, 0,
        "skill_embeddings must be empty for this plugin after rollback",
    );
}

#[test]
fn enable_failure_does_not_taint_a_subsequent_clean_enable() {
    // After a failed atomic enable, a fresh enable against a non-failing
    // embedder should succeed and leave the index in a consistent state.
    // This is the user-visible side of the rollback contract.
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog + symlink the cache dir onto the on-disk tree so
    // `resolve_plugin_dir` (DB-backed) resolves the fixture.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    // First attempt: fail after 1 embed.
    {
        let embedder = StubEmbedder::with_force_fail_after(1);
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
        lifecycle::enable(&id, &deps).expect_err("first enable should fail");
    }

    // Second attempt: clean stub — must succeed and insert the full 4 rows.
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
    let outcome = lifecycle::enable(&id, &deps).expect("second enable");
    assert_eq!(outcome.summary.total_skills, 4);
    assert_eq!(outcome.summary.newly_embedded, 4);
}

//! Library-API integration tests for the query path.
//!
//! Composition under test: enable a fixture plugin via the lifecycle API
//! (stub embedder), open the index read-only, embed a query with the same
//! stub, run KNN, then optionally rerank. We do not exercise
//! `commands::query` here because that path constructs a real
//! `FastembedEmbedder`/`FastembedReranker` and would need ONNX models on
//! disk.
//!
//! The stub embedder's determinism (SHA-256 → 384-dim vector) means
//! embedding `embedding_text(name, description)` twice yields the same
//! vector — the test uses that property to predict the top-1 result of a
//! KNN query.

mod common;

use common::{
    config_with_catalog, copy_sample_plugin_catalog, fabricate_models, lifecycle_paths,
    stub_embedder_seed, stub_reranker_seed,
};
use tempfile::TempDir;
use tome::embedding::stub::{ReverseStubReranker, StubEmbedder, StubReranker};
use tome::embedding::{Embedder, Reranker};
use tome::index::query::{Candidate, QueryFilters};
use tome::index::skills::embedding_text;
use tome::index::{self, OpenOptions, knn};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

/// Bootstrap helper: copy the sample-plugin-catalog fixture into a TempDir,
/// enable both plugins via the lifecycle API, return everything the tests
/// need to query the resulting index. Centralising the setup keeps the
/// individual test cases focused on the assertion under test.
struct QueryEnv {
    _tmp: TempDir,
    paths: tome::paths::Paths,
}

fn build_query_env() -> QueryEnv {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    for plugin_name in ["plugin-alpha", "plugin-beta"] {
        let id: PluginId = format!("sample-plugin-catalog/{plugin_name}")
            .parse()
            .unwrap();
        let deps = LifecycleDeps {
            paths: &paths,
            scope: &tome::workspace::Scope::Global,
            config: &config,
            embedder: &embedder,
            embedder_seed: stub_embedder_seed(),
            reranker_seed: stub_reranker_seed(),
            allow_model_download: false,
        };
        lifecycle::enable(&id, &deps).expect("enable plugin for query env");
    }

    QueryEnv { _tmp: tmp, paths }
}

fn open_conn(env: &QueryEnv) -> rusqlite::Connection {
    index::open(
        &env.paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
        },
    )
    .unwrap()
}

#[test]
fn knn_top_one_matches_self_embedded_skill() {
    let env = build_query_env();
    let conn = open_conn(&env);
    let embedder = StubEmbedder::new();

    // Target: skill-a in plugin-alpha. Embedding the exact text the
    // lifecycle hashes (`name\n\ndescription`) should yield the same vector,
    // so distance to its own row is ~0.
    let target_name = "skill-a";
    let target_description = "Well-formed skill that documents how to make alpha widgets shine.";
    let query_text = embedding_text(target_name, target_description);
    let query_vec = embedder.embed(&query_text).unwrap();

    let hits = knn(&conn, &query_vec, 10, &QueryFilters::default()).unwrap();
    assert!(!hits.is_empty(), "expected at least one KNN hit");
    let top = &hits[0];
    assert_eq!(top.name, target_name);
    assert_eq!(top.plugin, "plugin-alpha");
    assert_eq!(top.catalog, "sample-plugin-catalog");
    // Self-similarity → distance should be effectively zero.
    assert!(
        top.distance.abs() < 1e-4,
        "top-1 distance should be ~0 for a self-embedding, got {}",
        top.distance,
    );
}

#[test]
fn knn_filter_narrows_results_to_one_plugin() {
    let env = build_query_env();
    let conn = open_conn(&env);
    let embedder = StubEmbedder::new();

    // Unrelated query → results should span both plugins by default.
    let unrelated = embedder.embed("a totally unrelated query string").unwrap();
    let unfiltered: Vec<Candidate> = knn(&conn, &unrelated, 20, &QueryFilters::default()).unwrap();
    let unfiltered_plugins: std::collections::HashSet<&str> =
        unfiltered.iter().map(|c| c.plugin.as_str()).collect();
    assert!(
        unfiltered_plugins.contains("plugin-alpha"),
        "unfiltered KNN must surface plugin-alpha",
    );
    assert!(
        unfiltered_plugins.contains("plugin-beta"),
        "unfiltered KNN must surface plugin-beta",
    );

    // Now narrow to plugin-beta only.
    let filters = QueryFilters {
        catalog: Some("sample-plugin-catalog"),
        plugin: Some("plugin-beta"),
    };
    let filtered = knn(&conn, &unrelated, 20, &filters).unwrap();
    assert!(!filtered.is_empty(), "filter must not eliminate every row");
    for hit in &filtered {
        assert_eq!(hit.plugin, "plugin-beta");
        assert_eq!(hit.catalog, "sample-plugin-catalog");
    }
}

#[test]
fn knn_catalog_filter_rejects_unknown_catalog_with_empty_result() {
    let env = build_query_env();
    let conn = open_conn(&env);
    let embedder = StubEmbedder::new();
    let qv = embedder.embed("anything").unwrap();
    let filters = QueryFilters {
        catalog: Some("does-not-exist"),
        plugin: None,
    };
    let hits = knn(&conn, &qv, 10, &filters).unwrap();
    assert!(
        hits.is_empty(),
        "unknown-catalog filter must produce zero hits, got {hits:?}",
    );
}

#[test]
fn stub_reranker_preserves_input_order_and_scores_as_one_minus_distance() {
    let env = build_query_env();
    let conn = open_conn(&env);
    let embedder = StubEmbedder::new();

    let qv = embedder.embed("alpha widget shine").unwrap();
    let hits = knn(&conn, &qv, 5, &QueryFilters::default()).unwrap();
    assert!(hits.len() >= 2, "need >=2 candidates to test ordering");

    let before: Vec<(String, String)> = hits
        .iter()
        .map(|c| (c.plugin.clone(), c.name.clone()))
        .collect();
    let scored = StubReranker::new()
        .rerank("alpha widget shine", hits)
        .unwrap();
    let after: Vec<(String, String)> = scored
        .iter()
        .map(|s| (s.candidate.plugin.clone(), s.candidate.name.clone()))
        .collect();
    assert_eq!(before, after, "StubReranker must preserve input order");

    for s in &scored {
        let expected = 1.0 - s.candidate.distance;
        // Floating-point exact equality is safe here — the stub computes
        // exactly `1.0 - distance` without further arithmetic.
        assert!(
            (s.score - expected).abs() < f32::EPSILON,
            "expected score {expected}, got {}",
            s.score,
        );
    }
}

#[test]
fn reverse_stub_reranker_distinguishes_reranked_from_raw_ordering() {
    // Confirms the "rerank stage actually ran" signal: the reverse stub
    // flips the order, so we can tell apart "embedder output passed
    // through" from "reranker scored and reordered". This is the
    // distinction `tome query --no-rerank` exposes to the user.
    let env = build_query_env();
    let conn = open_conn(&env);
    let embedder = StubEmbedder::new();

    let qv = embedder.embed("plugin-beta skill-x sole skill").unwrap();
    let hits = knn(&conn, &qv, 5, &QueryFilters::default()).unwrap();
    assert!(hits.len() >= 2);

    let original: Vec<String> = hits.iter().map(|c| c.name.clone()).collect();
    let reversed = ReverseStubReranker::new().rerank("q", hits).unwrap();
    let reranked: Vec<String> = reversed.iter().map(|s| s.candidate.name.clone()).collect();

    // The reverse reranker flips the input order; check that the head of
    // the reranked list matches the tail of the original.
    assert_eq!(
        reranked.first(),
        original.last(),
        "ReverseStubReranker must put the original tail at the head",
    );
    assert_eq!(
        reranked.last(),
        original.first(),
        "ReverseStubReranker must put the original head at the tail",
    );
}

#[test]
fn knn_rejects_query_vector_of_wrong_length() {
    let env = build_query_env();
    let conn = open_conn(&env);
    // 383-dim vector — one short of the FLOAT[384] virtual table column.
    let bad = vec![0.0f32; 383];
    let err = knn(&conn, &bad, 5, &QueryFilters::default()).unwrap_err();
    assert!(
        matches!(err, tome::error::TomeError::IndexIntegrityCheckFailure(_)),
        "expected IndexIntegrityCheckFailure, got {err:?}",
    );
}

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
    config_with_catalog, copy_sample_plugin_catalog, enrol_catalog_symlinked, fabricate_models,
    lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use tome::cli::QueryArgs;
use tome::commands::query::{QueryDeps, ScoringMode, run_with_deps};
use tome::config::Config;
use tome::embedding::stub::{ReverseStubReranker, StubEmbedder, StubReranker};
use tome::embedding::{Embedder, Reranker};
use tome::index::query::{Candidate, QueryFilters};
use tome::index::skills::embedding_text;
use tome::index::{self, OpenOptions, knn};
use tome::output::Mode;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{Scope, WorkspaceName};

/// Bootstrap helper: copy the sample-plugin-catalog fixture into a TempDir,
/// enable both plugins via the lifecycle API, return everything the tests
/// need to query the resulting index. Centralising the setup keeps the
/// individual test cases focused on the assertion under test.
struct QueryEnv {
    _tmp: TempDir,
    paths: tome::paths::Paths,
    /// The catalog config used to bootstrap the env. Kept so library-API
    /// tests (`run_with_deps`) can pass it as `QueryDeps.config` without
    /// re-reading from disk.
    config: Config,
}

fn build_query_env() -> QueryEnv {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // FF1: `lifecycle::enable` resolves the plugin dir from the DB enrolment
    // now, so enrol the catalog + symlink the cache dir onto the on-disk
    // fixture. The in-memory `config` is still built because the `query`
    // command's own filter validation reads `config.catalogs` (its migration
    // off config.toml is a later PR).
    let catalog_root = copy_sample_plugin_catalog(&tmp, "catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    // Stamp the index `meta` with STUB seeds on the FIRST open, BEFORE
    // `enrol_catalog_symlinked` (whose `enrol_catalog_row` opens with REGISTRY
    // seeds). `index::open` only writes seeds when bootstrapping a fresh DB and
    // ignores `OpenOptions` on a reopen, so this first open wins the stamp and
    // the later registry-seeded open is a no-op for `meta`. This is required
    // for `run_with_deps`: its `pipeline` runs drift detection comparing the
    // stub `deps.embedder_seed` against the on-disk `meta` — a registry-stamped
    // `meta` would hard-fail with `EmbedderNameDrift`. The direct `knn_*` tests
    // don't hit this because `knn` skips drift detection.
    drop(
        index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: stub_embedder_seed(),
                reranker: stub_reranker_seed(),
                summariser: stub_summariser_seed(),
            },
        )
        .expect("bootstrap index meta with stub seeds"),
    );
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    for plugin_name in ["plugin-alpha", "plugin-beta"] {
        let id: PluginId = format!("sample-plugin-catalog/{plugin_name}")
            .parse()
            .unwrap();
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
        lifecycle::enable(&id, &deps).expect("enable plugin for query env");
    }

    QueryEnv {
        _tmp: tmp,
        paths,
        config,
    }
}

fn open_conn(env: &QueryEnv) -> rusqlite::Connection {
    index::open(
        &env.paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
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
    let query_text = embedding_text(target_name, target_description, None);
    let query_vec = embedder.embed(&query_text).unwrap();

    let hits = knn(&conn, "global", &query_vec, 10, &QueryFilters::default()).unwrap();
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
    let unfiltered: Vec<Candidate> =
        knn(&conn, "global", &unrelated, 20, &QueryFilters::default()).unwrap();
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
    let filtered = knn(&conn, "global", &unrelated, 20, &filters).unwrap();
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
    let hits = knn(&conn, "global", &qv, 10, &filters).unwrap();
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
    let hits = knn(&conn, "global", &qv, 5, &QueryFilters::default()).unwrap();
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
    let hits = knn(&conn, "global", &qv, 5, &QueryFilters::default()).unwrap();
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
    let err = knn(&conn, "global", &bad, 5, &QueryFilters::default()).unwrap_err();
    assert!(
        matches!(err, tome::error::TomeError::IndexIntegrityCheckFailure(_)),
        "expected IndexIntegrityCheckFailure, got {err:?}",
    );
}

// ---- run_with_deps library API (Phase 3 slice F6) -------------------------

/// Construct a `QueryArgs` with sensible defaults for the test path. JSON
/// mode keeps stdout structured but we discard it (no assertion against
/// the emitted bytes); the assertion target is the `QueryOutcome` return
/// value.
fn args_for(text: &str, top_k: u32) -> QueryArgs {
    QueryArgs {
        text: text.to_owned(),
        top_k,
        catalog: None,
        plugin: None,
        no_rerank: true,
        strict: false,
        min_score: None,
    }
}

#[test]
fn run_with_deps_returns_scored_results_without_reranker() {
    let env = build_query_env();
    let embedder = StubEmbedder::new();

    let target_name = "skill-a";
    let target_description = "Well-formed skill that documents how to make alpha widgets shine.";
    let query_text = embedding_text(target_name, target_description, None);

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &Scope(WorkspaceName::global()),
        config: &env.config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };

    let outcome = run_with_deps(args_for(&query_text, 5), deps, Mode::Json).expect("run_with_deps");

    assert_eq!(outcome.scoring, ScoringMode::Similarity);
    assert!(outcome.reranker_drift.is_none());
    assert!(
        !outcome.results.is_empty(),
        "expected at least one scored result",
    );
    let top = &outcome.results[0];
    assert_eq!(top.candidate.name, target_name);
    assert_eq!(top.candidate.plugin, "plugin-alpha");
    // Self-similarity → score should be ~1.0 (1.0 − 0 distance).
    assert!(
        (top.score - 1.0).abs() < 1e-3,
        "top-1 score should be ~1.0, got {}",
        top.score,
    );
}

#[test]
fn run_with_deps_uses_reranker_when_provided() {
    let env = build_query_env();
    let embedder = StubEmbedder::new();
    let reranker = ReverseStubReranker::new();

    let mut args = args_for("alpha widget", 5);
    args.no_rerank = false; // exercise the rerank branch

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &Scope(WorkspaceName::global()),
        config: &env.config,
        embedder: &embedder,
        reranker: Some(&reranker),
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };

    let outcome = run_with_deps(args, deps, Mode::Json).expect("run_with_deps with reranker");

    assert_eq!(outcome.scoring, ScoringMode::Reranked);
    assert!(!outcome.results.is_empty());
}

#[test]
fn run_with_deps_rejects_empty_query_text() {
    let env = build_query_env();
    let embedder = StubEmbedder::new();

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &Scope(WorkspaceName::global()),
        config: &env.config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };

    let err =
        run_with_deps(args_for("   ", 5), deps, Mode::Json).expect_err("empty query must error");
    assert!(matches!(err, tome::error::TomeError::Usage(_)));
}

#[test]
fn run_with_deps_strict_returns_query_no_results_on_empty_match() {
    let env = build_query_env();
    let embedder = StubEmbedder::new();

    // Cosine-similarity scoring with --strict + a hard threshold above
    // the max possible 1.0 score guarantees zero rows pass.
    let mut args = args_for("any query string", 5);
    args.strict = true;
    args.min_score = Some(2.0);

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &Scope(WorkspaceName::global()),
        config: &env.config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };

    let err = run_with_deps(args, deps, Mode::Json).expect_err("strict must surface no-results");
    assert!(matches!(
        err,
        tome::error::TomeError::QueryNoResultsStrict { .. }
    ));
}

#[test]
fn run_with_deps_filters_applied_pre_rerank() {
    let env = build_query_env();
    let embedder = StubEmbedder::new();
    let reranker = StubReranker::new();

    let mut args = args_for("anything", 10);
    args.no_rerank = false;
    args.catalog = Some("sample-plugin-catalog".to_owned());
    args.plugin = Some("plugin-beta".to_owned());

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &Scope(WorkspaceName::global()),
        config: &env.config,
        embedder: &embedder,
        reranker: Some(&reranker),
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };

    let outcome = run_with_deps(args, deps, Mode::Json).expect("run_with_deps");
    assert!(
        !outcome.results.is_empty(),
        "filter must not erase all hits"
    );
    for hit in &outcome.results {
        assert_eq!(hit.candidate.plugin, "plugin-beta");
        assert_eq!(hit.candidate.catalog, "sample-plugin-catalog");
    }
}

#[test]
fn run_with_deps_unknown_catalog_filter_returns_catalog_not_found() {
    let env = build_query_env();
    let embedder = StubEmbedder::new();

    let mut args = args_for("anything", 5);
    args.catalog = Some("does-not-exist".to_owned());

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &Scope(WorkspaceName::global()),
        config: &env.config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };

    let err = run_with_deps(args, deps, Mode::Json).expect_err("unknown catalog must error");
    assert!(matches!(err, tome::error::TomeError::CatalogNotFound(_)));
}

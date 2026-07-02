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

use crate::common::{
    TestCatalogConfig, config_with_catalog, copy_sample_plugin_catalog, enrol_catalog_symlinked,
    fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
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
    /// re-reading from disk. Deref coerces to `&Config` where needed.
    config: TestCatalogConfig,
}

fn build_query_env() -> QueryEnv {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // FF1: `lifecycle::enable` resolves the plugin dir from the DB enrolment
    // now, so enrol the catalog + symlink the cache dir onto the on-disk
    // fixture. FF2: `query` filter validation also resolves catalogs from the
    // DB; the in-memory `config` is still built and threaded into the
    // (now-vestigial) `QueryDeps.config` field to avoid churning the deps
    // construction here.
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
                profile: None,
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
            profile: None,
        },
    )
    .unwrap()
}

/// Bootstrap an env whose index is stamped with STUB seeds but has NO plugin
/// enabled — an empty searchable scope. Used to prove the empty-corpus signal
/// (`scope_searchable_count == 0`) that the human empty-state branches on.
fn build_empty_query_env() -> QueryEnv {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // Stamp `meta` with STUB seeds so `run_with_deps`'s drift check passes
    // against the stub-seeded deps below (a registry-seeded meta would hard-fail
    // with EmbedderNameDrift). No catalog enrolled, no plugin enabled → the KNN
    // universe is empty.
    drop(
        index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: stub_embedder_seed(),
                reranker: stub_reranker_seed(),
                summariser: stub_summariser_seed(),
                profile: None,
            },
        )
        .expect("bootstrap empty index meta with stub seeds"),
    );

    // The `config` field is vestigial for validation (resolved from the DB) but
    // must exist; build it before moving `tmp` into the struct.
    let config = config_with_catalog("sample-plugin-catalog", tmp.path());
    QueryEnv {
        _tmp: tmp,
        paths,
        config,
    }
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
        catalogs: vec!["sample-plugin-catalog"],
        plugins: vec!["plugin-beta"],
        ..Default::default()
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
        catalogs: vec!["does-not-exist"],
        ..Default::default()
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
        text: vec![text.to_owned()],
        query: None,
        top_k: Some(top_k),
        catalog: Vec::new(),
        plugin: Vec::new(),
        kind: Vec::new(),
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
fn run_with_deps_human_mode_on_empty_scope_reports_zero_searchable_count() {
    // #293 wiring proof: `run_with_deps` in Mode::Human runs the whole pipeline
    // through `emit_human`, which now consumes `scope_searchable_count`. On an
    // empty scope (no plugin enabled) the pipeline must return zero results AND
    // `scope_searchable_count == 0` — the exact value `emit_human` branches on
    // to print the "No skills indexed for this scope yet" enable-nudge (vs the
    // "No match — rephrase" line when the count is > 0). Deterministic with the
    // stub embedder; the real-binary end-to-end is the `#[ignore]`d
    // `getting_started_plugin_enable_and_query_with_real_models`-class test.
    let env = build_empty_query_env();
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

    // Mode::Human exercises the `emit_human` call site (empty-state branch);
    // it writes to process stdout, which we don't capture — the assertion
    // target is the returned outcome that feeds that branch.
    let outcome = run_with_deps(args_for("anything at all", 5), deps, Mode::Human)
        .expect("empty-scope query must succeed (non-strict) and reach emit_human");

    assert!(
        outcome.results.is_empty(),
        "empty scope must yield zero results, got {}",
        outcome.results.len(),
    );
    assert_eq!(
        outcome.scope_searchable_count, 0,
        "empty scope must report scope_searchable_count == 0 (the empty-corpus signal)",
    );
}

#[test]
fn run_with_deps_populated_scope_reports_nonzero_searchable_count() {
    // The complement of the empty-scope test: a populated scope must report a
    // non-zero `scope_searchable_count`, so the human empty-state (on a genuine
    // no-match) takes the "rephrase" branch, not the "nothing indexed" one.
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

    let outcome = run_with_deps(args_for("anything at all", 5), deps, Mode::Json)
        .expect("populated-scope query must succeed");
    assert!(
        outcome.scope_searchable_count > 0,
        "populated scope must report a non-zero searchable count, got {}",
        outcome.scope_searchable_count,
    );
}

// ---- #304: applied_min_score reflects reality (the knobs-header SSOT) ------
//
// The human-mode effective-knobs header prints `min_score` ONLY when a floor
// actually filtered rows. That floor is `QueryOutcome::applied_min_score`,
// populated by the pipeline: `Some(threshold)` under `--strict`, `None`
// otherwise. These prove the field matches the pipeline's real behaviour so
// the header never advertises a floor that was not applied.

#[test]
fn run_with_deps_non_strict_reports_no_applied_floor() {
    // No `--strict` → the pipeline computes `threshold_passed` but never filters,
    // so no floor is in effect. The header must show `min_score=none`.
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

    let outcome = run_with_deps(args_for("alpha widget", 5), deps, Mode::Json)
        .expect("non-strict query must succeed");
    assert_eq!(
        outcome.applied_min_score, None,
        "non-strict mode applies no floor, so applied_min_score must be None",
    );
}

#[test]
fn run_with_deps_strict_reports_applied_floor() {
    // `--strict` with an explicit `--min-score` applies that exact floor. With
    // cosine self-similarity (~1.0) a 0.5 floor keeps the top hit, so the query
    // succeeds AND reports the applied floor for the header (`min_score=0.5000`).
    let env = build_query_env();
    let embedder = StubEmbedder::new();

    let target_name = "skill-a";
    let target_description = "Well-formed skill that documents how to make alpha widgets shine.";
    let query_text = embedding_text(target_name, target_description, None);

    let mut args = args_for(&query_text, 5);
    args.strict = true;
    args.min_score = Some(0.5);

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &Scope(WorkspaceName::global()),
        config: &env.config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };

    let outcome =
        run_with_deps(args, deps, Mode::Json).expect("strict query above the floor must succeed");
    assert_eq!(
        outcome.applied_min_score,
        Some(0.5),
        "strict mode must report the applied floor (the explicit --min-score)",
    );
    assert!(
        !outcome.results.is_empty(),
        "the self-embedded top hit (~1.0) must pass a 0.5 floor",
    );
}

#[test]
fn run_with_deps_strict_default_floor_is_reported_for_cosine() {
    // `--strict` with NO `--min-score` under cosine scoring applies the mode
    // default (0.5). The header reads THIS resolved value, not a raw flag —
    // proving `applied_min_score` carries the effective (defaulted) floor.
    let env = build_query_env();
    let embedder = StubEmbedder::new();

    let target_name = "skill-a";
    let target_description = "Well-formed skill that documents how to make alpha widgets shine.";
    let query_text = embedding_text(target_name, target_description, None);

    let mut args = args_for(&query_text, 5);
    args.strict = true;
    args.min_score = None; // fall back to the cosine default (0.5)

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &Scope(WorkspaceName::global()),
        config: &env.config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };

    let outcome = run_with_deps(args, deps, Mode::Json)
        .expect("strict query with the default cosine floor must succeed");
    assert_eq!(
        outcome.applied_min_score,
        Some(0.5),
        "strict cosine mode must report the resolved default floor (0.5)",
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

/// The `pipeline` empty-query backstop must surface the SAME rich guidance as
/// the `run` gate (shared `EMPTY_QUERY_USAGE` SSOT), for BOTH a whitespace-only
/// positional and an empty `-q ""` — the latter passes the `run` `None` gate
/// (`Some("")`) and can only be caught by the backstop. A user must never get a
/// terse "query text is empty" from either form.
#[test]
fn run_with_deps_empty_query_uses_the_rich_usage_message() {
    let env = build_query_env();

    // The exact SSOT string both call sites share (mirrors `EMPTY_QUERY_USAGE`).
    const RICH: &str =
        "provide a query: positional words (`tome query reset a counter`), or -q/--query <text>";

    // Case A: whitespace-only positional word.
    {
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
        let err = run_with_deps(args_for("   ", 5), deps, Mode::Json)
            .expect_err("whitespace positional must error");
        match err {
            tome::error::TomeError::Usage(msg) => assert_eq!(msg, RICH, "whitespace positional"),
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    // Case B: `-q ""` (empty single-string form) — reaches only the backstop.
    {
        let embedder = StubEmbedder::new();
        let mut args = args_for("ignored", 5);
        args.text = Vec::new();
        args.query = Some(String::new());
        let deps = QueryDeps {
            paths: &env.paths,
            scope: &Scope(WorkspaceName::global()),
            config: &env.config,
            embedder: &embedder,
            reranker: None,
            embedder_seed: stub_embedder_seed(),
            reranker_seed: stub_reranker_seed(),
        };
        let err = run_with_deps(args, deps, Mode::Json).expect_err("empty -q must error");
        match err {
            tome::error::TomeError::Usage(msg) => assert_eq!(msg, RICH, "empty -q"),
            other => panic!("expected Usage, got {other:?}"),
        }
    }
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
    args.catalog = vec!["sample-plugin-catalog".to_owned()];
    args.plugin = vec!["plugin-beta".to_owned()];

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
    args.catalog = vec!["does-not-exist".to_owned()];

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

// ---- FF2: `--catalog`/`--plugin` filter validation reads the DB ----------
//
// `validate_filters` previously checked `config.catalogs`; on a fresh
// install that map is empty, so any `--catalog`/`--plugin` filter on a
// DB-enrolled catalog failed with `CatalogNotFound`/`PluginNotFound`. These
// drive `run_with_deps` with an EMPTY in-memory `Config` (the `config`
// field is now vestigial for validation) and a catalog enrolled ONLY in the
// DB, proving validation resolves against `workspace_catalogs`.

#[test]
fn run_with_deps_catalog_plugin_filter_validates_against_db_not_config() {
    let env = build_query_env();
    let embedder = StubEmbedder::new();
    let reranker = StubReranker::new();
    let empty_config = Config::default();

    let mut args = args_for("anything", 10);
    args.no_rerank = false;
    args.catalog = vec!["sample-plugin-catalog".to_owned()];
    args.plugin = vec!["plugin-beta".to_owned()];

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &Scope(WorkspaceName::global()),
        // Empty config — validation must NOT depend on it. The catalog is
        // enrolled in the DB by `build_query_env`'s `enrol_catalog_symlinked`.
        config: &empty_config,
        embedder: &embedder,
        reranker: Some(&reranker),
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };

    let outcome = run_with_deps(args, deps, Mode::Json)
        .expect("catalog/plugin filter must validate against the DB enrolment, not config");
    for hit in &outcome.results {
        assert_eq!(hit.candidate.plugin, "plugin-beta");
        assert_eq!(hit.candidate.catalog, "sample-plugin-catalog");
    }
}

#[test]
fn run_with_deps_unknown_catalog_filter_errors_against_db_with_empty_config() {
    let env = build_query_env();
    let embedder = StubEmbedder::new();
    let empty_config = Config::default();

    let mut args = args_for("anything", 5);
    args.catalog = vec!["does-not-exist".to_owned()];

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &Scope(WorkspaceName::global()),
        config: &empty_config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };

    let err = run_with_deps(args, deps, Mode::Json).expect_err("unknown catalog must error");
    assert!(matches!(err, tome::error::TomeError::CatalogNotFound(_)));
}

#[test]
fn run_with_deps_unknown_plugin_filter_errors_against_db_with_empty_config() {
    // Known catalog (DB-enrolled) + unknown plugin → PluginNotFound,
    // message scoped to `<catalog>/<plugin>` when both filters are set.
    let env = build_query_env();
    let embedder = StubEmbedder::new();
    let empty_config = Config::default();

    let mut args = args_for("anything", 5);
    args.catalog = vec!["sample-plugin-catalog".to_owned()];
    args.plugin = vec!["ghost-plugin".to_owned()];

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &Scope(WorkspaceName::global()),
        config: &empty_config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };

    let err = run_with_deps(args, deps, Mode::Json).expect_err("unknown plugin must error");
    match err {
        tome::error::TomeError::PluginNotFound(msg) => {
            assert_eq!(msg, "sample-plugin-catalog/ghost-plugin");
        }
        other => panic!("expected PluginNotFound, got {other:?}"),
    }
}

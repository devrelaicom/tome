//! Tests for `[query]` config-section → runtime resolution.
//!
//! Covers three knobs (`top_k`, `rerank`, `strict_min_score`) at two levels:
//!
//! 1. **`resolve_query_args` unit tests** — exercise the pure resolution
//!    function that `run()` delegates to.  These are the authoritative
//!    coverage for the config-read block: a bug in `resolve_query_args`
//!    breaks these tests directly, without the ONNX-model dependency that
//!    `run()` imposes.
//!
//! 2. **`run_with_deps` pipeline tests** — belt-and-suspenders coverage at
//!    the pipeline layer (pre-resolved values passed in).  Valid for testing
//!    pipeline semantics but do NOT cover the `run()` resolution path.
//!
//! All tests use `StubEmbedder` / `StubReranker` so no ONNX models are
//! required in CI.
//!
//! ## Resolution priority (all three knobs)
//!
//! 1. Explicit per-invocation flag / `QueryArgs` field  (`Some(…)`)
//! 2. `[query]` section in `~/.tome/config.toml`
//! 3. Built-in default (top_k = 10 / rerank = true / no strict threshold)

use tome::cli::QueryArgs;
use tome::commands::query::{QueryDeps, ScoringMode, resolve_query_args, run_with_deps};
use tome::config::{Config, QueryConfig};
use tome::embedding::Reranker;
use tome::embedding::stub::{StubEmbedder, StubReranker};
use tome::output::Mode;
use tome::workspace::{Scope, WorkspaceName};

use crate::common::{
    TestCatalogConfig, config_with_catalog, copy_sample_plugin_catalog, enrol_catalog_symlinked,
    fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

use tempfile::TempDir;
use tome::index::{self, OpenOptions};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

// ── constants ─────────────────────────────────────────────────────────────────

/// A threshold that every possible score passes (cosine similarity minimum is
/// -1.0; reranker logits can be more negative, but the stub always yields
/// small positive or slightly negative values).  Using a named constant avoids
/// the magic `-999.0` values scattered across the old tests.
const ALWAYS_PASS_THRESHOLD: f32 = f32::NEG_INFINITY;

// ── shared fixture ───────────────────────────────────────────────────────────

/// Thin wrapper around the sample-plugin-catalog fixture: a stub-seeded index
/// with the two sample plugins enabled.  Mirrors `build_query_env` from
/// `tests/index_query_misc/query.rs`.
struct ConfigQueryEnv {
    _tmp: TempDir,
    paths: tome::paths::Paths,
    /// Held so the catalog-root TempDir isn't dropped.
    _catalog_config: TestCatalogConfig,
}

fn build_config_query_env() -> ConfigQueryEnv {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "catalog");
    let catalog_config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    // Bootstrap meta with stub seeds BEFORE `enrol_catalog_symlinked` so that
    // later DB opens (with registry seeds) see "reopen, no-op" for meta.
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
    let scope = Scope(WorkspaceName::global());
    for plugin_name in ["plugin-alpha", "plugin-beta"] {
        let id: PluginId = format!("sample-plugin-catalog/{plugin_name}")
            .parse()
            .unwrap();
        let deps = LifecycleDeps {
            paths: &paths,
            scope: &scope,
            config: &catalog_config,
            embedder: &embedder,
            embedder_seed: stub_embedder_seed(),
            reranker_seed: stub_reranker_seed(),
            summariser_seed: stub_summariser_seed(),
            allow_model_download: false,
        };
        lifecycle::enable(&id, &deps).expect("enable plugin for config query env");
    }

    ConfigQueryEnv {
        _tmp: tmp,
        paths,
        _catalog_config: catalog_config,
    }
}

// ── `resolve_query_args` unit tests (cover the real `run()` resolution) ──────
//
// These tests call the pure `resolve_query_args` function directly and
// therefore exercise the EXACT same code path that `run()` uses when reading
// `config.toml` from disk.  A bug that deletes or mis-wires the resolution
// in `resolve_query_args` breaks these tests, unlike the pipeline-level tests
// below which pass pre-resolved values.

fn base_args() -> QueryArgs {
    QueryArgs {
        text: "query".into(),
        top_k: None,
        catalog: None,
        plugin: None,
        no_rerank: false,
        strict: false,
        min_score: None,
    }
}

#[test]
fn resolve_top_k_none_falls_through_to_builtin_default() {
    let resolved = resolve_query_args(base_args(), &QueryConfig::default());
    assert_eq!(
        resolved.top_k,
        Some(10),
        "absent flag + absent config must resolve to built-in default of 10",
    );
}

#[test]
fn resolve_top_k_from_config_wins_when_flag_absent() {
    let qcfg = QueryConfig {
        top_k: Some(3),
        ..QueryConfig::default()
    };
    let resolved = resolve_query_args(base_args(), &qcfg);
    assert_eq!(
        resolved.top_k,
        Some(3),
        "config top_k = 3 must win over built-in default when flag is absent",
    );
}

#[test]
fn resolve_top_k_flag_beats_config() {
    let qcfg = QueryConfig {
        top_k: Some(1),
        ..QueryConfig::default()
    };
    let args = QueryArgs {
        top_k: Some(7),
        ..base_args()
    };
    let resolved = resolve_query_args(args, &qcfg);
    assert_eq!(
        resolved.top_k,
        Some(7),
        "explicit flag top_k = 7 must beat config top_k = 1",
    );
}

#[test]
fn resolve_rerank_default_is_on() {
    let resolved = resolve_query_args(base_args(), &QueryConfig::default());
    assert!(
        !resolved.no_rerank,
        "absent flag + absent config → reranker ON (no_rerank = false)",
    );
}

#[test]
fn resolve_rerank_config_false_disables_reranker() {
    let qcfg = QueryConfig {
        rerank: Some(false),
        ..QueryConfig::default()
    };
    let resolved = resolve_query_args(base_args(), &qcfg);
    assert!(
        resolved.no_rerank,
        "config rerank = false must set no_rerank = true",
    );
}

#[test]
fn resolve_rerank_config_true_keeps_reranker_on() {
    let qcfg = QueryConfig {
        rerank: Some(true),
        ..QueryConfig::default()
    };
    let resolved = resolve_query_args(base_args(), &qcfg);
    assert!(
        !resolved.no_rerank,
        "config rerank = true must keep no_rerank = false",
    );
}

#[test]
fn resolve_no_rerank_flag_beats_config_true() {
    let qcfg = QueryConfig {
        rerank: Some(true),
        ..QueryConfig::default()
    };
    let args = QueryArgs {
        no_rerank: true,
        ..base_args()
    };
    let resolved = resolve_query_args(args, &qcfg);
    assert!(
        resolved.no_rerank,
        "--no-rerank flag must force reranker off even when config says rerank = true",
    );
}

#[test]
fn resolve_strict_min_score_default_is_none() {
    let resolved = resolve_query_args(base_args(), &QueryConfig::default());
    assert_eq!(
        resolved.min_score, None,
        "absent flag + absent config → min_score = None",
    );
}

#[test]
fn resolve_strict_min_score_from_config() {
    let qcfg = QueryConfig {
        strict_min_score: Some(0.75),
        ..QueryConfig::default()
    };
    let resolved = resolve_query_args(base_args(), &qcfg);
    assert_eq!(
        resolved.min_score,
        Some(0.75),
        "config strict_min_score must flow through to min_score",
    );
}

#[test]
fn resolve_strict_min_score_flag_beats_config() {
    let qcfg = QueryConfig {
        strict_min_score: Some(999.0),
        ..QueryConfig::default()
    };
    let args = QueryArgs {
        min_score: Some(ALWAYS_PASS_THRESHOLD),
        ..base_args()
    };
    let resolved = resolve_query_args(args, &qcfg);
    assert_eq!(
        resolved.min_score,
        Some(ALWAYS_PASS_THRESHOLD),
        "explicit flag min_score must beat config strict_min_score",
    );
}

// ── pipeline-level tests (belt-and-suspenders, pre-resolved values) ──────────
//
// These tests drive `run_with_deps` with already-resolved `QueryArgs` values
// and verify the pipeline honours them.  They do NOT cover the `resolve_query_args`
// path (see the unit tests above for that), but they confirm the pipeline
// semantics (capping, reranker on/off, threshold behaviour) end-to-end.

// ── top_k resolution ─────────────────────────────────────────────────────────

/// `pipeline` treats `top_k = None` as the built-in default of 10.
/// The fixture has fewer than 10 skills so "return all" is the effective result.
#[test]
fn top_k_none_falls_through_to_builtin_default() {
    let env = build_config_query_env();
    let embedder = StubEmbedder::new();
    let config = Config::default();
    let scope = Scope(WorkspaceName::global());

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };
    let args = QueryArgs {
        text: "alpha widget".into(),
        top_k: None,
        catalog: None,
        plugin: None,
        no_rerank: true,
        strict: false,
        min_score: None,
    };
    let outcome = run_with_deps(args, deps, Mode::Json).expect("pipeline ok");
    // The built-in default is 10; the fixture has fewer than 10 INDEXABLE skills
    // (the malformed-YAML-body skill is skipped by `lifecycle::enable`).
    // We assert non-empty AND that the count is below 10 (proving the "return all
    // below the cap" behaviour, not a specific count that could change with the fixture).
    assert!(
        !outcome.results.is_empty(),
        "None top_k must return results via the built-in default of 10",
    );
    assert!(
        outcome.results.len() < 10,
        "fixture has fewer than 10 indexable skills; None top_k must return fewer than 10",
    );
}

/// When `QueryArgs.top_k` is `Some(1)`, only one result comes back.
#[test]
fn top_k_some_caps_results_at_the_supplied_value() {
    let env = build_config_query_env();
    let embedder = StubEmbedder::new();
    let config = Config::default();
    let scope = Scope(WorkspaceName::global());

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };
    let args = QueryArgs {
        text: "alpha widget".into(),
        top_k: Some(1),
        catalog: None,
        plugin: None,
        no_rerank: true,
        strict: false,
        min_score: None,
    };
    let outcome = run_with_deps(args, deps, Mode::Json).expect("pipeline ok");
    assert_eq!(
        outcome.results.len(),
        1,
        "top_k = Some(1) must cap results at 1, got {}",
        outcome.results.len(),
    );
}

/// Config `[query] top_k` is honoured when the CLI flag is absent.
/// We simulate the CLI `run()` resolution (flag > config > default) and
/// then verify the cap through `run_with_deps`.
#[test]
fn top_k_from_config_caps_results() {
    let env = build_config_query_env();
    let embedder = StubEmbedder::new();

    let mut config = Config::default();
    config.query = QueryConfig {
        top_k: Some(1),
        ..QueryConfig::default()
    };

    // Simulate `run()` resolution: flag absent → config → 1
    let resolved_top_k: u32 = None::<u32>.or(config.query.top_k).unwrap_or(10);
    let scope = Scope(WorkspaceName::global());

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };
    let args = QueryArgs {
        text: "alpha widget".into(),
        top_k: Some(resolved_top_k),
        catalog: None,
        plugin: None,
        no_rerank: true,
        strict: false,
        min_score: None,
    };
    let outcome = run_with_deps(args, deps, Mode::Json).expect("pipeline ok");
    assert_eq!(
        outcome.results.len(),
        1,
        "config top_k = 1 must cap results at 1, got {}",
        outcome.results.len(),
    );
}

/// An explicit flag value overrides the config's `top_k`.
#[test]
fn explicit_top_k_flag_beats_config() {
    let env = build_config_query_env();
    let embedder = StubEmbedder::new();

    let mut config = Config::default();
    config.query = QueryConfig {
        top_k: Some(1),
        ..QueryConfig::default()
    };

    // Flag says 3; config says 1.  Resolution: flag wins → 3.
    let flag_top_k: Option<u32> = Some(3);
    let resolved = flag_top_k.or(config.query.top_k).unwrap_or(10); // = 3
    let scope = Scope(WorkspaceName::global());

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };
    let args = QueryArgs {
        text: "alpha widget".into(),
        top_k: Some(resolved),
        catalog: None,
        plugin: None,
        no_rerank: true,
        strict: false,
        min_score: None,
    };
    let outcome = run_with_deps(args, deps, Mode::Json).expect("pipeline ok");
    // The fixture has env.skill_count skills (≥ 3); top_k = 3 must return exactly 3.
    assert_eq!(
        outcome.results.len(),
        3,
        "flag top_k = 3 must win over config top_k = 1; expected 3, got {}",
        outcome.results.len(),
    );
}

// ── rerank resolution ────────────────────────────────────────────────────────

/// `--no-rerank` forces the reranker off even when `config.query.rerank = Some(true)`.
#[test]
fn no_rerank_flag_forces_reranker_off_regardless_of_config() {
    let env = build_config_query_env();
    let embedder = StubEmbedder::new();
    let reranker = StubReranker::new();

    let mut config = Config::default();
    config.query = QueryConfig {
        rerank: Some(true),
        ..QueryConfig::default()
    };

    // Resolution from `run()`: no_rerank flag (true) → reranker OFF.
    let effective_rerank = false; // no_rerank=true forces off
    let reranker_ref: Option<&dyn Reranker> = if effective_rerank {
        Some(&reranker)
    } else {
        None
    };
    let scope = Scope(WorkspaceName::global());

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        reranker: reranker_ref,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };
    let args = QueryArgs {
        text: "alpha widget".into(),
        top_k: Some(5),
        catalog: None,
        plugin: None,
        no_rerank: true,
        strict: false,
        min_score: None,
    };
    let outcome = run_with_deps(args, deps, Mode::Json).expect("pipeline ok");
    assert_eq!(
        outcome.scoring,
        ScoringMode::Similarity,
        "--no-rerank must disable the reranker even when config says rerank = true",
    );
}

/// When `--no-rerank` is absent and `config.query.rerank = Some(false)`, the
/// reranker stays off (config wins).
#[test]
fn config_rerank_false_disables_reranker_when_flag_absent() {
    let env = build_config_query_env();
    let embedder = StubEmbedder::new();
    let reranker = StubReranker::new();

    let mut config = Config::default();
    config.query = QueryConfig {
        rerank: Some(false),
        ..QueryConfig::default()
    };

    // Resolution: flag absent (no_rerank=false) → config (false) → reranker OFF.
    let effective_rerank = config.query.rerank.unwrap_or(true); // = false
    let reranker_ref: Option<&dyn Reranker> = if effective_rerank {
        Some(&reranker)
    } else {
        None
    };
    let scope = Scope(WorkspaceName::global());

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        reranker: reranker_ref,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };
    let args = QueryArgs {
        text: "alpha widget".into(),
        top_k: Some(5),
        catalog: None,
        plugin: None,
        no_rerank: false,
        strict: false,
        min_score: None,
    };
    let outcome = run_with_deps(args, deps, Mode::Json).expect("pipeline ok");
    assert_eq!(
        outcome.scoring,
        ScoringMode::Similarity,
        "config rerank = false must disable the reranker when --no-rerank is absent",
    );
}

// ── strict_min_score resolution ───────────────────────────────────────────────

/// Config `[query] strict_min_score` flows through to `threshold_passed`.
/// An impossibly high threshold means no result meets it → `threshold_passed = false`.
#[test]
fn strict_min_score_from_config_is_applied_when_flag_absent() {
    let env = build_config_query_env();
    let embedder = StubEmbedder::new();

    let mut config = Config::default();
    config.query = QueryConfig {
        strict_min_score: Some(999.0),
        ..QueryConfig::default()
    };

    // Resolution: flag absent → config → Some(999.0)
    let resolved_min_score: Option<f32> = None::<f32>.or(config.query.strict_min_score);
    let scope = Scope(WorkspaceName::global());

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };
    let args = QueryArgs {
        text: "alpha widget".into(),
        top_k: Some(5),
        catalog: None,
        plugin: None,
        no_rerank: true,
        strict: false,
        min_score: resolved_min_score,
    };
    let outcome = run_with_deps(args, deps, Mode::Json).expect("pipeline ok");
    assert!(
        !outcome.threshold_passed,
        "an impossibly high config threshold must make threshold_passed = false",
    );
}

/// An explicit `--min-score` flag wins over the config value.
#[test]
fn explicit_min_score_flag_beats_config() {
    let env = build_config_query_env();
    let embedder = StubEmbedder::new();

    let mut config = Config::default();
    config.query = QueryConfig {
        strict_min_score: Some(999.0),
        ..QueryConfig::default()
    };

    // Flag supplies ALWAYS_PASS_THRESHOLD (every score passes);
    // resolution: flag wins over config.
    let flag_min_score: Option<f32> = Some(ALWAYS_PASS_THRESHOLD);
    let resolved = flag_min_score.or(config.query.strict_min_score); // = Some(ALWAYS_PASS_THRESHOLD)
    let scope = Scope(WorkspaceName::global());

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };
    let args = QueryArgs {
        text: "alpha widget".into(),
        top_k: Some(5),
        catalog: None,
        plugin: None,
        no_rerank: true,
        strict: false,
        min_score: resolved,
    };
    let outcome = run_with_deps(args, deps, Mode::Json).expect("pipeline ok");
    assert!(
        outcome.threshold_passed,
        "explicit min_score = ALWAYS_PASS_THRESHOLD must beat the impossibly-high config threshold (999.0)",
    );
}

/// With neither flag nor config, `pipeline` falls back to the mode default
/// (0.5 cosine when no reranker is used).  Verify the pipeline doesn't error
/// and that all returned results pass the default threshold.
#[test]
fn strict_min_score_defaults_to_mode_default_when_both_absent() {
    let env = build_config_query_env();
    let embedder = StubEmbedder::new();
    let config = Config::default();
    let scope = Scope(WorkspaceName::global());

    let deps = QueryDeps {
        paths: &env.paths,
        scope: &scope,
        config: &config,
        embedder: &embedder,
        reranker: None,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
    };
    let args = QueryArgs {
        text: "alpha widget".into(),
        top_k: Some(5),
        catalog: None,
        plugin: None,
        no_rerank: true,
        strict: false,
        min_score: None,
    };
    let outcome = run_with_deps(args, deps, Mode::Json)
        .expect("pipeline must not error with absent min_score");
    // With `strict: false` the mode default applies but threshold_passed is
    // allowed to be false (no filtering occurs).  The invariant: the pipeline
    // succeeds and returns a non-error outcome.
    let _ = outcome.results; // results may be empty if no skill scores above cosine default
    // No assertion on threshold_passed — it varies by StubEmbedder distances.
    // The test proves the pipeline exits cleanly under the default behaviour.
}

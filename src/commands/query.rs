//! `tome query <text>` — KNN search over enabled skills with an optional
//! cross-encoder reranker stage.
//!
//! Slice 2 of Phase 3 (User Story 1). The lifecycle and `tome plugin …`
//! commands shipped in slice 1; this slice composes the read-only side:
//! open the index, embed the query, KNN, optionally rerank, render.
//!
//! Spec: `contracts/query.md`. No model download is offered here — the user
//! should have installed models via `tome plugin enable` first, where the
//! TTY prompt belongs. Query is meant to be fast; surfacing a multi-MB
//! download behind a `tome query` is hostile UX.

use std::io::Write;
use std::path::PathBuf;

use comfy_table::{Cell, CellAlignment};
use serde::Serialize;

use crate::cli::QueryArgs;
use crate::config::Config;
use crate::embedding::fastembed::{FastembedEmbedder, FastembedReranker};
use crate::embedding::{Embedder, Reranker, Scored};
use crate::error::TomeError;
use crate::index::meta::{self, DriftStatus, ModelIdent};
use crate::index::query::{QueryFilters, knn};
use crate::index::schema::MetaSeed;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::presentation::{colour, progress, tables};
use crate::workspace::{ResolvedScope, Scope, ScopeSource};

use super::plugin::{
    embedder_entry, missing_models, open_index_for_read, read_catalog_manifest, registry_seeds,
    reranker_entry,
};

/// Either the reranker's raw logit ("reranked") or 1.0 − cosine distance
/// ("embedding-similarity"). The string is duplicated at the top level and
/// per result in the JSON envelope so consumers can pick either form.
const SCORING_RERANKED: &str = "reranked";
const SCORING_SIMILARITY: &str = "embedding-similarity";

/// Inputs to [`run_with_deps`] — pre-built handles + scope context the
/// caller has already paid for. Mirrors `LifecycleDeps` from the lifecycle
/// module so tests can inject `StubEmbedder` / `StubReranker` instead of
/// loading the multi-MB `FastembedEmbedder` / `FastembedReranker`.
pub struct QueryDeps<'a> {
    pub paths: &'a Paths,
    pub scope: &'a Scope,
    /// Vestigial since FF2 moved `--catalog`/`--plugin` validation onto the
    /// `workspace_catalogs` DB — nothing in the query pipeline reads this.
    /// Retained as a field to avoid churning the test construction sites in
    /// this bug-fix slice; callers may pass `Config::default()`.
    pub config: &'a Config,
    pub embedder: &'a dyn Embedder,
    pub reranker: Option<&'a dyn Reranker>,
    /// Identity recorded by the embedder/reranker the caller loaded.
    /// Drift detection compares this against the on-disk `meta` rows; in
    /// the CLI path it comes from `registry_seeds()`, but tests can pass
    /// stub seeds to keep `StubEmbedder` consistent with the bootstrap.
    pub embedder_seed: MetaSeed,
    pub reranker_seed: MetaSeed,
}

/// Result of one `run_with_deps` invocation. Returned for the test path;
/// the CLI path also emits to stdout/stderr per `mode` as a side effect
/// before returning.
#[derive(Debug, Clone)]
pub struct QueryOutcome {
    pub results: Vec<Scored>,
    pub scoring: ScoringMode,
    /// Whether every returned row meets `min_score` (or the default for
    /// the scoring mode in use). Always `true` after `--strict` filtering.
    pub threshold_passed: bool,
    pub reranker_drift: Option<String>,
    /// Total searchable corpus size (enabled skill embeddings in scope), used
    /// only for the bucketed `tome.search.corpus_size_bucket` telemetry field.
    /// Best-effort: a count failure yields `0` rather than aborting the query.
    pub corpus_size: u64,
}

/// Scoring source for a `QueryOutcome`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoringMode {
    Reranked,
    Similarity,
}

impl ScoringMode {
    fn as_str(self) -> &'static str {
        match self {
            ScoringMode::Reranked => SCORING_RERANKED,
            ScoringMode::Similarity => SCORING_SIMILARITY,
        }
    }
}

pub fn run(args: QueryArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    // FF2: `validate_filters` resolves catalogs from the `workspace_catalogs`
    // DB now, so the command no longer reads `config.toml [catalogs]`. The
    // vestigial `QueryDeps.config` field is populated with an empty default.
    let config = Config::default();

    // Model presence — embedder always required, reranker required unless
    // `--no-rerank`. We check before constructing the heavy
    // `FastembedEmbedder` so a missing-model error doesn't pay the load
    // cost first.
    let embedder_meta = embedder_entry();
    let missing = missing_models(&paths);
    if missing.iter().any(|e| e.name == embedder_meta.name) {
        return Err(TomeError::ModelMissing {
            model: embedder_meta.name.to_owned(),
        });
    }
    let reranker_meta = reranker_entry();
    if !args.no_rerank && missing.iter().any(|e| e.name == reranker_meta.name) {
        return Err(TomeError::ModelMissing {
            model: reranker_meta.name.to_owned(),
        });
    }

    // Load models. Wrap each in a spinner; non-TTY stderr → hidden draw
    // target (see presentation::progress::target).
    let embedder = {
        let pb = progress::spinner(format!("loading embedder ({})", embedder_meta.name));
        let result = FastembedEmbedder::load(embedder_meta, &paths.model_path(embedder_meta.name)?);
        pb.finish_and_clear();
        result?
    };
    let reranker_loaded: Option<FastembedReranker> = if args.no_rerank {
        None
    } else {
        let pb = progress::spinner(format!("loading reranker ({})", reranker_meta.name));
        let result = FastembedReranker::load(reranker_meta, &paths.model_path(reranker_meta.name)?);
        pb.finish_and_clear();
        Some(result?)
    };
    let reranker: Option<&dyn Reranker> = reranker_loaded.as_ref().map(|r| r as &dyn Reranker);

    let (embedder_seed, reranker_seed, _summariser_seed) = registry_seeds();
    let deps = QueryDeps {
        paths: &paths,
        scope: &scope.scope,
        config: &config,
        embedder: &embedder,
        reranker,
        embedder_seed,
        reranker_seed,
    };

    run_with_deps(args, deps, mode).map(|_| ())
}

/// Library entry point. Accepts pre-built embedder/reranker handles; runs
/// the full pipeline (filter validation → drift check → KNN → rerank →
/// threshold → emit) and returns the structured outcome.
///
/// The CLI path constructs `FastembedEmbedder` + `FastembedReranker` and
/// hands them in. Tests pass `StubEmbedder` / `StubReranker` along with
/// the matching `MetaSeed`s — keeping drift detection consistent without
/// requiring on-disk ONNX models.
///
/// Phase 3 / Foundational slice F6 — closes the Phase 10 deferred item.
pub fn run_with_deps(
    args: QueryArgs,
    deps: QueryDeps<'_>,
    mode: Mode,
) -> Result<QueryOutcome, TomeError> {
    // Measure latency around the COMPUTE boundary ONLY (FR-027a) — the silent
    // `pipeline` call — so the bucketed `latency_bucket` excludes all emit and
    // telemetry overhead. The raw duration never leaves this scope; only its
    // bucket is reported.
    let reranker_used = deps.reranker.is_some();
    let started = std::time::Instant::now();
    let outcome = pipeline(&args, &deps)?;
    let elapsed = started.elapsed();

    // FR-027: `tome.search` fires on a successful query (CLI surface). On an
    // error the pipeline returns `Err` and the app-boundary `tome.error` covers
    // it, so we do NOT reach here on failure (no double-emit). Best-effort
    // enqueue — never blocks or alters the result.
    crate::telemetry::enqueue(crate::telemetry::event::Search {
        surface: crate::telemetry::event::Surface::Cli,
        latency_bucket: crate::telemetry::buckets::LatencyBucket::from(elapsed),
        candidates_returned: crate::telemetry::buckets::CountBucket::from(outcome.results.len()),
        reranker_used,
        strict: args.strict,
        corpus_size_bucket: crate::telemetry::buckets::CountBucket::from(outcome.corpus_size),
        // The embedder identity is the pinned registry entry — a `&'static str`
        // from the closed `MODEL_REGISTRY`, never a free-form string.
        embedder_model_id: Some(embedder_entry().name),
        // CLI surface has no calling harness (that's an MCP-only dimension).
        calling_harness: None,
    });

    // Co-H1 / FR-052 + FR-057: ALONGSIDE the anonymous `tome.search`, emit one
    // catalog-attributed `catalog.<id>.search_result` per result entry whose
    // catalog resolves — by SOURCE, at emit time — to an allowlisted catalog.
    // Mirrors the MCP `search_skills` path (the divergence is `calling_harness:
    // None` — the CLI has no host harness). `rank` is the EXACT 1-indexed
    // position in the returned (already top-k, reranked) list, NOT bucketed
    // (FR-057). Attribution is memoised per catalog NAME so a result set spanning
    // several catalogs opens the read-only index at most once per distinct
    // catalog (NFR-009 — read-only, no advisory lock; fine inline on this sync
    // CLI path). Best-effort: a `None` resolution ⇒ anonymous only; never alters
    // the result or fails the query.
    //
    // `resolve_attribution` reads only `scope.scope.name()`; wrap the `&Scope`
    // we hold in a throwaway `ResolvedScope` (provenance/project_root are unused
    // by the attribution read) rather than thread a `ResolvedScope` through the
    // whole dep struct.
    let attribution_scope = ResolvedScope {
        scope: deps.scope.clone(),
        source: ScopeSource::GlobalFallback,
        project_root: None,
    };
    let mut attribution_cache: std::collections::HashMap<String, Option<&'static str>> =
        std::collections::HashMap::new();
    for (idx, s) in outcome.results.iter().enumerate() {
        let c = &s.candidate;
        let catalog_id = *attribution_cache
            .entry(c.catalog.clone())
            .or_insert_with(|| {
                crate::telemetry::resolve_attribution(&attribution_scope, &c.catalog)
            });
        if let Some(catalog_id) = catalog_id {
            crate::telemetry::enqueue_attributed(crate::telemetry::event::SearchResult {
                entry_name: c.name.clone(),
                entry_kind: c.kind.into(),
                plugin_name: c.plugin.clone(),
                // EXACT 1-indexed rank (FR-057) — `idx + 1`, never bucketed.
                rank: (idx + 1) as u32,
                catalog_id,
                // CLI surface has no calling harness.
                calling_harness: None,
            });
        }
    }

    let home = std::env::var_os("HOME").map(PathBuf::from);
    match mode {
        Mode::Human => emit_human(
            &outcome.results,
            outcome.scoring.as_str(),
            outcome.reranker_drift.as_deref(),
            home.as_deref(),
        )?,
        Mode::Json => emit_json(
            &outcome.results,
            outcome.scoring.as_str(),
            outcome.threshold_passed,
            outcome.reranker_drift.as_deref(),
        )?,
    }
    Ok(outcome)
}

/// The silent compute path. Runs filter validation → drift check →
/// embed → KNN → optional rerank → trim → threshold check, and
/// returns the [`QueryOutcome`] without emitting any stdout/stderr.
///
/// Phase 3 / US1.b uses this from the MCP `search_skills` handler:
/// the protocol channel is sacred (FR-221) so the CLI emit step
/// would corrupt the transport.
///
/// CLI callers go through [`run_with_deps`] which calls this then
/// emits per `mode`.
pub fn pipeline(args: &QueryArgs, deps: &QueryDeps<'_>) -> Result<QueryOutcome, TomeError> {
    let text = args.text.trim();
    if text.is_empty() {
        return Err(TomeError::Usage("query text is empty".into()));
    }

    let conn = open_index_for_read(deps.paths, deps.scope)?;

    // Validate filter flags before any model work — a cheap DB enrolment
    // lookup plus at most one catalog-manifest read per catalog, failing
    // fast on typos. FF2: catalog existence is resolved from
    // `workspace_catalogs`, not `config.toml` (never written in production).
    validate_filters(args, &conn, deps.scope.name().as_str(), deps.paths)?;

    // Drift detection. Embedder drift hard-fails (vectors are stale);
    // reranker drift only degrades quality, so we keep the value and
    // surface it as a warning later. The seeds carried in `deps` are the
    // identities the *caller* has loaded — drift fires when they disagree
    // with the on-disk `meta` rows.
    let reranker_drift = check_drift(&conn, &deps.embedder_seed, &deps.reranker_seed)?;

    // Embed the query text as-is — FR-014's name/description composition
    // applies only to skill ingestion.
    let query_vec = deps.embedder.embed(text)?;

    // Pull candidates. Reranking benefits from a wider pool — 4× per the
    // contract — and we trim back after.
    let candidate_k: u32 = if deps.reranker.is_some() {
        args.top_k.saturating_mul(4).max(args.top_k)
    } else {
        args.top_k
    };
    let filters = QueryFilters {
        catalog: args.catalog.as_deref(),
        plugin: args.plugin.as_deref(),
    };
    let candidates = knn(
        &conn,
        deps.scope.name().as_str(),
        &query_vec,
        candidate_k,
        &filters,
    )?;

    // Score + sort. With a reranker, scores come from the cross-encoder;
    // without, we treat 1.0 − distance as cosine similarity.
    let scored: Vec<Scored> = match deps.reranker {
        Some(r) => r.rerank(text, candidates)?,
        None => {
            let mut s: Vec<Scored> = candidates
                .into_iter()
                .map(|c| Scored {
                    score: 1.0 - c.distance,
                    candidate: c,
                })
                .collect();
            s.sort_by(|a, b| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            s
        }
    };

    let top_k = args.top_k as usize;
    let mut trimmed: Vec<Scored> = scored.into_iter().take(top_k).collect();

    // Default threshold depends on the scoring mode. The contract distinguishes
    // reranker logits (default 0.0) from cosine similarity (default 0.5).
    let default_threshold = if deps.reranker.is_some() {
        0.0_f32
    } else {
        0.5_f32
    };
    let threshold = args.min_score.unwrap_or(default_threshold);

    if args.strict {
        trimmed.retain(|s| s.score >= threshold);
        if trimmed.is_empty() {
            return Err(TomeError::QueryNoResultsStrict { threshold });
        }
    }

    // Even without `--strict`, the JSON `threshold_passed` field reflects
    // whether every returned row meets the (possibly default) threshold.
    let threshold_passed = trimmed.iter().all(|s| s.score >= threshold);
    let scoring = if deps.reranker.is_some() {
        ScoringMode::Reranked
    } else {
        ScoringMode::Similarity
    };

    // Total embeddings count for the bucketed telemetry `corpus_size_bucket`.
    // Best-effort: a count failure must not fail the (already-computed) query —
    // we fall back to 0 (which buckets to `0`). This is the whole-index count,
    // not the in-scope filtered count: the latter would require re-running the
    // filtered KNN universe, and a coarse 5-bucket signal does not warrant it.
    let corpus_size: u64 = conn
        .query_row("SELECT COUNT(*) FROM skill_embeddings", [], |r| {
            r.get::<_, i64>(0)
        })
        .ok()
        .map(|n: i64| n.max(0) as u64)
        .unwrap_or(0);

    Ok(QueryOutcome {
        results: trimmed,
        scoring,
        threshold_passed,
        reranker_drift,
        corpus_size,
    })
}

/// Validate `--catalog` / `--plugin` against the `workspace_catalogs` DB
/// enrolment + the on-disk catalog manifests.
///
/// FF2: catalog existence is resolved from the DB (`config.toml [catalogs]`
/// is never written in production, so reading it failed every filter on a
/// fresh install). Costs one enrolment lookup plus at most one TOML parse
/// per enrolled catalog when a `--plugin` filter is set — bounded and cheap
/// relative to the query itself. The `<catalog>/<plugin>` vs bare error
/// message semantics are preserved.
fn validate_filters(
    args: &QueryArgs,
    conn: &rusqlite::Connection,
    workspace_name: &str,
    paths: &Paths,
) -> Result<(), TomeError> {
    use crate::index::workspace_catalogs;

    if let Some(catalog) = args.catalog.as_deref()
        && workspace_catalogs::find(conn, workspace_name, catalog)?.is_none()
    {
        return Err(TomeError::CatalogNotFound(catalog.to_owned()));
    }

    let Some(plugin) = args.plugin.as_deref() else {
        return Ok(());
    };

    // Resolve the set of enrolments to scan: the one named catalog (already
    // confirmed to exist above), or every enrolment in the workspace when no
    // `--catalog` was given.
    let enrolments = match args.catalog.as_deref() {
        Some(c) => workspace_catalogs::find(conn, workspace_name, c)?
            .into_iter()
            .collect(),
        None => workspace_catalogs::list_for_workspace(conn, workspace_name)?,
    };

    for enrolment in &enrolments {
        let clone_dir = paths.cache_dir_for(&enrolment.url);
        let Some(manifest) = read_catalog_manifest(&clone_dir) else {
            continue;
        };
        if manifest.plugins.iter().any(|p| p.name == plugin) {
            return Ok(());
        }
    }

    // Scope the error message: when both filters were given, the
    // `<catalog>/<plugin>` form is the precise identity. Otherwise the bare
    // plugin name is enough — there is no catalog scope to attach.
    let message = match args.catalog.as_deref() {
        Some(c) => format!("{c}/{plugin}"),
        None => plugin.to_owned(),
    };
    Err(TomeError::PluginNotFound(message))
}

/// Run drift detection. Embedder drift converts to a hard error; reranker
/// drift returns `Ok(Some(label))` for the caller to surface.
///
/// The configured identities come from the deps (`run_with_deps`) so
/// tests using `StubEmbedder` / `StubReranker` can pass their own seeds
/// and not trip false drift against the BGE registry constants.
fn check_drift(
    conn: &rusqlite::Connection,
    embedder_seed: &MetaSeed,
    reranker_seed: &MetaSeed,
) -> Result<Option<String>, TomeError> {
    let embedder_ident = ModelIdent {
        name: embedder_seed.name.clone(),
        version: embedder_seed.version.clone(),
    };
    let reranker_ident = ModelIdent {
        name: reranker_seed.name.clone(),
        version: reranker_seed.version.clone(),
    };
    // Phase 4 / F9: summariser identity is recorded in the index but
    // never affects query correctness. We pass the configured registry
    // identity so the drift check stays consistent with bootstrap; any
    // drift surfaced here is a transient observability signal, not a
    // failure.
    let summariser_entry = crate::summarise::registry::summariser_entry();
    let summariser_ident = ModelIdent {
        name: summariser_entry.name.to_owned(),
        version: summariser_entry.version.to_owned(),
    };
    match meta::detect_drift(conn, &embedder_ident, &reranker_ident, &summariser_ident)? {
        DriftStatus::None | DriftStatus::SummariserDrift { .. } => Ok(None),
        DriftStatus::EmbedderNameDrift { stored, configured } => {
            Err(TomeError::EmbedderNameDrift { stored, configured })
        }
        DriftStatus::EmbedderVersionDrift { stored, configured } => {
            Err(TomeError::EmbedderVersionDrift { stored, configured })
        }
        DriftStatus::RerankerDrift { stored, configured } => {
            Ok(Some(format!("stored={stored}, configured={configured}")))
        }
    }
}

fn emit_human(
    results: &[Scored],
    scoring: &str,
    reranker_drift: Option<&str>,
    home: Option<&std::path::Path>,
) -> Result<(), TomeError> {
    // Stderr-only notices first so structured stdout stays clean even when
    // a banner / warning is rendered.
    if scoring == SCORING_SIMILARITY {
        let mut err = std::io::stderr().lock();
        let _ = writeln!(err, "(reranker disabled — showing embedding similarity)");
    }
    if let Some(drift) = reranker_drift {
        let mut err = std::io::stderr().lock();
        let _ = writeln!(
            err,
            "{} reranker drift detected — {drift}",
            colour::warning("warning:")
        );
    }

    let mut out = std::io::stdout().lock();
    if results.is_empty() {
        writeln!(out, "No results.")?;
        return Ok(());
    }

    let mut table = tables::new_table();
    table.set_header(vec![
        Cell::new("Score").set_alignment(CellAlignment::Right),
        Cell::new("Catalog"),
        Cell::new("Plugin"),
        Cell::new("Skill"),
        Cell::new("Version"),
        Cell::new("Path"),
    ]);

    for s in results {
        let c = &s.candidate;
        table.add_row(vec![
            Cell::new(format_score(s.score)).set_alignment(CellAlignment::Right),
            Cell::new(&c.catalog),
            Cell::new(&c.plugin),
            Cell::new(&c.name),
            Cell::new(&c.plugin_version),
            Cell::new(shorten_home(&c.path, home)),
        ]);
    }

    writeln!(out, "{table}")?;
    Ok(())
}

fn emit_json(
    results: &[Scored],
    scoring: &'static str,
    threshold_passed: bool,
    reranker_drift: Option<&str>,
) -> Result<(), TomeError> {
    let rows: Vec<JsonResult<'_>> = results
        .iter()
        .map(|s| JsonResult {
            catalog: &s.candidate.catalog,
            plugin: &s.candidate.plugin,
            skill: &s.candidate.name,
            plugin_version: &s.candidate.plugin_version,
            score: s.score,
            // JSON keeps the full path — the contract spells this out
            // explicitly. The `~` shorthand is human-mode only.
            path: &s.candidate.path,
            scoring,
        })
        .collect();

    let env = JsonEnvelope {
        scoring,
        threshold_passed,
        results: rows,
        reranker_drift,
    };
    output::write_json(&env)
}

#[derive(Serialize)]
struct JsonEnvelope<'a> {
    scoring: &'a str,
    threshold_passed: bool,
    results: Vec<JsonResult<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reranker_drift: Option<&'a str>,
}

#[derive(Serialize)]
struct JsonResult<'a> {
    catalog: &'a str,
    plugin: &'a str,
    skill: &'a str,
    plugin_version: &'a str,
    score: f32,
    path: &'a str,
    scoring: &'a str,
}

/// Format a score with four decimals. Reranker logits can be negative or
/// exceed 1; similarity scores live in `[-1, 1]`. The contract example
/// shows `{:.4}` so we stay consistent regardless.
fn format_score(score: f32) -> String {
    format!("{score:.4}")
}

/// Replace `$HOME` with `~` when `path` is under the user's home directory.
/// Falls back to the path verbatim on any non-prefix or missing-home case.
/// Inline so we avoid pulling in a new crate; the rule is the standard
/// shell shorthand.
fn shorten_home(path: &str, home: Option<&std::path::Path>) -> String {
    let Some(home) = home else {
        return path.to_owned();
    };
    let home_str = home.to_string_lossy();
    if home_str.is_empty() {
        return path.to_owned();
    }
    if let Some(rest) = path.strip_prefix(home_str.as_ref())
        && (rest.starts_with('/') || rest.is_empty())
    {
        return format!("~{rest}");
    }
    path.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_score_uses_four_decimals_for_positive() {
        assert_eq!(format_score(3.123_456_7), "3.1235");
    }

    #[test]
    fn format_score_uses_four_decimals_for_negative_logits() {
        // Reranker logits can be < 0; padding still applies.
        assert_eq!(format_score(-1.2), "-1.2000");
    }

    #[test]
    fn format_score_handles_zero() {
        assert_eq!(format_score(0.0), "0.0000");
    }

    #[test]
    fn shorten_home_replaces_prefix() {
        let home = std::path::Path::new("/Users/alice");
        let got = shorten_home("/Users/alice/.local/share/tome/foo/SKILL.md", Some(home));
        assert_eq!(got, "~/.local/share/tome/foo/SKILL.md");
    }

    #[test]
    fn shorten_home_leaves_unrelated_path_alone() {
        let home = std::path::Path::new("/Users/alice");
        let got = shorten_home("/etc/hosts", Some(home));
        assert_eq!(got, "/etc/hosts");
    }

    #[test]
    fn shorten_home_only_replaces_at_boundary() {
        // `/Users/alice-other` must NOT be shortened to `~-other`.
        let home = std::path::Path::new("/Users/alice");
        let got = shorten_home("/Users/alice-other/foo", Some(home));
        assert_eq!(got, "/Users/alice-other/foo");
    }

    #[test]
    fn shorten_home_handles_exact_home() {
        let home = std::path::Path::new("/Users/alice");
        let got = shorten_home("/Users/alice", Some(home));
        assert_eq!(got, "~");
    }

    #[test]
    fn shorten_home_returns_input_when_home_unset() {
        let got = shorten_home("/Users/alice/foo", None);
        assert_eq!(got, "/Users/alice/foo");
    }
}

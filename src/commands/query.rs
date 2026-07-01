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
use crate::embedding::{Embedder, Reranker, Scored};
use crate::error::TomeError;
use crate::index::meta::{self, DriftStatus, ModelIdent};
use crate::index::query::{QueryFilters, knn};
use crate::index::schema::MetaSeed;
use crate::output::{self, Mode};
use crate::paths::Paths;
use crate::presentation::{colour, progress, tables};
use crate::workspace::{ResolvedScope, Scope, ScopeSource};

use super::plugin::{missing_models, open_index_for_read, read_catalog_manifest};

/// Either the reranker's raw logit ("reranked") or 1.0 − cosine distance
/// ("embedding-similarity"). The string is duplicated at the top level and
/// per result in the JSON envelope so consumers can pick either form.
const SCORING_RERANKED: &str = "reranked";
const SCORING_SIMILARITY: &str = "embedding-similarity";

/// Built-in default result cap when neither `--top-k` nor `[query] top_k` is
/// set. The single source of truth for this default — the CLI resolver, the MCP
/// `search_skills` tool, and `tome config show` all reference it so a change
/// here can't drift the shown default away from the effective one.
pub const DEFAULT_TOP_K: u32 = 10;

/// Built-in default for whether the reranker stage runs when neither
/// `--no-rerank` nor `[query] rerank` is set. Single source of truth for the
/// shown default (`tome config show`).
pub const DEFAULT_RERANK: bool = true;

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
    /// WHOLE-INDEX embedding count (every workspace, ignoring `searchable`),
    /// used ONLY for the bucketed `tome.search.corpus_size_bucket` telemetry
    /// field. Best-effort: a count failure yields `0` rather than aborting the
    /// query. NOT the universe the KNN searches — see
    /// [`Self::scope_searchable_count`] for that.
    pub corpus_size: u64,
    /// SCOPE-EFFECTIVE searchable count: the enabled, `searchable = 1` skills
    /// joined into the resolved workspace — i.e. exactly the universe the KNN
    /// searches (minus the vector distance / LIMIT). #285: the MCP
    /// `search_skills` empty-result signal branches on THIS (not the
    /// whole-index `corpus_size`) so `== 0` ⇔ "index empty for this scope →
    /// reindex" and `> 0` ⇔ "no semantic match → rephrase". Best-effort: a
    /// count failure yields `0`.
    pub scope_searchable_count: u64,
}

/// Scoring source for a `QueryOutcome`.
///
/// The `as_str` values are the SSOT for both the CLI JSON envelope and the
/// MCP `search_skills` output — a caller reads `scoring` to know whether
/// `score` is a reranker logit (`"reranked"`) or `1.0 − cosine distance`
/// (`"embedding-similarity"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScoringMode {
    Reranked,
    Similarity,
}

impl ScoringMode {
    /// The canonical scoring-mode string (`"reranked"` |
    /// `"embedding-similarity"`). Reused verbatim by the MCP `search_skills`
    /// tool so the CLI and MCP surfaces never diverge on the wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            ScoringMode::Reranked => SCORING_RERANKED,
            ScoringMode::Similarity => SCORING_SIMILARITY,
        }
    }
}

/// Resolve per-invocation query knobs: explicit flag → `[query]` config →
/// built-in default.  Pure function so it can be tested independently of the
/// `run()` path (which requires real ONNX models to be present on disk).
///
/// Priority (highest to lowest):
/// 1. Explicit per-call flag (`args.top_k` / `args.no_rerank` / `args.min_score`).
/// 2. `[query]` section in `~/.tome/config.toml` (passed as `qcfg`).
/// 3. Built-in default (`top_k = 10`, `rerank = true`, `strict_min_score = None`).
///
/// `no_rerank` semantics: if the caller explicitly passed `--no-rerank` that
/// wins unconditionally.  Otherwise the *config* decides (`rerank = false` →
/// reranker off; default or `rerank = true` → reranker on).
pub fn resolve_query_args(args: QueryArgs, qcfg: &crate::config::QueryConfig) -> QueryArgs {
    let effective_rerank = if args.no_rerank {
        false
    } else {
        qcfg.rerank.unwrap_or(DEFAULT_RERANK)
    };
    let effective_top_k: u32 = args.top_k.or(qcfg.top_k).unwrap_or(DEFAULT_TOP_K);
    QueryArgs {
        top_k: Some(effective_top_k),
        no_rerank: !effective_rerank,
        min_score: args.min_score.or(qcfg.strict_min_score),
        ..args
    }
}

pub fn run(args: QueryArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    // Strict load of the global config so a malformed `config.toml` surfaces
    // as exit 5 rather than silently falling through to defaults. The vestigial
    // `QueryDeps.config` field still receives a `Config::default()` (FF2).
    let cfg = crate::config::load(&paths)?;

    // Resolve per-invocation knobs: flag > config > built-in default.
    // We compute this BEFORE the model-presence check so `--no-rerank`
    // prevents a hard-fail on a missing reranker model when the flag is
    // explicitly passed.
    let args = resolve_query_args(args, &cfg.query);

    let config = Config::default();

    // B4: resolve the ACTIVE profile's embedder + reranker. Open the index
    // read-only when present; on a fresh install (no DB) fall back to the
    // default profile, which the bootstrap will stamp. `missing_models` walks
    // the whole registry but we only ever name-match the two resolved entries,
    // so the query path is already profile-safe (it never demands a model the
    // active profile doesn't use).
    let (embedder_meta, reranker_meta) = if paths.index_db.is_file() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        (
            crate::index::meta::active_embedder(&conn)?,
            crate::index::meta::active_reranker(&conn)?,
        )
    } else {
        use crate::embedding::profile::{Profile, embedder_for, reranker_for};
        (
            embedder_for(Profile::DEFAULT),
            reranker_for(Profile::DEFAULT),
        )
    };

    // Phase 12 / US2: is an `[embedding]` provider configured? When so, the
    // query embeds remotely (no local embedder model required) and drift fires
    // against the remote identity. `resolve` failures (a bad reference) are a
    // 93 surfaced here, the same as any other command.
    let remote_embedding =
        crate::provider::resolve(&cfg, crate::provider::Capability::Embedding)?.is_some();

    // Phase 12 / US3: is a `[reranker]` provider configured? When so, reranking
    // is remote (no local reranker model required). A non-Voyage kind / undefined
    // reference / missing model surfaces as `ProviderConfigInvalid`/93 here — the
    // same code `build_reranker` would later produce, surfaced before the (now
    // skippable) missing-model check.
    let remote_reranking =
        crate::provider::resolve(&cfg, crate::provider::Capability::Reranker)?.is_some();

    // Model presence — embedder always required (BUNDLED only — a remote
    // embedder has no local model), reranker required unless `--no-rerank`. We
    // check before constructing the heavy `FastembedEmbedder` so a missing-model
    // error doesn't pay the load cost first.
    let missing = missing_models(&paths);
    if !remote_embedding && missing.iter().any(|e| e.name == embedder_meta.name) {
        return Err(TomeError::ModelMissing {
            model: embedder_meta.name.to_owned(),
        });
    }
    if !args.no_rerank && !remote_reranking && missing.iter().any(|e| e.name == reranker_meta.name)
    {
        return Err(TomeError::ModelMissing {
            model: reranker_meta.name.to_owned(),
        });
    }

    // Build the embedder: remote when `[embedding]` is configured, else the
    // bundled active-profile model. On the remote path seed the validator's
    // expected dimension from the persisted `meta.embedder_dimension` so
    // query-time validation asserts the SAME dimension the index was built at.
    let persisted_dim = if remote_embedding && paths.index_db.is_file() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        crate::index::read_embedder_dimension(&conn)?
    } else {
        None
    };
    let embedder: Box<dyn Embedder> = {
        let pb = progress::spinner(format!("loading embedder ({})", embedder_meta.name));
        let result = crate::embedding::build_embedder(&cfg, &paths, embedder_meta, persisted_dim);
        pb.finish_and_clear();
        result?
    };
    // Build the reranker: remote when `[reranker]` is configured, else the
    // bundled active-profile model. `build_reranker` fires the one-time remote
    // notice and surfaces a non-Voyage kind as 93. `--no-rerank` (or `[query]
    // rerank=false` via `resolve_query_args`) skips it entirely.
    let reranker_loaded: Option<Box<dyn Reranker>> = if args.no_rerank {
        None
    } else {
        let pb = progress::spinner(format!("loading reranker ({})", reranker_meta.name));
        let result = crate::embedding::build_reranker(&cfg, &paths, reranker_meta);
        pb.finish_and_clear();
        Some(result?)
    };
    let reranker: Option<&dyn Reranker> = reranker_loaded.as_deref();

    // The drift-detection seed must reflect the ACTIVE embedder identity —
    // remote (`"<provider>/<model>"`/`"external"`) when configured, else the
    // active-profile registry identity — so switching `[embedding]` model
    // surfaces as embedder drift on the query path.
    let embedder_seed = crate::embedding::embedder_seed(&cfg, embedder_meta)?;
    // Phase 12 / US3: the reranker drift seed reflects the ACTIVE reranker
    // identity too. A remote `[reranker]` has identity `"<provider>/<model>"` /
    // `"external"`; bundled keeps the registry identity. Reranking is stateless
    // (no persisted artefact), so reranker drift is only a soft warning at query
    // time — but the seed must still match the active reranker so a bundled index
    // queried under a remote reranker (or vice-versa) reports the drift honestly
    // rather than a spurious or missing one.
    let reranker_seed = match crate::provider::resolve(&cfg, crate::provider::Capability::Reranker)?
    {
        Some(resolved) => MetaSeed {
            name: format!("{}/{}", resolved.name, resolved.model),
            version: crate::embedding::REMOTE_EMBEDDER_VERSION.to_owned(),
        },
        None => MetaSeed {
            name: reranker_meta.name.to_owned(),
            version: reranker_meta.version.to_owned(),
        },
    };
    let deps = QueryDeps {
        paths: &paths,
        scope: &scope.scope,
        config: &config,
        embedder: embedder.as_ref(),
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
    //
    // Load config ONCE (defensively) for BOTH provider-kind fields — telemetry
    // must never hard-fail on a malformed config.
    let telemetry_cfg = crate::config::load_or_default(deps.paths);
    crate::telemetry::emit(crate::telemetry::event::Search {
        surface: crate::telemetry::event::Surface::Cli,
        latency_ms: elapsed.as_millis().min(u32::MAX as u128) as u32,
        candidates_returned: outcome.results.len() as u32,
        reranker_used,
        strict: args.strict,
        corpus_size: outcome.corpus_size as u32,
        // The embedder identity is the one the caller loaded. The telemetry
        // field is `&'static str`, so recover the pinned registry entry by the
        // seed name; a non-registry seed (e.g. a test stub) falls back to the
        // DEFAULT profile's pinned embedder so the field is never free-form and
        // is byte-stable with the pre-tiering behaviour.
        embedder_model_id: Some(
            crate::embedding::registry::lookup(&deps.embedder_seed.name)
                .map(|e| e.name)
                .unwrap_or_else(|| {
                    crate::embedding::profile::embedder_for(
                        crate::embedding::profile::Profile::DEFAULT,
                    )
                    .name
                }),
        ),
        // Phase 12: which provider kind served the embedding + the reranking,
        // each derived defensively from config (telemetry is best-effort — a
        // malformed config must not break the emit) via the shared SSOT mappers.
        // `Bundled` when no provider is configured for that capability. Records
        // ONLY the kind. Load config ONCE for both. FR-022: independent fields.
        embedding_provider_kind: crate::telemetry::event::ProviderKind::for_embedding(
            &telemetry_cfg,
        ),
        reranker_provider_kind: crate::telemetry::event::ProviderKind::for_reranker(&telemetry_cfg),
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
    // R-L1: gate the attribution work ONCE. `resolve_attribution` opens the
    // read-only index per distinct catalog, so skip the whole loop when telemetry
    // is disabled (the `emit`s would no-op anyway, but the attribution reads would
    // still run). The exact rank, the per-catalog memoised resolution, and the
    // alongside-the-anonymous semantics are unchanged. Best-effort: a disabled
    // install skips the attributed loop (the anonymous `tome.search` already
    // fired above).
    if telemetry_attribution_enabled() {
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
                crate::telemetry::emit(crate::telemetry::event::SearchResult {
                    catalog: catalog_id,
                    entry_name: c.name.clone(),
                    entry_kind: c.kind.into(),
                    plugin_name: c.plugin.clone(),
                    // EXACT 1-indexed rank (FR-057) — `idx + 1`, never bucketed.
                    rank: (idx + 1) as u32,
                    // CLI surface has no calling harness.
                    calling_harness: None,
                });
            }
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

/// Whether to do the attributed-search work, gated ONCE on the telemetry enabled
/// state (R-L1). When telemetry is disabled the whole attributed loop is skipped
/// — the per-result `resolve_attribution` reads (a read-only index open per
/// distinct catalog) are then never run, and the `emit`s would no-op anyway. The
/// enabled state is the process-global handle's (built in `main` before dispatch),
/// so no per-result `config.toml` read happens.
fn telemetry_attribution_enabled() -> bool {
    crate::telemetry::is_enabled()
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

    // Resolve top_k — callers of `pipeline` (the CLI `run` path via `run_with_deps`,
    // and the MCP handler) should always pass a `Some(resolved)` value; the
    // `DEFAULT_TOP_K` fallback here is a belt-and-suspenders default for direct
    // library callers (e.g. tests) that may omit the config-resolution step.
    let top_k_resolved: u32 = args.top_k.unwrap_or(DEFAULT_TOP_K);

    // Pull candidates. Reranking benefits from a wider pool — 4× per the
    // contract — and we trim back after.
    let candidate_k: u32 = if deps.reranker.is_some() {
        top_k_resolved.saturating_mul(4).max(top_k_resolved)
    } else {
        top_k_resolved
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

    let top_k = top_k_resolved as usize;
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

    // #285: the scope-effective searchable count — exactly the universe the
    // KNN above searched (same workspace_skills join + `searchable = 1`),
    // minus the vector distance / LIMIT. The MCP empty-result signal branches
    // on THIS so an empty-scope-with-content-elsewhere layout is correctly
    // reported as `index_empty` (reindex), not `no_match` (rephrase).
    // Best-effort: a count failure falls back to 0 (treated as empty scope),
    // which is the safe direction — it steers a user toward reindexing rather
    // than fruitlessly rephrasing.
    let scope_searchable_count =
        crate::index::query::scope_searchable_count(&conn, deps.scope.name().as_str()).unwrap_or(0);

    Ok(QueryOutcome {
        results: trimmed,
        scoring,
        threshold_passed,
        reranker_drift,
        corpus_size,
        scope_searchable_count,
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

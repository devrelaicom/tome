//! `search_skills` MCP tool — input/output schemas + handler.
//!
//! Contract: [`mcp-tools.md` §search_skills](../../../specs/003-phase-3-mcp-workspaces/contracts/mcp-tools.md).

use std::sync::Arc;
use std::time::Instant;

use rmcp::ErrorData as McpError;
use rmcp::model::ErrorCode;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{error, info};

use crate::cli::QueryArgs;
use crate::commands::query;
use crate::embedding::Reranker;
use crate::error::{ErrorCategory, TomeError};
use crate::index::MetaSeed;
use crate::mcp::state::McpState;
use crate::mcp::tools::common::{error_data, error_data_with_code};
use crate::plugin::identity::EntryKind;

/// The tool description per `mcp-tools.md` §search_skills lives on the
/// `#[tool]`-decorated method in `mcp::server` as a doc comment — the
/// `#[tool]` macro accepts string literals only via `description = "..."`
/// and falls back to doc comments otherwise. Tests in
/// `tests/mcp_server.rs` assert the wording (FR-108).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// Natural-language description of the task.
    pub query: String,
    /// Maximum results to return after reranking. 1..=100, default 10.
    /// When absent, falls back to `[query] top_k` in `~/.tome/config.toml`,
    /// then to the built-in default of 10.
    #[serde(default)]
    pub top_k: Option<u32>,
    /// Restrict to one catalog by name (must match an enabled catalog
    /// in the resolved scope).
    #[serde(default)]
    pub catalog: Option<String>,
    /// Restrict to one plugin within `catalog` (requires `catalog`).
    /// Format: plugin name only, NOT `<catalog>/<plugin>`.
    #[serde(default)]
    pub plugin: Option<String>,
    /// Truncate each result's description at this many characters
    /// (Unicode scalar values), per FR-092. When absent, falls back to
    /// `[mcp] description_max_chars` in `~/.tome/config.toml`, then to
    /// the built-in default of 150. Pass a large value to minimise
    /// truncation; the resolved value must not exceed
    /// [`MAX_DESCRIPTION_MAX_CHARS`] — anything strictly above is rejected as
    /// `invalid_description_max_chars`. Negative values are rejected by the
    /// `u32` deserialiser.
    #[serde(default)]
    pub description_max_chars: Option<u32>,
}

/// Sanity cap on `description_max_chars`. Values strictly above this
/// surface as `invalid_description_max_chars` — anything in this range
/// already vastly exceeds what an agent should ever request in a single
/// result, so we reject rather than silently allocate a megabyte string.
///
/// US4.d M-1: this constant is INTENTIONALLY a sanity guard above the
/// documented contract surface (`contracts/mcp-tools-p5.md` § Error
/// responses only lists `description_max_chars < 0` as the rejection
/// trigger). `u32` deserialisation handles the negative branch; this
/// catches absurd-but-legal-u32 values that no real agent would request.
/// Contract amended in same commit to mention the 100_000 sanity cap.
pub const MAX_DESCRIPTION_MAX_CHARS: u32 = 100_000;

/// Built-in default description-truncation cap when neither the per-call
/// `description_max_chars` arg nor `[mcp] description_max_chars` is set. Single
/// source of truth: this tool AND `tome config show` reference it so the shown
/// default can't drift away from the effective one.
pub const DEFAULT_DESCRIPTION_MAX_CHARS: u32 = 150;

/// Maximum allowed length of an MCP `search_skills.query` input, in
/// `char`s (Unicode scalar values). Research §R-17 documents 4096 as
/// the cap; queries strictly longer than this are rejected with a
/// dedicated MCP error envelope. Length is measured in `char`s rather
/// than `bytes` so multi-byte UTF-8 inputs aren't penalised by their
/// encoding.
pub const MAX_QUERY_CHARS: usize = 4096;

#[derive(Debug, Serialize, JsonSchema)]
pub struct Output {
    pub matches: Vec<SkillMatch>,
    /// #285: the number of searchable entries actually searched IN THE
    /// RESOLVED SCOPE — the enabled, `searchable = 1` skills joined into
    /// this workspace, i.e. the exact universe the KNN ran over. Always
    /// present; best-effort `0` on a count failure. Lets a caller
    /// distinguish "nothing is indexed for this scope — reindex / enable a
    /// plugin" (`corpus_size == 0`) from "the scope has searchable content
    /// but nothing matched — rephrase" (`corpus_size > 0`) when `matches`
    /// is empty. Threaded from `QueryOutcome::scope_searchable_count`
    /// (computed in the query pipeline), not recomputed here. NOTE: this is
    /// the SCOPE-EFFECTIVE count, deliberately distinct from the whole-index
    /// count that feeds the `tome.search` telemetry bucket.
    pub corpus_size: u64,
    /// #285: how the returned `score` on each match was produced —
    /// `"reranked"` (a cross-encoder logit) or `"embedding-similarity"`
    /// (`1.0 − cosine distance`, raw KNN order). Always present. This is
    /// the SSOT string from the CLI's [`ScoringMode`](crate::commands::query::ScoringMode);
    /// read it to interpret `score`, which is otherwise opaque and NOT
    /// comparable across the two modes.
    pub scoring: String,
    /// #285: a one-line note when the ACTIVE reranker's identity has
    /// drifted from the identity the index was built against, so results
    /// fell back to embedding-similarity ranking. Present only under
    /// drift (`skip_serializing_if`); absent on the common path so the
    /// wire shape stays stable. Threaded from `QueryOutcome::reranker_drift`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reranker_drift: Option<String>,
    /// #285: a machine-readable reason present ONLY when `matches` is
    /// empty (`skip_serializing_if`), so the wire shape of a normal
    /// non-empty result is unchanged. Distinguishes an empty index
    /// (`index_empty`) from a populated index with no semantic match
    /// (`no_match`); pair it with [`Self::hint`] for the human-readable
    /// guidance.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub no_results_reason: Option<NoResultsReason>,
    /// #285: a one-line actionable hint present ONLY when `matches` is
    /// empty (`skip_serializing_if`). Reindex guidance when the index is
    /// empty; rephrase/broaden guidance when it has content but nothing
    /// matched. This is the human-readable companion to
    /// [`Self::no_results_reason`] — surfaced in the structured output (not
    /// prose) so an MCP client can read it directly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// #285: why a `search_skills` call returned no matches. A closed enum so
/// the signal is structural — an agent can branch on it rather than parse
/// the human-readable [`Output::hint`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NoResultsReason {
    /// The resolved scope has zero searchable entries (`corpus_size == 0`) —
    /// nothing was searchable here, so the fix is to reindex / enable a
    /// plugin for this scope, NOT to rephrase. (Other scopes may hold
    /// content; this reason is about the queried scope only.)
    IndexEmpty,
    /// The resolved scope has searchable content (`corpus_size > 0`) but
    /// nothing scored a semantic match — the fix is to rephrase or broaden
    /// the query, not to reindex.
    NoMatch,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SkillMatch {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    /// Phase 5: entry kind discriminator (`skill` | `command`) per
    /// FR-091. Lets callers distinguish skills from commands in the
    /// same ranked result set.
    pub kind: EntryKind,
    /// The indexed description (frontmatter `description` or fallback
    /// per FR-012), truncated to `description_max_chars` characters
    /// (Unicode scalar values) with the ellipsis character `…` (U+2026)
    /// appended when truncation occurred, per FR-092.
    pub description: String,
    pub plugin_version: String,
    /// Absolute path to the SKILL.md file.
    pub path: String,
    /// Ranking score for this match. Its scale is OPAQUE and mode-dependent
    /// — read [`Output::scoring`] to interpret it: `"reranked"` means a
    /// cross-encoder logit (can be negative or exceed 1, higher is better);
    /// `"embedding-similarity"` means `1.0 − cosine distance` in `[-1, 1]`.
    /// Scores are only comparable WITHIN one result set / one scoring mode;
    /// do not compare a score against a fixed threshold or across calls.
    /// #285: the top-level `scoring` field names the mode for the whole set.
    pub score: f32,
    /// #289: the MCP prompt name (`<plugin>__<entry>` form, post-override
    /// and post-collision-suffix) this entry is reachable under via
    /// `prompts/list` / `prompts/get`. Present for any user-invocable entry
    /// (every command by default; a skill that opted into the prompt
    /// surface). Absent (`skip_serializing_if`) for an entry with no prompt
    /// — so the skill-kind wire shape is byte-stable, and a caller seeing
    /// `kind: "command"` without `prompt_name` knows it has no prompt to
    /// invoke. Resolved from the live `PromptRegistry` (the SSOT), never
    /// re-derived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_name: Option<String>,
}

/// Pipeline:
///
/// 1. Validate `plugin` requires `catalog` / catalog known / plugin known
///    against the resolved scope's config (rmcp error codes per contract).
/// 2. Lazy-load the reranker on first call (idempotent for the rest of
///    the server's lifetime, per FR-109).
/// 3. Dispatch to `commands::query::pipeline` (silent — no stdout/stderr
///    side-effects, FR-221).
/// 4. Map `Scored` rows to `SkillMatch`; return.
pub async fn handle(state: Arc<McpState>, input: Input) -> Result<Output, McpError> {
    let started = Instant::now();

    // Resolve effective top_k: per-call argument → config default → built-in 10.
    // Load config defensively (MCP handlers must never hard-fail on a malformed
    // config.toml — that's the CLI's job).
    let cfg = crate::config::load_or_default(&state.paths);
    let effective_top_k: u32 = input
        .top_k
        .or(cfg.query.top_k)
        .unwrap_or(crate::commands::query::DEFAULT_TOP_K);

    // Bounds-check the RESOLVED value so the config default is also guarded.
    if effective_top_k == 0 || effective_top_k > 100 {
        return Err(McpError::invalid_params(
            "top_k must be between 1 and 100",
            None,
        ));
    }
    // Resolve effective description_max_chars:
    // per-call arg → [mcp] description_max_chars in config → the built-in default.
    // Sanity-cap the RESOLVED value per `mcp-tools-p5.md` § Error responses.
    let effective_dmc = input
        .description_max_chars
        .or(cfg.mcp.description_max_chars)
        .unwrap_or(DEFAULT_DESCRIPTION_MAX_CHARS);
    if effective_dmc > MAX_DESCRIPTION_MAX_CHARS {
        return Err(McpError::invalid_params(
            format!("description_max_chars must be at most {MAX_DESCRIPTION_MAX_CHARS}"),
            Some(error_data_with_code(
                "invalid_description_max_chars",
                ErrorCategory::Usage,
                &[("max", json!(MAX_DESCRIPTION_MAX_CHARS))],
            )),
        ));
    }
    if input.query.trim().is_empty() {
        return Err(McpError::invalid_params("query must not be empty", None));
    }
    // FR-573 / P8 deferred fold-in (US5.a T373): cap query length so a
    // hostile or accidental megabyte-blob doesn't tie up the embedder.
    // 4096 chars is the documented maximum (research §R-17); strictly
    // greater than the cap is rejected, equal is allowed.
    if input.query.chars().count() > MAX_QUERY_CHARS {
        return Err(McpError::invalid_params(
            format!("query exceeds maximum length of {MAX_QUERY_CHARS} characters"),
            Some(error_data_with_code(
                "query_too_long",
                ErrorCategory::Usage,
                &[("max_chars", json!(MAX_QUERY_CHARS))],
            )),
        ));
    }
    if input.plugin.is_some() && input.catalog.is_none() {
        return Err(McpError::invalid_params(
            "plugin requires catalog",
            Some(error_data_with_code(
                "plugin_without_catalog",
                ErrorCategory::Usage,
                &[],
            )),
        ));
    }

    // FF3: catalog existence resolves from the `workspace_catalogs` DB, not
    // `config.toml [catalogs]` (never written in production → any `--catalog`
    // filter returned `unknown_catalog` on a fresh install). Checked here,
    // before the (expensive) reranker load, so an unknown catalog fails fast
    // with the same envelope the query pipeline would later produce. The
    // pipeline's own `validate_filters` (DB-backed since FF2) remains the
    // backstop and additionally validates the `--plugin` filter.
    if let Some(catalog) = input.catalog.as_deref() {
        let paths = state.paths.clone();
        let scope = state.scope.scope.clone();
        let catalog_owned = catalog.to_owned();
        let exists = tokio::task::spawn_blocking(move || {
            let conn = crate::index::db::open_read_only(&paths.index_db)?;
            crate::index::workspace_catalogs::find(&conn, scope.name().as_str(), &catalog_owned)
                .map(|o| o.is_some())
        })
        .await
        .map_err(|e| {
            McpError::internal_error(
                format!("catalog check join: {e}"),
                Some(error_data(ErrorCategory::Internal)),
            )
        })?
        .map_err(tome_to_mcp)?;
        if !exists {
            return Err(McpError::invalid_params(
                format!("catalog `{catalog}` is not enabled in the resolved scope"),
                Some(error_data_with_code(
                    "unknown_catalog",
                    ErrorCategory::EntryNotFound,
                    &[("catalog", json!(catalog))],
                )),
            ));
        }
    }

    // Lazy-build the reranker. `tokio::sync::OnceCell::get_or_try_init`
    // dedupes concurrent first-call requests; the build runs once and the
    // resulting `Arc<dyn Reranker>` is startup-frozen for the rest of the
    // server's lifetime (mirroring the startup-frozen embedder).
    //
    // Phase 12 / US3: select remote-vs-bundled INSIDE the `spawn_blocking` (the
    // RemoteReranker is sync). `build_reranker` resolves `[reranker]` from config
    // (loaded defensively — MCP handlers never hard-fail on a malformed config;
    // a malformed config resolves to bundled here). A remote build is infallible
    // at construction; a missing bundled model still surfaces via `load`.
    let reranker_entry = state.reranker_entry;
    let reranker_paths = state.paths.clone();
    let reranker_arc = state
        .reranker
        .get_or_try_init(|| async move {
            tokio::task::spawn_blocking(move || {
                let cfg = crate::config::load_or_default(&reranker_paths);
                crate::embedding::build_reranker(&cfg, &reranker_paths, reranker_entry)
            })
            .await
            .map_err(|e| TomeError::McpStartupFailed {
                reason: format!("reranker build join: {e}"),
            })?
            .map(Arc::from)
        })
        .await
        .map_err(tome_to_mcp)?
        .clone();

    // Translate Input → QueryArgs.
    //
    // `rerank` follows `cfg.query.rerank` (no per-call MCP arg exists today):
    //   config `rerank = false` → `no_rerank: true` → reranker skipped.
    //   This matters when the reranker model is not installed for the profile.
    //
    // `strict` / `min_score` (strict_min_score) are intentionally CLI-only.
    // MCP returns the top_k scored results and lets the agent decide; applying
    // a strict floor would silently drop results with no visible signal to the
    // caller.  Leave `strict: false` / `min_score: None`.
    let no_rerank = !cfg.query.rerank.unwrap_or(true);
    let args = QueryArgs {
        text: input.query.clone(),
        top_k: Some(effective_top_k),
        catalog: input.catalog.clone(),
        plugin: input.plugin.clone(),
        no_rerank,
        strict: false,
        min_score: None,
    };

    // Phase 12 / US2: the embedder seed is the ACTIVE identity computed at
    // startup (remote `"<provider>/<model>"`/`"external"` or the bundled
    // registry identity) — NOT re-derived from `embedder_entry`, so drift
    // detection on a remote index compares against the right stored `meta` rows.
    let embedder_seed = state.embedder_seed.clone();
    // Phase 12 / US3: the reranker drift seed reflects the ACTIVE reranker
    // identity (remote `"<provider>/<model>"`/`"external"` when `[reranker]` is
    // configured, else the bundled registry identity), mirroring the CLI. Resolve
    // defensively from the already-loaded `cfg`; a malformed reference degrades to
    // the bundled identity (telemetry/drift is best-effort on the MCP surface —
    // reranker drift is a soft, non-fatal label the handler never surfaces). The
    // build above already picked remote-vs-bundled from the same config.
    let reranker_seed = match crate::provider::resolve(&cfg, crate::provider::Capability::Reranker)
    {
        Ok(Some(resolved)) => MetaSeed {
            name: format!("{}/{}", resolved.name, resolved.model),
            version: crate::embedding::REMOTE_EMBEDDER_VERSION.to_owned(),
        },
        _ => MetaSeed {
            name: state.reranker_entry.name.into(),
            version: state.reranker_entry.version.into(),
        },
    };

    let embedder = state.embedder.clone();
    let reranker: Arc<dyn Reranker> = reranker_arc;
    let paths = state.paths.clone();
    let scope = state.scope.scope.clone();
    // FF2 vestigial slot: `QueryDeps.config` is unused by `query::pipeline`
    // (catalog/plugin validation was moved to the `workspace_catalogs` DB).
    // Distinct from `cfg` above, which was loaded for `top_k` / `rerank`
    // resolution; this slot receives a bare default so the FF2 DB-only path
    // is preserved without a second config read.
    let config = crate::config::Config::default();

    // Capture the strict flag for the telemetry emit before `args` is moved
    // into the blocking closure.
    let strict = args.strict;

    // The pipeline calls into `rusqlite` + `fastembed`, both sync.
    // Run on the blocking pool so the single-threaded reactor isn't
    // held up by inference latency.
    //
    // FR-027a: time the COMPUTE boundary ONLY — `Instant` wraps the `pipeline`
    // call inside the closure so the bucketed `latency_bucket` excludes the
    // `spawn_blocking` dispatch, the enqueue overhead, and the result mapping
    // below. The raw `Duration` rides back out of the closure alongside the
    // outcome; only its bucket is ever reported.
    let (outcome, compute_elapsed) = tokio::task::spawn_blocking(move || {
        let deps = query::QueryDeps {
            paths: &paths,
            scope: &scope,
            config: &config,
            embedder: embedder.as_ref(),
            reranker: Some(reranker.as_ref()),
            embedder_seed,
            reranker_seed,
        };
        let compute_started = Instant::now();
        let result = query::pipeline(&args, &deps);
        let compute_elapsed = compute_started.elapsed();
        result.map(|o| (o, compute_elapsed))
    })
    .await
    .map_err(|e| {
        McpError::internal_error(
            format!("query pipeline join: {e}"),
            Some(error_data(ErrorCategory::Internal)),
        )
    })?
    .map_err(|e| {
        // C-L1: best-effort MCP-surface `tome.error` (closed category only, never
        // the raw message), carrying this session's `calling_harness` + the `Mcp`
        // surface. Emitted at the terminal `TomeError`→`McpError` conversion;
        // never alters the returned `McpError`.
        crate::mcp::enqueue_tool_error(&state, e.category());
        // US4 deferral: no clean plugin context at this error boundary. A search
        // failure is not scoped to a single plugin (it is a query/embedder/index
        // failure), so there is no `plugin_name`/`plugin_version` to attribute —
        // the attributed `catalog.<id>.error` stays deferred here. Anonymous
        // `tome.error` above is the right granularity for a search failure.
        // Translate filter-validation results into the contract's
        // structured error codes.
        match &e {
            TomeError::CatalogNotFound(name) => McpError::invalid_params(
                format!("catalog `{name}` is not enabled in the resolved scope"),
                Some(error_data_with_code(
                    "unknown_catalog",
                    ErrorCategory::EntryNotFound,
                    &[("catalog", json!(name))],
                )),
            ),
            TomeError::PluginNotFound(id) => {
                let (catalog, plugin) = split_id(id);
                McpError::invalid_params(
                    format!("plugin `{id}` is not enabled in the resolved scope"),
                    Some(error_data_with_code(
                        "unknown_plugin",
                        ErrorCategory::EntryNotFound,
                        &[("catalog", json!(catalog)), ("plugin", json!(plugin))],
                    )),
                )
            }
            _ => tome_to_mcp(e),
        }
    })?;

    let description_max_chars = effective_dmc as usize;
    // #289: resolve each result's MCP prompt name from the live registry (the
    // SSOT) so a command (or any user-invocable entry) returned by search is
    // immediately actionable via `prompts/get`. The lookup is a sub-µs scan
    // over the in-memory registry (no DB I/O) — safe on the reactor. Clone the
    // `Arc` out of the read lock so the borrow isn't tied to the guard.
    let registry = state
        .prompt_registry
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let matches: Vec<SkillMatch> = outcome
        .results
        .iter()
        .map(|s| SkillMatch {
            catalog: s.candidate.catalog.clone(),
            plugin: s.candidate.plugin.clone(),
            name: s.candidate.name.clone(),
            kind: s.candidate.kind,
            description: truncate_description(&s.candidate.description, description_max_chars),
            plugin_version: s.candidate.plugin_version.clone(),
            path: s.candidate.path.clone(),
            score: s.score,
            prompt_name: registry
                .prompt_name_for(
                    &s.candidate.catalog,
                    &s.candidate.plugin,
                    s.candidate.kind,
                    &s.candidate.name,
                )
                .map(str::to_owned),
        })
        .collect();

    // FR-028: record this search's result ranks into the per-session funnel
    // state BEFORE emitting, clearing the prior search's ranks so only the
    // latest search attributes a `rank_bucket` to a later get. The rank is the
    // 1-indexed position in the returned (already top-k, reranked) list. The
    // lock is held only for this clear+repopulate — never across an `.await`.
    if let Ok(mut ranks) = state.last_search_ranks.lock() {
        ranks.clear();
        for (idx, m) in matches.iter().enumerate() {
            // `idx + 1` is the 1-indexed rank. A duplicate `name` (same entry
            // name across plugins in one result set) keeps its FIRST/best rank.
            ranks.entry(m.name.clone()).or_insert((idx + 1) as u32);
        }
    }
    // Note: a poisoned lock (a prior holder panicked) silently skips the
    // funnel update — telemetry is best-effort and must never fail the tool.

    // FR-027: `tome.search` fires on the MCP surface for a successful search.
    // Best-effort `enqueue` (a sub-ms local append; SC-009: an unreachable
    // endpoint never blocks the handler — enqueue does NOT flush). Mirrors the
    // CLI `query::run_with_deps` emit; the divergence is `surface = Mcp` plus
    // the `calling_harness` dimension (CLI has no host harness).
    crate::telemetry::emit(crate::telemetry::event::Search {
        surface: crate::telemetry::event::Surface::Mcp,
        latency_ms: compute_elapsed.as_millis().min(u32::MAX as u128) as u32,
        candidates_returned: matches.len() as u32,
        // Reranker used iff `no_rerank` was not set by the config resolution
        // above.  Mirrors the `run_with_deps` emit: `reranker.is_some()`.
        reranker_used: !no_rerank,
        strict,
        // Mirror the CLI: the whole-index corpus size the pipeline already
        // computed (best-effort `0` on a count failure; the kernel buckets it).
        corpus_size: outcome.corpus_size as u32,
        // The embedder identity is the pinned registry entry's `&'static str`
        // name — a closed-set value from `MODEL_REGISTRY`, never free-form. On a
        // remote-embedding server this still names the active profile's bundled
        // registry entry (the per-search provider kind is the new field below).
        embedder_model_id: Some(state.embedder_entry.name),
        // Phase 12: which provider kind served the embedding + the reranking for
        // this MCP search. Same SSOT mappers as the CLI; `cfg` was loaded
        // defensively above. Records ONLY the kind. FR-022: independent fields.
        embedding_provider_kind: crate::telemetry::event::ProviderKind::for_embedding(&cfg),
        reranker_provider_kind: crate::telemetry::event::ProviderKind::for_reranker(&cfg),
        calling_harness: crate::mcp::calling_harness(&state),
    });

    // FR-052 + FR-057: ALONGSIDE the anonymous `tome.search` above, emit one
    // catalog-attributed `catalog.<id>.search_result` per result entry whose
    // catalog resolves — by SOURCE, at emit time — to an allowlisted catalog.
    // `rank` is the EXACT 1-indexed position in the returned (already top-k,
    // reranked) list, NOT bucketed (FR-057): server-side selection attribution
    // joins this against the later `entry_invoked` on `(session_uuid,
    // entry_name)`. Attribution is memoised per catalog name so a result set
    // spanning several catalogs opens the read-only index at most once per
    // distinct catalog (NFR-009 — no lock). Best-effort; never alters the result.
    //
    // Sec-M1 / R-L1: `resolve_attribution` does a SYNC SQLite open+query (5s
    // busy_timeout) — running it inline on the single-threaded MCP reactor can
    // stall the server under index-write contention, violating the project's
    // "spawn_blocking for sync work in async MCP handlers" discipline (which
    // `sync_boundary.rs` cannot catch — it only guards the module boundary, not
    // blocking-on-the-reactor). Fold the per-catalog resolution + each
    // `enqueue_attributed` (itself a sync queue append) into ONE `spawn_blocking`;
    // ignore a join error (best-effort). The funnel-rank capture above stays on
    // the reactor (a sub-µs `Mutex`, not a DB read). Exact-rank + per-catalog
    // memoisation + the `Mcp`-surface `calling_harness` are all preserved.
    let attribution_scope = state.scope.clone();
    let search_harness = crate::mcp::calling_harness(&state);
    // Snapshot only what the closure needs (name/kind/plugin/catalog/rank) so the
    // `matches` Vec can be returned in the `Output` unmoved.
    let attribution_rows: Vec<(
        String,
        crate::telemetry::event::EntryKind,
        String,
        String,
        u32,
    )> = matches
        .iter()
        .enumerate()
        .map(|(idx, m)| {
            (
                m.name.clone(),
                m.kind.into(),
                m.plugin.clone(),
                m.catalog.clone(),
                // EXACT 1-indexed rank (FR-057) — `idx + 1`, never bucketed.
                (idx + 1) as u32,
            )
        })
        .collect();
    let _ = tokio::task::spawn_blocking(move || {
        // R-L1: gate the attribution work ONCE on the handle-backed enabled state.
        // When telemetry is disabled, skip the whole loop — the per-result
        // `resolve_attribution` reads (a read-only index open per distinct
        // catalog) are then never run, and the `emit`s would no-op anyway. The
        // anonymous `tome.search` already fired above.
        if !crate::telemetry::is_enabled() {
            return;
        }
        let mut attribution_cache: std::collections::HashMap<String, Option<&'static str>> =
            std::collections::HashMap::new();
        for (entry_name, entry_kind, plugin_name, catalog, rank) in attribution_rows {
            let catalog_id = *attribution_cache.entry(catalog.clone()).or_insert_with(|| {
                crate::telemetry::resolve_attribution(&attribution_scope, &catalog)
            });
            if let Some(catalog_id) = catalog_id {
                crate::telemetry::emit(crate::telemetry::event::SearchResult {
                    catalog: catalog_id,
                    entry_name,
                    entry_kind,
                    plugin_name,
                    rank,
                    calling_harness: search_harness,
                });
            }
        }
    })
    .await;

    // FR-M-LOG-5: contract names `filter` as a nested JSON object.
    // tracing flattens fields, so emit two named slots (`filter_catalog`,
    // `filter_plugin`). Closer to a structured shape than `?FilterLog`'s
    // Rust-Debug string; consumers can re-nest in jq via
    // `{filter: {catalog, plugin}}`.
    info!(
        target: "tome::mcp::tools::search_skills",
        query_len = input.query.len(),
        top_k = effective_top_k,
        filter_catalog = input.catalog.as_deref(),
        filter_plugin = input.plugin.as_deref(),
        matches = matches.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );

    // #285: thread the signal the pipeline already computed into the tool
    // output. `corpus_size` + `scoring` are always present; `reranker_drift`
    // rides only when detected. On an empty result set, attach a structured
    // reason + a one-line hint so the agent can tell "index empty for this
    // scope → reindex" from "no semantic match → rephrase" without parsing
    // prose.
    //
    // `corpus_size` here is the SCOPE-EFFECTIVE searchable count (the exact
    // universe the KNN searched), NOT the whole-index count that feeds
    // telemetry (`outcome.corpus_size`). Using the scoped count is what makes
    // `corpus_size == 0` ⇔ `index_empty` self-consistent: an empty scope whose
    // OTHER scopes hold content must be reported as `index_empty` (reindex /
    // enable a plugin for THIS scope), not `no_match` (rephrase).
    let corpus_size = outcome.scope_searchable_count;
    let scoring = outcome.scoring.as_str().to_owned();
    let reranker_drift = outcome.reranker_drift;
    let (no_results_reason, hint) = if matches.is_empty() {
        if corpus_size == 0 {
            (
                Some(NoResultsReason::IndexEmpty),
                Some(
                    "No skills are indexed for this scope — run `tome reindex`, or enable a plugin for this workspace."
                        .to_owned(),
                ),
            )
        } else {
            (
                Some(NoResultsReason::NoMatch),
                Some(
                    "No semantic match — try rephrasing or broadening the query, or check that a relevant plugin is enabled."
                        .to_owned(),
                ),
            )
        }
    } else {
        (None, None)
    };

    Ok(Output {
        matches,
        corpus_size,
        scoring,
        reranker_drift,
        no_results_reason,
        hint,
    })
}

/// Truncate `s` to `max` Unicode scalar values, appending the ellipsis
/// character `…` (U+2026) when truncation occurred. Per FR-092 the
/// post-truncation string is exactly `max` content chars + the one
/// ellipsis char (total `max + 1`). When `s` already fits within `max`
/// chars, it's returned verbatim. When `max == 0` an empty string is
/// returned (defensive — the input validation rejects values strictly
/// above [`MAX_DESCRIPTION_MAX_CHARS`] but `0` is a legal opt-out
/// value if a caller really wants empty descriptions).
///
/// Character count uses Unicode scalar values (`chars()`), NOT bytes —
/// a multi-byte UTF-8 input isn't penalised by its encoding. Mirrors
/// the same discipline used for `MAX_QUERY_CHARS` (Phase 4 US5.a).
///
/// US4.d C-2 + Security HIGH fix: this implementation uses
/// `char_indices` to walk past `max` chars then stop — bounded O(max)
/// work regardless of input size. The previous implementation called
/// `chars().count()` for the early-return check, which is O(n) over
/// the FULL input even when no truncation was needed. With caller-
/// controlled `description_max_chars` (sanity cap 100,000) running over
/// `top_k` results × multi-KB descriptions, that was a meaningful DoS
/// amplifier. The new shape: walk at most `max + 1` chars, then either
/// return the input verbatim (no truncation needed) or slice at the
/// `max+1`-th char's byte offset and append the ellipsis. No
/// `take().collect()` allocation in the truncation path; no full-string
/// traversal in the no-truncation path.
fn truncate_description(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut iter = s.char_indices();
    // Walk past `max` chars; if we exhaust the iterator within those,
    // no truncation needed (input already fit).
    for _ in 0..max {
        if iter.next().is_none() {
            return s.to_owned();
        }
    }
    // If the (max+1)-th char exists, slice at its byte offset and
    // append the ellipsis. Otherwise the input was exactly `max` chars
    // — no truncation needed.
    match iter.next() {
        None => s.to_owned(),
        Some((byte_idx, _)) => {
            let mut out = String::with_capacity(byte_idx + '\u{2026}'.len_utf8());
            out.push_str(&s[..byte_idx]);
            out.push('\u{2026}');
            out
        }
    }
}

/// Split a `<catalog>/<plugin>` identifier for the JSON error payload.
/// When the input contains no slash we return `("", id)` so the caller
/// still gets two fields to surface.
fn split_id(id: &str) -> (&str, &str) {
    match id.split_once('/') {
        Some((c, p)) => (c, p),
        None => ("", id),
    }
}

/// Translate a `TomeError` to an `McpError` with a contract-defined
/// `code` payload. Reaches for the residual `internal_error` only when
/// no specific mapping applies.
fn tome_to_mcp(e: TomeError) -> McpError {
    let msg = e.to_string();
    // FR-M-LOG-1 / log-format.md §Scrubbing: error chains may carry
    // signed URLs from reqwest or git output. Pass through
    // `scrub_credentials::scrub_to_string` before logging. The
    // user-facing McpError still gets the raw msg — the contract
    // doesn't ask us to scrub the protocol payload (that channel is
    // already authenticated to the harness).
    let scrubbed = crate::catalog::git::scrub_to_string(msg.as_bytes());
    error!(
        target: "tome::mcp::tools::search_skills",
        error_code = e.category().as_str(),
        error_message = %scrubbed,
        "tool error",
    );
    match &e {
        // Both name + version drift collapse onto the custom `embedder_drift`
        // slug (NOT `category().as_str()`); `retryable`/`remediation` still come
        // from the drift category so the "run `tome reindex --force`" fix that
        // used to live only in the prose now rides the structured payload.
        TomeError::EmbedderNameDrift { .. } | TomeError::EmbedderVersionDrift { .. } => {
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                msg,
                Some(error_data_with_code(
                    "embedder_drift",
                    ErrorCategory::EmbedderNameDrift,
                    &[],
                )),
            )
        }
        TomeError::IndexBusy => McpError::new(
            ErrorCode::INTERNAL_ERROR,
            msg,
            Some(error_data(ErrorCategory::IndexBusy)),
        ),
        _ => McpError::internal_error(msg, Some(error_data(e.category()))),
    }
}

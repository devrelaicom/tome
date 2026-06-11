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
use crate::embedding::fastembed::FastembedReranker;
use crate::error::TomeError;
use crate::index::MetaSeed;
use crate::mcp::state::McpState;
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
    #[serde(default = "default_top_k")]
    pub top_k: u32,
    /// Restrict to one catalog by name (must match an enabled catalog
    /// in the resolved scope).
    #[serde(default)]
    pub catalog: Option<String>,
    /// Restrict to one plugin within `catalog` (requires `catalog`).
    /// Format: plugin name only, NOT `<catalog>/<plugin>`.
    #[serde(default)]
    pub plugin: Option<String>,
    /// Truncate each result's description at this many characters
    /// (Unicode scalar values), per FR-092. Default 150 — agent-consumed
    /// search results preserve token budget. Set to a very large value
    /// (e.g. 99999) to opt out. Negative values are rejected by the
    /// `u32` deserialiser; values above [`MAX_DESCRIPTION_MAX_CHARS`]
    /// surface as `invalid_description_max_chars`.
    #[serde(default = "default_description_max_chars")]
    pub description_max_chars: u32,
}

fn default_top_k() -> u32 {
    10
}

fn default_description_max_chars() -> u32 {
    150
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
    /// Reranker score by default; embedding similarity if reranker
    /// drift forced fallback. The output does NOT distinguish — the
    /// score is opaque.
    pub score: f32,
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

    // Bounds-check top_k. The schema's `default = 10` covers the
    // missing case; here we enforce the 1..=100 range.
    if input.top_k == 0 || input.top_k > 100 {
        return Err(McpError::invalid_params(
            "top_k must be between 1 and 100",
            None,
        ));
    }
    // Sanity-cap description_max_chars per `mcp-tools-p5.md` § Error
    // responses. Serde's `u32` deserialisation already rejects negative
    // values; this guards against absurdly-large values that would
    // defeat the purpose of truncation.
    if input.description_max_chars > MAX_DESCRIPTION_MAX_CHARS {
        return Err(McpError::invalid_params(
            format!("description_max_chars must be at most {MAX_DESCRIPTION_MAX_CHARS}"),
            Some(json!({
                "code": "invalid_description_max_chars",
                "max": MAX_DESCRIPTION_MAX_CHARS,
            })),
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
            Some(json!({
                "code": "query_too_long",
                "max_chars": MAX_QUERY_CHARS,
            })),
        ));
    }
    if input.plugin.is_some() && input.catalog.is_none() {
        return Err(McpError::invalid_params(
            "plugin requires catalog",
            Some(json!({ "code": "plugin_without_catalog" })),
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
                Some(json!({ "code": "internal" })),
            )
        })?
        .map_err(tome_to_mcp)?;
        if !exists {
            return Err(McpError::invalid_params(
                format!("catalog `{catalog}` is not enabled in the resolved scope"),
                Some(json!({ "code": "unknown_catalog", "catalog": catalog })),
            ));
        }
    }

    // Lazy-load the reranker. `tokio::sync::OnceCell::get_or_try_init`
    // dedupes concurrent first-call requests; the model load runs once.
    let reranker_entry = state.reranker_entry;
    let reranker_dir = state
        .paths
        .model_path(reranker_entry.name)
        .map_err(tome_to_mcp)?;
    let reranker_arc = state
        .reranker
        .get_or_try_init(|| async move {
            tokio::task::spawn_blocking(move || {
                FastembedReranker::load(reranker_entry, &reranker_dir)
            })
            .await
            .map_err(|e| TomeError::McpStartupFailed {
                reason: format!("reranker load join: {e}"),
            })?
            .map(|r| Arc::new(r) as Arc<dyn Reranker>)
        })
        .await
        .map_err(tome_to_mcp)?
        .clone();

    // Translate Input → QueryArgs. `strict` / `no_rerank` / `min_score`
    // have no MCP equivalents — the agent decides what to do with the
    // scores. We always run the production pipeline.
    let args = QueryArgs {
        text: input.query.clone(),
        top_k: input.top_k,
        catalog: input.catalog.clone(),
        plugin: input.plugin.clone(),
        no_rerank: false,
        strict: false,
        min_score: None,
    };

    let embedder_seed = MetaSeed {
        name: state.embedder_entry.name.into(),
        version: state.embedder_entry.version.into(),
    };
    let reranker_seed = MetaSeed {
        name: state.reranker_entry.name.into(),
        version: state.reranker_entry.version.into(),
    };

    let embedder = state.embedder.clone();
    let reranker: Arc<dyn Reranker> = reranker_arc;
    let paths = state.paths.clone();
    let scope = state.scope.scope.clone();
    // FF2: `QueryDeps.config` is vestigial — `query::pipeline` resolves
    // `--catalog`/`--plugin` from the `workspace_catalogs` DB. Pass an empty
    // default rather than reading the never-written `config.toml`.
    let config = crate::config::Config::default();

    // The pipeline calls into `rusqlite` + `fastembed`, both sync.
    // Run on the blocking pool so the single-threaded reactor isn't
    // held up by inference latency.
    let outcome = tokio::task::spawn_blocking(move || {
        let deps = query::QueryDeps {
            paths: &paths,
            scope: &scope,
            config: &config,
            embedder: embedder.as_ref(),
            reranker: Some(reranker.as_ref()),
            embedder_seed,
            reranker_seed,
        };
        query::pipeline(&args, &deps)
    })
    .await
    .map_err(|e| {
        McpError::internal_error(
            format!("query pipeline join: {e}"),
            Some(json!({ "code": "internal" })),
        )
    })?
    .map_err(|e| {
        // Translate filter-validation results into the contract's
        // structured error codes.
        match &e {
            TomeError::CatalogNotFound(name) => McpError::invalid_params(
                format!("catalog `{name}` is not enabled in the resolved scope"),
                Some(json!({ "code": "unknown_catalog", "catalog": name })),
            ),
            TomeError::PluginNotFound(id) => {
                let (catalog, plugin) = split_id(id);
                McpError::invalid_params(
                    format!("plugin `{id}` is not enabled in the resolved scope"),
                    Some(json!({
                        "code": "unknown_plugin",
                        "catalog": catalog,
                        "plugin": plugin,
                    })),
                )
            }
            _ => tome_to_mcp(e),
        }
    })?;

    let description_max_chars = input.description_max_chars as usize;
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
        })
        .collect();

    // FR-M-LOG-5: contract names `filter` as a nested JSON object.
    // tracing flattens fields, so emit two named slots (`filter_catalog`,
    // `filter_plugin`). Closer to a structured shape than `?FilterLog`'s
    // Rust-Debug string; consumers can re-nest in jq via
    // `{filter: {catalog, plugin}}`.
    info!(
        target: "tome::mcp::tools::search_skills",
        query_len = input.query.len(),
        top_k = input.top_k,
        filter_catalog = input.catalog.as_deref(),
        filter_plugin = input.plugin.as_deref(),
        matches = matches.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );

    Ok(Output { matches })
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
        TomeError::EmbedderNameDrift { .. } | TomeError::EmbedderVersionDrift { .. } => {
            McpError::new(
                ErrorCode::INTERNAL_ERROR,
                msg,
                Some(json!({ "code": "embedder_drift" })),
            )
        }
        TomeError::IndexBusy => McpError::new(
            ErrorCode::INTERNAL_ERROR,
            msg,
            Some(json!({ "code": "index_busy" })),
        ),
        _ => McpError::internal_error(msg, Some(json!({ "code": e.category().as_str() }))),
    }
}

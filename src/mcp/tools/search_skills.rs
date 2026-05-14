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

use crate::catalog::store;
use crate::cli::QueryArgs;
use crate::commands::query;
use crate::embedding::Reranker;
use crate::embedding::fastembed::FastembedReranker;
use crate::error::TomeError;
use crate::index::MetaSeed;
use crate::mcp::state::McpState;

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
}

fn default_top_k() -> u32 {
    10
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Output {
    pub matches: Vec<SkillMatch>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct SkillMatch {
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    /// The indexed description (frontmatter `description` or fallback
    /// per FR-012).
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
    if input.query.trim().is_empty() {
        return Err(McpError::invalid_params("query must not be empty", None));
    }
    if input.plugin.is_some() && input.catalog.is_none() {
        return Err(McpError::invalid_params(
            "plugin requires catalog",
            Some(json!({ "code": "plugin_without_catalog" })),
        ));
    }

    let config = store::load(&state.paths.config_file_for(&state.scope.scope)).map_err(|e| {
        McpError::internal_error(
            format!("load config: {e}"),
            Some(json!({ "code": "internal" })),
        )
    })?;

    if let Some(catalog) = input.catalog.as_deref()
        && !config.catalogs.contains_key(catalog)
    {
        return Err(McpError::invalid_params(
            format!("catalog `{catalog}` is not enabled in the resolved scope"),
            Some(json!({ "code": "unknown_catalog", "catalog": catalog })),
        ));
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

    let matches: Vec<SkillMatch> = outcome
        .results
        .iter()
        .map(|s| SkillMatch {
            catalog: s.candidate.catalog.clone(),
            plugin: s.candidate.plugin.clone(),
            name: s.candidate.name.clone(),
            description: s.candidate.description.clone(),
            plugin_version: s.candidate.plugin_version.clone(),
            path: s.candidate.path.clone(),
            score: s.score,
        })
        .collect();

    info!(
        target: "tome::mcp::tools::search_skills",
        query_len = input.query.len(),
        top_k = input.top_k,
        filter = ?FilterLog {
            catalog: input.catalog.as_deref(),
            plugin: input.plugin.as_deref(),
        },
        matches = matches.len(),
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );

    Ok(Output { matches })
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
    error!(
        target: "tome::mcp::tools::search_skills",
        error_code = e.category(),
        error_message = %msg,
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
        _ => McpError::internal_error(msg, Some(json!({ "code": e.category() }))),
    }
}

/// Local payload type for the `filter` log field. `tracing` doesn't
/// know how to render `Input` directly, but a small struct keyed to the
/// contract's shape works fine through `Debug`. Fields are accessed by
/// the `tracing` derived `Debug` impl, which clippy's dead-code
/// analysis doesn't see — hence the explicit allow.
#[derive(Debug)]
#[allow(dead_code)]
struct FilterLog<'a> {
    catalog: Option<&'a str>,
    plugin: Option<&'a str>,
}

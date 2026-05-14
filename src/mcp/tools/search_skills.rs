//! `search_skills` MCP tool — input/output schemas + handler.
//!
//! Contract: [`mcp-tools.md` §search_skills](../../../specs/003-phase-3-mcp-workspaces/contracts/mcp-tools.md).
//!
//! US1.a ships the schemas + a stub handler body. US1.b fills the
//! handler with the KNN+rerank pipeline (reusing `commands::query::run_with_deps`
//! per T083).

use std::sync::Arc;

use rmcp::ErrorData as McpError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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

/// Handler stub for US1.a. The full KNN+rerank pipeline lands in US1.b
/// per T082, reusing `commands::query::run_with_deps` where the
/// pipeline overlaps.
pub async fn handle(_state: Arc<McpState>, _input: Input) -> Result<Output, McpError> {
    Err(McpError::internal_error(
        "search_skills handler not yet implemented (US1.b)",
        None,
    ))
}

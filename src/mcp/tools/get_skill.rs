//! `get_skill` MCP tool — input/output schemas + handler.
//!
//! Contract: [`mcp-tools.md` §get_skill](../../../specs/003-phase-3-mcp-workspaces/contracts/mcp-tools.md).
//!
//! US1.a ships the schemas + a stub handler body. US1.c fills in
//! `(catalog, plugin, name)` resolution against the enabled-skills
//! index, SKILL.md read + frontmatter strip, and the recursive
//! resource-file walk.

use std::sync::Arc;

use rmcp::ErrorData as McpError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::mcp::state::McpState;

/// The tool description per `mcp-tools.md` §get_skill lives on the
/// `#[tool]`-decorated method in `mcp::server` as a doc comment.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub catalog: String,
    pub plugin: String,
    /// The skill `name` field as returned by `search_skills`.
    pub name: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Output {
    /// SKILL.md body with YAML frontmatter stripped. Body is otherwise
    /// verbatim — no normalisation, no rewrites, no path-relative-to-
    /// absolute resolution in code blocks.
    pub content: String,
    /// Absolute path to the SKILL.md file.
    pub path: String,
    /// Absolute paths of every OTHER file in the skill's directory
    /// (recursive). The agent may load any of them via its own
    /// file-reading tools.
    pub resources: Vec<String>,
}

/// Handler stub for US1.a. The full resolve + read + strip + walk
/// pipeline lands in US1.c per T088.
pub async fn handle(_state: Arc<McpState>, _input: Input) -> Result<Output, McpError> {
    Err(McpError::internal_error(
        "get_skill handler not yet implemented (US1.c)",
        None,
    ))
}

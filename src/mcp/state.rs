//! Shared state for the MCP server. Constructed by `mcp::run` after the
//! pre-flight succeeds; threaded into every tool handler via the
//! `Server` wrapper in `mcp::server`.
//!
//! Reranker is lazy-loaded on the first `search_skills` call per
//! FR-109; the `tokio::sync::OnceCell` enables async-friendly
//! initialisation without blocking subsequent calls.

use std::sync::Arc;

use tokio::sync::OnceCell;

use crate::embedding::registry::ModelEntry;
use crate::embedding::{Embedder, Reranker};
use crate::mcp::prompts::PromptRegistry;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

pub struct McpState {
    pub embedder: Arc<dyn Embedder>,
    pub reranker: OnceCell<Arc<dyn Reranker>>,
    pub scope: ResolvedScope,
    pub paths: Paths,
    /// Registry entry for the loaded embedder. Used by the
    /// `search_skills` pipeline to record drift / pass identity into
    /// `query::run_with_deps`.
    pub embedder_entry: &'static ModelEntry,
    /// Registry entry for the reranker that will be loaded on first
    /// `search_skills` call.
    pub reranker_entry: &'static ModelEntry,
    /// Phase 5 / US1.b: prompts capability registry. Built once at MCP
    /// server startup from the resolved workspace's enabled-and-user-
    /// invocable entries. Immutable for the session — workspace
    /// switches require a server restart (NFR-008, `list_changed:
    /// false`).
    pub prompt_registry: Arc<PromptRegistry>,
}

//! Shared state for the MCP server. Constructed by `mcp::run` after the
//! pre-flight succeeds; threaded into every tool handler via the
//! `Server` wrapper in `mcp::server`.
//!
//! Reranker is lazy-loaded on the first `search_skills` call per
//! FR-109; the `tokio::sync::OnceCell` enables async-friendly
//! initialisation without blocking subsequent calls.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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
    /// Phase 9 / US3: the harness hosting this MCP server, conveyed by
    /// `tome mcp --harness <name>` (stamped into the `tome mcp` args at
    /// `harness sync`). `None` for a legacy/unstamped config — the `meta`
    /// tool then fails closed (FR-029) rather than guessing a harness.
    pub host_harness: Option<String>,
    /// Phase 10 / US2 (FR-028): per-session search→selection funnel state.
    /// Maps an entry `name` to its 1-indexed rank in the MOST RECENT
    /// `search_skills` result list this session. `get_skill` / `get_skill_info`
    /// look the selected entry up here to attribute a `rank_bucket` on their
    /// `tome.entry_invoked` / `tome.entry_info` events — the bucket is `none`
    /// when no preceding search this session ranked the entry.
    ///
    /// WHY a `Mutex<HashMap>` rather than per-request state: the MCP server is
    /// a long-running session, so the funnel join is across SEPARATE tool calls
    /// (search, then a later get) — the rank must outlive the search handler.
    /// Each search clears + repopulates it (only the latest search's ranks
    /// attribute), so it never grows unbounded. The lock is held only for the
    /// sub-µs clear/insert/lookup; it is never held across an `.await`.
    pub last_search_ranks: Mutex<HashMap<String, u32>>,
}

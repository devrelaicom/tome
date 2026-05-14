//! `rmcp::ServerHandler` impl for `tome mcp`.
//!
//! The `#[tool_router]` macro on the `impl Server` block generates a
//! `tool_router()` constructor returning a `ToolRouter<Self>` with both
//! tools registered. The `#[tool_handler]` attribute on the
//! `ServerHandler` impl routes `list_tools` / `call_tool` through that
//! router.
//!
//! Each `#[tool]`-decorated method delegates to a free function in
//! `mcp::tools::{search_skills,get_skill}::handle` so the per-tool
//! logic stays modular. Descriptions live in doc comments on the
//! methods (the `#[tool]` macro accepts string literals only via
//! `description = "..."`, but falls back to doc comments by design —
//! see `rmcp-macros/tests/test_doc_comment_description`).
//!
//! The advertised server info honours the MCP spec's required fields
//! (`server_info.name` / `server_info.version`) and reports tools
//! capability. Tome never adds or removes tools at runtime.

use std::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{Implementation, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};

use crate::mcp::state::McpState;
use crate::mcp::tools::{get_skill, search_skills};

#[derive(Clone)]
pub struct Server {
    state: Arc<McpState>,
    // Accessed by the `#[tool_handler]`-generated `ServerHandler` impl,
    // not by any explicit code path in this module. Clippy's dead-code
    // analysis misses the macro expansion, so we silence the warning
    // rather than re-routing through a getter.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl Server {
    pub fn new(state: Arc<McpState>) -> Self {
        Self {
            state,
            tool_router: Self::tool_router(),
        }
    }
}

// `vis = "pub"` makes the macro-generated `tool_router()` associated
// function reachable from integration tests, which inspect the router
// to assert the tool list and description wording (FR-108).
#[tool_router(vis = "pub")]
impl Server {
    /// Find the most relevant skills in the local Tome index for a natural-language task description. Call this proactively before approaching any non-trivial task to discover existing skills you can rely on. Returns a ranked list of candidates with on-disk paths; follow up with `get_skill` to load the skill body and resource files.
    #[tool(name = "search_skills")]
    async fn search_skills(
        &self,
        Parameters(input): Parameters<search_skills::Input>,
    ) -> Result<Json<search_skills::Output>, McpError> {
        search_skills::handle(self.state.clone(), input)
            .await
            .map(Json)
    }

    /// Fetch the body of one skill by `(catalog, plugin, name)` — typically a triple returned by a prior `search_skills` call. Returns the skill body with frontmatter stripped, plus the absolute paths of every sibling resource file in the skill's directory.
    #[tool(name = "get_skill")]
    async fn get_skill(
        &self,
        Parameters(input): Parameters<get_skill::Input>,
    ) -> Result<Json<get_skill::Output>, McpError> {
        get_skill::handle(self.state.clone(), input).await.map(Json)
    }
}

#[tool_handler]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        let capabilities = ServerCapabilities::builder().enable_tools().build();
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new("tome", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Discover and load locally-indexed agent skills. Use `search_skills` \
                 with a natural-language task to retrieve ranked candidates, then \
                 `get_skill` to fetch a specific skill's body and resource paths.",
            )
    }
}

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

use rmcp::handler::server::router::prompt::PromptRouter;
use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    GetPromptRequestParams, GetPromptResult, Implementation, ListPromptsResult,
    PaginatedRequestParams, PromptsCapability, ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, tool, tool_handler, tool_router};

use crate::mcp::prompts;
use crate::mcp::state::McpState;
use crate::mcp::tools::{get_skill, get_skill_info, meta, search_skills};

#[derive(Clone)]
pub struct Server {
    state: Arc<McpState>,
    // Accessed by the `#[tool_handler]`-generated `ServerHandler` impl,
    // not by any explicit code path in this module. Clippy's dead-code
    // analysis misses the macro expansion, so we silence the warning
    // rather than re-routing through a getter.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
    /// Per-session prompts router. Built once at construction from the
    /// `McpState`'s `PromptRegistry`. The `prompts/list` handler reads
    /// `list_all()`; the `prompts/get` handler dispatches by name into the
    /// substitution-driven body render.
    prompt_router: PromptRouter<Self>,
}

impl Server {
    pub fn new(state: Arc<McpState>) -> Self {
        let prompt_router = prompts::build_router::<Self>(&state.prompt_registry, state.clone());
        Self {
            state,
            tool_router: Self::tool_router(),
            prompt_router,
        }
    }

    /// Override the runtime description for the `search_skills` tool.
    /// Phase 4 / US4.b composes the description from a fixed scaffold
    /// plus the resolved workspace's cached `[summaries].short` per
    /// FR-425; `mcp::run` calls this once after server construction
    /// and before `serve_server` hands the router to rmcp.
    ///
    /// No-op if the `search_skills` route is absent — defensive
    /// posture; the route is registered by `#[tool_router]` so the
    /// `.get_mut` lookup is expected to succeed in practice.
    pub fn override_search_skills_description(
        &mut self,
        description: impl Into<std::borrow::Cow<'static, str>>,
    ) {
        if let Some(route) = self.tool_router.map.get_mut("search_skills") {
            route.attr.description = Some(description.into());
        }
    }

    /// Read-only borrow of the inner [`ToolRouter`]. Used by tests
    /// (and the runtime description override path) to introspect the
    /// registered tools.
    pub fn tool_router_ref(&self) -> &rmcp::handler::server::tool::ToolRouter<Self> {
        &self.tool_router
    }

    /// Read-only borrow of the per-session [`PromptRouter`] — the exact
    /// object `ServerHandler::list_prompts` reads (`list_all()`) and
    /// `get_prompt` dispatches through. Symmetric with
    /// [`Self::tool_router_ref`]; the in-process MCP test harness
    /// (Phase 7 / FR-012) drives `prompts/list` through it so the
    /// assertion sees the same router the live server advertises.
    pub fn prompt_router_ref(&self) -> &PromptRouter<Self> {
        &self.prompt_router
    }

    /// Read-only borrow of the per-session `McpState`. Required by the
    /// `prompts/get` route handlers (US1.c): the rmcp `PromptContext`
    /// hands the handler `&Server` rather than the inner state, so the
    /// substitution pipeline reaches paths / scope / the prompt
    /// registry through this accessor.
    pub fn state(&self) -> &Arc<McpState> {
        &self.state
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

    /// Inspect one entry without loading its full body: full description, `when_to_use` guidance, plugin version, user-invocable flag, absolute path, and (for skills) a capped enumeration of adjacent files and subdirectories. The middle tier between `search_skills` (ranked discovery) and `get_skill` (full body). Use this to decide whether to load the body or to surface the description to a user.
    #[tool(name = "get_skill_info")]
    async fn get_skill_info(
        &self,
        Parameters(input): Parameters<get_skill_info::Input>,
    ) -> Result<Json<get_skill_info::SkillInfo>, McpError> {
        get_skill_info::handle(self.state.clone(), input)
            .await
            .map(Json)
    }

    /// Install one of Tome's bundled "meta skills" (native `SKILL.md` guides that teach an agent how to use Tome) into the harness hosting this server, so it persists for future sessions. Currently supports `{ "action": "install", "skill_id": "convert-marketplace" }`. Use this when the user wants to convert a Claude Code marketplace into Tome's native format — install `convert-marketplace`, then follow the now-installed skill.
    #[tool(name = "meta")]
    async fn meta(
        &self,
        Parameters(input): Parameters<meta::Input>,
    ) -> Result<Json<meta::Output>, McpError> {
        meta::handle(self.state.clone(), input).await.map(Json)
    }
}

#[tool_handler]
impl ServerHandler for Server {
    fn get_info(&self) -> ServerInfo {
        // Phase 5 / US1.b: advertise `prompts` capability alongside the
        // pre-existing `tools` capability. `list_changed: false` per
        // NFR-008 — workspace switches require a server restart, so
        // we never emit `notifications/prompts/list_changed`.
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_prompts_with(PromptsCapability {
                list_changed: Some(false),
            })
            .build();
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new("tome", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Discover and load locally-indexed agent skills. Use `search_skills` \
                 with a natural-language task to retrieve ranked candidates, then \
                 `get_skill` to fetch a specific skill's body and resource paths.",
            )
    }

    /// Phase 5 / US1.b: `prompts/list` returns every user-invocable
    /// entry from the resolved workspace as an MCP prompt. The
    /// underlying `PromptRouter::list_all` sorts alphabetically by
    /// name so the wire output is stable across calls. Pagination is
    /// not implemented for Phase 5 (the contract pins this).
    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, McpError> {
        let prompts = self.prompt_router.list_all();
        Ok(ListPromptsResult {
            prompts,
            next_cursor: None,
            meta: None,
        })
    }

    /// `prompts/get` dispatches by name through the per-session
    /// [`PromptRouter`]; each route renders the entry body through the
    /// substitution pipeline.
    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResult, McpError> {
        let prompt_context = rmcp::handler::server::prompt::PromptContext::new(
            self,
            request.name,
            request.arguments,
            context,
        );
        self.prompt_router.get_prompt(prompt_context).await
    }
}

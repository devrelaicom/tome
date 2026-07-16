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
    GetPromptRequestParams, GetPromptResult, Implementation, ListPromptsResult, ListToolsResult,
    PaginatedRequestParams, PromptsCapability, ServerCapabilities, ServerInfo, ToolsCapability,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer, ServerHandler, tool, tool_handler, tool_router};

use crate::mcp::prompts;
use crate::mcp::state::McpState;
use crate::mcp::tools::{get_skill, list_catalogs, list_plugins, meta, search_skills, status};

#[derive(Clone)]
pub struct Server {
    state: Arc<McpState>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
    /// Per-session prompts router, swappable by the live-sync watcher.
    /// `list_prompts` / `get_prompt` read it under the read lock; the
    /// watcher rebuilds and swaps it on drift.
    prompt_router: Arc<std::sync::RwLock<PromptRouter<Self>>>,
    /// The live `search_skills` description. Seeded at startup from the
    /// cached workspace summary, swapped by the watcher on drift. The
    /// custom `list_tools` reads it on each call.
    search_desc: Arc<std::sync::RwLock<String>>,
}

impl Server {
    pub fn new(state: Arc<McpState>) -> Self {
        let registry = state
            .prompt_registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let prompt_router = prompts::build_router::<Self>(&registry, state.clone());
        Self {
            state,
            tool_router: Self::tool_router(),
            prompt_router: Arc::new(std::sync::RwLock::new(prompt_router)),
            search_desc: Arc::new(std::sync::RwLock::new(String::new())),
        }
    }

    /// Clone the handles the live-sync watcher needs (called before the
    /// server is moved into `serve_server`).
    pub fn live_sync_cells(
        &self,
    ) -> (
        Arc<std::sync::RwLock<PromptRouter<Self>>>,
        Arc<std::sync::RwLock<String>>,
    ) {
        (self.prompt_router.clone(), self.search_desc.clone())
    }

    /// Override the runtime description for the `search_skills` tool.
    /// Phase 4 / US4.b composes the description from a fixed scaffold
    /// plus the resolved workspace's cached `[summaries].short` per
    /// FR-425; `mcp::run` calls this once after server construction
    /// and before `serve_server` hands the router to rmcp.
    pub fn override_search_skills_description(&mut self, description: impl Into<String>) {
        *self.search_desc.write().unwrap_or_else(|e| e.into_inner()) = description.into();
    }

    /// Read-only borrow of the inner [`ToolRouter`]. Used by tests to
    /// introspect the statically registered tools (without the live
    /// `search_desc` override applied — use `search_desc_snapshot` to
    /// read the active override).
    pub fn tool_router_ref(&self) -> &rmcp::handler::server::tool::ToolRouter<Self> {
        &self.tool_router
    }

    /// Snapshot of the live `search_skills` description override. Empty
    /// string means no override is active (the router's static description
    /// is used). Test-seam only — [`list_tools`] is the production reader.
    #[doc(hidden)]
    pub fn search_desc_snapshot(&self) -> String {
        self.search_desc
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Read-only access to the per-session [`PromptRouter`] — the exact
    /// object `ServerHandler::list_prompts` reads (`list_all()`) and
    /// `get_prompt` dispatches through. The in-process MCP test harness
    /// (Phase 7 / FR-012) drives `prompts/list` through it so the
    /// assertion sees the same router the live server advertises.
    pub fn prompt_router_ref(&self) -> impl std::ops::Deref<Target = PromptRouter<Self>> + '_ {
        self.prompt_router.read().unwrap_or_else(|e| e.into_inner())
    }

    /// The exact tool list `ServerHandler::list_tools` advertises: the
    /// statically registered tools with the live `search_skills`
    /// description override applied when one is active (empty cell ⇒ the
    /// static `#[tool]` doc-comment description stands).
    ///
    /// The production `list_tools` handler delegates here so the wire
    /// output is computed in one place; the in-process MCP test harness
    /// drives this directly (the real `list_tools` requires a
    /// `RequestContext<RoleServer>` that is only obtainable over a live
    /// transport — see the harness header — so the injection branch would
    /// otherwise be untestable in-process).
    pub fn tools_listing(&self) -> Vec<rmcp::model::Tool> {
        let mut tools = self.tool_router.list_all();
        let desc = self
            .search_desc
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if !desc.is_empty()
            && let Some(t) = tools.iter_mut().find(|t| t.name == "search_skills")
        {
            t.description = Some(std::borrow::Cow::Owned(desc));
        }
        tools
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

    /// Fetch one entry by `(catalog, plugin, name)` — typically a triple returned by a prior `search_skills` call. By default returns the entry body with frontmatter stripped and `${TOME_*}` substitutions applied, plus the absolute paths of every sibling resource file. Pass `metadata_only: true` for the cheap middle tier — full description, `when_to_use` guidance, plugin version, user-invocable flag, and (for skills) a capped enumeration of adjacent files — WITHOUT reading or rendering the body. Use `kind` to disambiguate a name shared across skill/command/agent; `name` accepts a `*` wildcard that resolves a unique match (the error lists the available `(name, kind)` entries if it resolves nothing). In body mode, pass `raw: true` to preserve literal `${TOME_*}` tokens (for authoring/conversion) and `include_resource_bodies: true` to inline small text resources as `{ path, content }` (byte-capped per-file and in total).
    ///
    /// Alternatively pass `uri` — a path to a SKILL.md (or its directory), a
    /// `<plugin>:<skill>` or `<catalog>:<plugin>:<skill>` name (delimiter `:`,
    /// `_`, or `__`), or a bare name — INSTEAD of the triple; provide EITHER
    /// `uri` OR the full triple, not both. An ambiguous `uri` returns
    /// `matches` (candidate identities + descriptions) and index-aligned
    /// `next_actions` (exact `get_skill` triples) rather than a body, so you
    /// can pick one and call again.
    #[tool(name = "get_skill")]
    async fn get_skill(
        &self,
        Parameters(input): Parameters<get_skill::Input>,
    ) -> Result<Json<get_skill::Output>, McpError> {
        get_skill::handle(self.state.clone(), input).await.map(Json)
    }

    /// Enumerate the enabled plugins in the resolved workspace and their contents — the skills, commands, and agents each plugin ships, with per-entry index + invocability status. This is the "browse my full toolbox" surface: unlike `search_skills` (ranked semantic discovery), it lists everything available so you can plan against it. Optional filters: `catalog` (restrict to one catalog), `enabled_only` (default true — omit plugins with nothing enabled), `kind` (restrict the listed entries to `skill`/`command`/`agent`). Read-only.
    #[tool(name = "list_plugins")]
    async fn list_plugins(
        &self,
        Parameters(input): Parameters<list_plugins::Input>,
    ) -> Result<Json<list_plugins::Output>, McpError> {
        list_plugins::handle(self.state.clone(), input)
            .await
            .map(Json)
    }

    /// List the catalogs enrolled in the resolved workspace and their metadata — name, source URL, pinned ref, plugin count, and last-synced time. Pair with `list_plugins` to see what each catalog contributes. Read-only.
    #[tool(name = "list_catalogs")]
    async fn list_catalogs(
        &self,
        Parameters(input): Parameters<list_catalogs::Input>,
    ) -> Result<Json<list_catalogs::Output>, McpError> {
        list_catalogs::handle(self.state.clone(), input)
            .await
            .map(Json)
    }

    /// Snapshot of this Tome environment: active workspace, entry counts (skills/commands/agents), models on disk, index freshness, and per-harness MCP integration state. Use it to understand your context or self-diagnose "why did search return nothing" (e.g. an empty index or a drifted embedder). Pass `include_doctor: true` to also fold in the READ-ONLY doctor diagnostic (per-subsystem health + suggested fixes); this never applies any repair. Read-only.
    #[tool(name = "status")]
    async fn status(
        &self,
        Parameters(input): Parameters<status::Input>,
    ) -> Result<Json<status::Output>, McpError> {
        status::handle(self.state.clone(), input).await.map(Json)
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
        // Advertise live-update support for both prompts and tools. A workspace
        // *switch* still requires a restart (the server is stamped with a fixed
        // --workspace); intra-workspace content drift (enable/disable) is picked
        // up by the live-sync watcher, which emits list_changed. (Updates the
        // prior NFR-008 rationale, which assumed a frozen session.)
        let capabilities = ServerCapabilities::builder()
            .enable_tools_with(ToolsCapability {
                list_changed: Some(true),
            })
            .enable_prompts_with(PromptsCapability {
                list_changed: Some(true),
            })
            .build();
        ServerInfo::new(capabilities)
            .with_server_info(Implementation::new("tome", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Discover and load locally-indexed agent skills: `search_skills` \
                 with a natural-language task retrieves ranked candidates, then \
                 `get_skill` fetches a candidate — call it with `metadata_only: \
                 true` first to inspect the description and `when_to_use` guidance \
                 without loading the full body, then again (default) to fetch the \
                 body and resource paths once you've confirmed the match. To browse \
                 the full inventory instead of searching, use `list_plugins` (skills \
                 / commands / agents per plugin) and `list_catalogs`; use `status` \
                 to inspect the environment or self-diagnose why a search returned \
                 nothing.",
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
        let prompts = self
            .prompt_router
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .list_all();
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
        // Clone the current router out of the cell so we don't hold the
        // lock across the `.await` (the render may be slow).
        let router = self
            .prompt_router
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        router.get_prompt(prompt_context).await
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        Ok(ListToolsResult {
            tools: self.tools_listing(),
            next_cursor: None,
            meta: None,
        })
    }
}

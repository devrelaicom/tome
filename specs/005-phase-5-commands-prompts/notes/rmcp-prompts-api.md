# T122 — rmcp prompts API verification (rmcp 1.7.0)

**Verification date**: 2026-05-26
**rmcp version**: 1.7.0 (pinned `rmcp = { version = "1", features = ["transport-io", "schemars"] }` in Cargo.toml)
**rmcp-macros version**: 1.7.0
**Conclusion**: API surface present. **No hard-stop.** Proceeding with US1.b implementation per contracts/mcp-prompts.md with the deviations recorded below.

## What's present in rmcp 1.7.0 (and we will use)

### Model types (rmcp::model::*)

```rust
pub struct Prompt {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub arguments: Option<Vec<PromptArgument>>,
    pub icons: Option<Vec<Icon>>,
    pub meta: Option<Meta>,
}

pub struct PromptArgument {
    pub name: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub required: Option<bool>,
}

pub struct PromptMessage {
    pub role: PromptMessageRole,  // User | Assistant
    pub content: PromptMessageContent,
}

pub enum PromptMessageContent {
    Text { text: String },
    Image { image: ImageContent },
    Resource { resource: EmbeddedResource },
    ResourceLink { link: Resource },
}

pub struct GetPromptResult {
    pub description: Option<String>,
    pub messages: Vec<PromptMessage>,
}

pub struct PromptsCapability {
    pub list_changed: Option<bool>,
}
```

All carry `Serialize + Deserialize + Debug + Clone + PartialEq`. With the `schemars` feature (already enabled in our Cargo.toml), they also derive `JsonSchema`.

### Routing (rmcp::handler::server::router::prompt::*)

```rust
pub struct PromptRouter<S> { ... }
impl<S: MaybeSend + 'static> PromptRouter<S> {
    pub fn new() -> Self;
    pub fn add_route(&mut self, route: PromptRoute<S>);
    pub fn list_all(&self) -> Vec<Prompt>;
    pub fn has_route(&self, name: &str) -> bool;
    pub async fn get_prompt(&self, ctx: PromptContext<S>) -> Result<GetPromptResult, ErrorData>;
}

pub struct PromptRoute<S> { ... }
impl<S: MaybeSend + 'static> PromptRoute<S> {
    pub fn new<H, A: 'static>(attr: impl Into<Prompt>, handler: H) -> Self
        where H: GetPromptHandler<S, A> + MaybeSend + Clone + 'static;

    pub fn new_dyn<H>(attr: impl Into<Prompt>, handler: H) -> Self
        where H: for<'a> Fn(PromptContext<'a, S>)
                    -> MaybeBoxFuture<'a, Result<GetPromptResult, ErrorData>>
              + MaybeSend + 'static;
}
```

### Macro support (rmcp-macros)

- `#[prompt(name = "...", description = "...", arguments = ...)]` — annotates an async fn returning `Result<Vec<PromptMessage>>` or `Result<GetPromptResult>`. The macro generates a `pub fn <name>_prompt() -> rmcp::model::Prompt` companion that returns the Prompt attribute, and a registered route.
- `#[prompt_router]` — applies to an `impl` block, generates a `pub fn prompt_router() -> PromptRouter<Self>` that aggregates every `#[prompt]`-marked fn in the impl.
- `#[prompt_handler]` — stacks alongside `#[tool_handler]` to make the `impl ServerHandler` block route both tools and prompts.

### Wire methods

- `prompts/list` (model const `ListPromptsRequestMethod`)
- `prompts/get` (model const `GetPromptRequestMethod`)
- `notifications/prompts/list_changed` (only used when `list_changed: true` in capabilities)

## Deviations from contracts/mcp-prompts.md + data-model.md

### Deviation 1: Use rmcp's `Prompt` directly instead of a Tome-defined `PromptDescriptor`

The data-model says:
> `PromptDescriptor` | `src/mcp/prompts.rs` | `pub` | rmcp-wire shape

rmcp 1.7.0 already ships `rmcp::model::Prompt` as that wire shape. Defining our own `PromptDescriptor` would force double-marshalling for zero gain. **Decision**: alias

```rust
pub use rmcp::model::Prompt as PromptDescriptor;
pub use rmcp::model::PromptArgument;
pub use rmcp::model::PromptMessage;
pub use rmcp::model::PromptMessageContent as PromptContent;
pub use rmcp::model::PromptMessageRole as PromptRole;
pub use rmcp::model::GetPromptResult as PromptGetResponse;
```

The `tests/mcp_prompts.rs` test surface continues to reference `PromptDescriptor`-by-name (so contract test naming holds).

### Deviation 2: Dynamic registration, NOT `#[prompt_router]` macro

Tome's prompts are NOT known at compile time. They're driven by the resolved workspace's enabled-and-user-invocable entries at MCP startup. The `#[prompt_router]` macro builds a *static* router from compile-time-declared `#[prompt]` functions.

**Decision**: build the router by hand using `PromptRouter::new()` + `add_route(PromptRoute::new_dyn(prompt, handler))`. The `handler` closure receives a `PromptContext<S>` (S = `Server`, our existing tool-handling state) and returns `MaybeBoxFuture<'_, Result<GetPromptResult, ErrorData>>`.

The handler closure for each route looks up the entry by the prompt name (via the `PromptRegistry` per data-model §4.5–4.6), reads the entry body, calls `substitution::render()` (the F3 stub for now), and wraps in `GetPromptResult { description, messages: vec![PromptMessage::new_text(User, rendered)] }`.

### Deviation 3: `PromptListResponse` is not needed as a separate Tome type

`PromptRouter::list_all()` returns `Vec<Prompt>` directly. The router handles the `ListPromptsResult` wire envelope internally — we don't need a Tome `PromptListResponse` wrapper.

### Deviation 4: Capability declaration

`PromptsCapability { list_changed: Some(false) }` declared in our `ServerCapabilities` at MCP startup, per the contract. Look for the existing `ServerCapabilities` construction in `src/mcp/server.rs` (where `tools` capability is declared) and add prompts there.

## What we still need to implement

Per the contract, the Tome-side artefacts are:

1. **`src/mcp/prompt_name.rs`** — `derive_name(entry, override)` + `sanitise(s)` + `sanitise_trunc(s, max)`. Pure functions, no rmcp dependency. (T123)
2. **`src/mcp/prompt_collision.rs`** — `CollisionRecord`, `EntryIdentity`, `resolve_collisions(entries)`. Pure logic. (T124)
3. **`src/mcp/prompts.rs`** — type re-exports (per Deviation 1) + handler glue + `PromptRegistry`. Hooks into rmcp's `PromptRouter`. (T125, T127, T129, T130)
4. **`src/mcp/state.rs`** — extend `McpState` with `prompt_registry: Arc<PromptRegistry>` field. (T126)
5. **`src/mcp/server.rs`** — declare `PromptsCapability { list_changed: Some(false) }` in the capabilities. (T128)
6. **`src/mcp/mod.rs`** — build the `PromptRegistry` at startup (after preflight). (T131)

## Sync-boundary discipline

Per Phase 3's `tests/sync_boundary.rs` and the `src/mcp/*` async island convention, all the work above stays in `src/mcp/`. Handler closures use `tokio::task::spawn_blocking` for any sync work (rusqlite queries to look up entries, fs::read for entry bodies, substitution::render which is sync per F3). The existing tool handlers (`search_skills`, `get_skill`) use this pattern.

## No hard-stop

This verification matches the assumption in tasks.md T122 (the `#[prompt_router]` macro pattern exists). The deviations above are clarifications, not blocks. **Proceeding with US1.b implementation.**

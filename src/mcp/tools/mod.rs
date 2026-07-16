//! MCP tool input/output schemas + handler bodies.
//!
//! Each tool lives in its own submodule exposing:
//!
//! * `Input` / `Output` types derived from `Deserialize` (input),
//!   `Serialize` (output), and `JsonSchema` (both, for `rmcp`'s tool
//!   advertisement).
//! * A `handle(state, input) -> Result<Output, McpError>` async function
//!   that the `#[tool]` macro in `mcp::server` delegates to.
//!
//! The read-only surface (issue #497): `search_skills` (ranked discovery),
//! `get_skill` (consolidated body-fetch + metadata-only introspection),
//! `list_plugins` / `list_catalogs` (inventory browse), `status` (environment
//! snapshot + optional read-only doctor report). Plus the write-capable `meta`
//! tool. Every tool but `meta` is read-only.
//!
//! Each new discovery tool is a thin wrapper over the corresponding CLI compute
//! path (`plugin list`/`show`, `catalog list`, `status`, `doctor` report),
//! `spawn_blocking`-ing the sync work inside the async handler.

pub mod common;
pub mod get_skill;
pub mod list_catalogs;
pub mod list_plugins;
pub mod meta;
pub mod search_skills;
pub mod status;
pub mod uri_resolver;

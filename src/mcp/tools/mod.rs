//! MCP tool input/output schemas + handler bodies.
//!
//! Each tool lives in its own submodule (`search_skills`, `get_skill`)
//! exposing:
//!
//! * `Input` / `Output` types derived from `Deserialize` (input),
//!   `Serialize` (output), and `JsonSchema` (both, for `rmcp`'s tool
//!   advertisement).
//! * A `handle(state, input) -> Result<Output, McpError>` async function
//!   that the `#[tool]` macro in `mcp::server` delegates to.
//!
//! The contract for both tools lives at
//! [`specs/003-phase-3-mcp-workspaces/contracts/mcp-tools.md`].

pub mod common;
pub mod get_skill;
pub mod get_skill_info;
pub mod meta;
pub mod search_skills;

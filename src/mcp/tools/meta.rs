//! `meta` MCP tool — install a bundled meta skill into the host harness.
//!
//! Contract: [`mcp-meta-tool.md`](../../../specs/009-phase-9-meta-skills/contracts/mcp-meta-tool.md).
//!
//! Shares the exact install compute the CLI uses
//! ([`crate::authoring::meta::install_skill`], NFR-005), run under
//! `spawn_blocking` (the install path is sync; this is the async island).
//! The host harness is resolved from [`McpState::host_harness`] (stamped into
//! the `tome mcp` args at `harness sync`); `None` → **fail closed** (FR-029),
//! never a guess.

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::ErrorData as McpError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::authoring::meta as meta_skill;
use crate::commands::harness::home_root;
use crate::harness;
use crate::mcp::state::McpState;

/// The tool action. Only `install` ships; the enum is left open for a future
/// `list`/`get` without a breaking input change.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Install,
}

/// Install scope. Project (default) writes under the resolved project root;
/// global writes under the user home.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    #[default]
    Project,
    Global,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Scope::Project => "project",
            Scope::Global => "global",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// The action to perform. Only `install` is supported.
    pub action: Action,
    /// The bundled skill id (e.g. `convert-marketplace`).
    pub skill_id: String,
    /// `project` (default) or `global`.
    #[serde(default)]
    pub scope: Scope,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct InstalledAt {
    pub harness: String,
    pub scope: String,
    pub dir: String,
    pub revision: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Output {
    pub skill_id: String,
    pub installed_at: InstalledAt,
}

pub async fn handle(state: Arc<McpState>, input: Input) -> Result<Output, McpError> {
    // Only `install` ships; the binding is explicit so a future variant must be
    // handled deliberately rather than silently mis-routed.
    let Action::Install = input.action;

    // (1) Unknown skill → 87 (mirror), before any I/O. P9 contract MINOR:
    // check skill existence FIRST so the CLI↔MCP precedence matches — the CLI
    // rejects an unknown skill (87) before resolving harness targets, so when
    // BOTH an unknown skill AND no host hold, both surfaces return 87.
    if meta_skill::find(&input.skill_id).is_none() {
        return Err(McpError::invalid_params(
            format!("no embedded meta skill with id `{}`", input.skill_id),
            Some(json!({ "code": "meta_skill_not_found", "skill_id": input.skill_id })),
        ));
    }

    // (2) Host harness must be known — fail closed (FR-029), never guess.
    let Some(host) = state.host_harness.clone() else {
        return Err(McpError::invalid_params(
            "this Tome MCP server has no host-harness identity; re-run `tome sync` to \
             stamp it, or install via `tome meta add` from the CLI"
                .to_string(),
            Some(json!({ "code": "no_harness_detected" })),
        ));
    };
    let Some(module) = harness::lookup(&host) else {
        return Err(McpError::invalid_params(
            format!("unknown host harness `{host}`"),
            Some(json!({ "code": "no_harness_detected", "harness": host })),
        ));
    };
    if !module.supports_native_skills() {
        return Err(McpError::invalid_params(
            format!("host harness `{host}` does not consume native skills"),
            Some(json!({ "code": "no_harness_detected", "harness": host })),
        ));
    }

    // (3) Resolve the skills dir for (host, scope).
    let home = home_root().map_err(|e| McpError::internal_error(e.to_string(), None))?;
    let dir: PathBuf = match input.scope {
        Scope::Project => {
            // Unlike the CLI (which fails closed when project scope is unbound),
            // the MCP server treats its LAUNCH CWD as the project root when the
            // resolved scope carries none — a harness launches `tome mcp` in the
            // project dir, so CWD is the right target there. The symlink-safe
            // landing in `install_skill` applies regardless of how `dir` resolves.
            let project_root = match state.scope.project_root.clone() {
                Some(root) => root,
                None => std::env::current_dir()
                    .map_err(|e| McpError::internal_error(e.to_string(), None))?,
            };
            module.skill_dir(&project_root)
        }
        Scope::Global => module.skill_dir_global(&home),
    }
    .ok_or_else(|| {
        McpError::invalid_params(
            format!(
                "host harness `{host}` has no skills dir for scope `{}`",
                input.scope.as_str()
            ),
            Some(json!({ "code": "no_harness_detected" })),
        )
    })?;

    // (4) Install under spawn_blocking (the compute path is sync).
    let skill_id = input.skill_id.clone();
    let dir_for_task = dir.clone();
    let installed =
        tokio::task::spawn_blocking(move || meta_skill::install_skill(&skill_id, &dir_for_task))
            .await
            .map_err(|e| McpError::internal_error(format!("install join: {e}"), None))?
            .map_err(|e| {
                // C-L1: best-effort MCP-surface `tome.error` (closed category
                // only), with this session's `calling_harness`. Never alters the
                // returned `McpError`. This is the `meta` handler's one
                // `TomeError`→`McpError` conversion (the earlier guard arms are
                // non-`TomeError` fail-closed contract codes).
                crate::mcp::enqueue_tool_error(&state, e.category());
                // Carry the CLI exit-code slug (meta_skill_not_found /
                // meta_install_failed) so the MCP error stays consistent.
                McpError::invalid_params(
                    e.to_string(),
                    Some(json!({ "code": e.category().as_str() })),
                )
            })?;

    Ok(Output {
        skill_id: input.skill_id,
        installed_at: InstalledAt {
            harness: host,
            scope: input.scope.as_str().to_string(),
            dir: installed.target_dir.display().to_string(),
            revision: installed.revision,
        },
    })
}

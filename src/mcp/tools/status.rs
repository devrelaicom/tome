//! `status` MCP tool — environment snapshot, optionally with the doctor report.
//!
//! Issue #497. Read-only introspection: mirrors `tome status --json`, and with
//! `include_doctor: true` folds in the READ-ONLY doctor diagnostic
//! (`tome doctor --json`, NEVER `--fix`). Lets an agent understand its context
//! and self-diagnose "why did search return nothing".
//!
//! Reuses the exact compute paths the CLI uses:
//! * [`crate::commands::status::full_report`] — the same read-only report
//!   `tome status` renders (never takes the advisory lock, never downloads).
//! * [`crate::doctor::assemble_report`] — the READ-ONLY doctor projection. The
//!   `--fix` repair path (`doctor::fixes::apply`) is NEVER reached from here.
//!
//! `doctor::assemble_report` is called with `verify = false` so the tool stays
//! purely local — no model rehashing, no provider network round-trip, and no
//! `tome mcp` subprocess probe. The sync compute runs inside `spawn_blocking`.

use std::sync::Arc;
use std::time::Instant;

use rmcp::ErrorData as McpError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{error, info};

use crate::error::{ErrorCategory, TomeError};
use crate::mcp::state::McpState;
use crate::mcp::tools::common::error_data;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// Fold in the READ-ONLY doctor diagnostic (`tome doctor`, never `--fix`) so
    /// a single call surfaces both the status snapshot and per-subsystem
    /// findings + suggested fixes. Default false — the doctor report is heavier
    /// (it walks every subsystem). Never runs the model / provider / MCP-probe
    /// verification (that is the CLI's `--verify`, not exposed here).
    #[serde(default)]
    pub include_doctor: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Output {
    /// The `tome status` report (byte-identical to `tome status --json`).
    pub status: Value,
    /// The READ-ONLY `tome doctor` report (byte-identical to `tome doctor
    /// --json` without `--verify`). Present only when `include_doctor` is true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doctor: Option<Value>,
}

pub async fn handle(state: Arc<McpState>, input: Input) -> Result<Output, McpError> {
    let started = Instant::now();

    let paths = state.paths.clone();
    let scope = state.scope.clone();
    let include_doctor = input.include_doctor;

    let result = tokio::task::spawn_blocking(move || pipeline(&paths, &scope, include_doctor))
        .await
        .map_err(|e| {
            internal(
                started,
                format!("status join: {e}"),
                ErrorCategory::Internal,
            )
        })?
        .map_err(|e| {
            crate::mcp::enqueue_tool_error(&state, e.category());
            internal(started, e.to_string(), e.category())
        })?;

    info!(
        target: "tome::mcp::tools::status",
        include_doctor,
        result = "ok",
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );

    Ok(result)
}

/// Silent compute: assemble the status report and, when requested, the READ-ONLY
/// doctor report. Both are serialised to `serde_json::Value` so the tool's
/// output schema stays a thin JSON wrapper (the CLI report structs are the SSOT
/// for the fields). No advisory lock; no verification network/subprocess.
fn pipeline(
    paths: &crate::paths::Paths,
    scope: &crate::workspace::ResolvedScope,
    include_doctor: bool,
) -> Result<Output, TomeError> {
    // The same read-only report `tome status` renders (harness/config fills
    // included). `verify = false`: never rehash model artefacts.
    let status_report = crate::commands::status::full_report(scope, paths, false)?;
    let status = serde_json::to_value(&status_report).map_err(|e| {
        TomeError::IndexIntegrityCheckFailure(format!("serialise status report: {e}"))
    })?;

    let doctor = if include_doctor {
        // Resolve HOME via the validated resolver every meta path uses.
        let home = crate::commands::harness::home_root()?;
        // READ-ONLY doctor projection: `assemble_report` alone (NOT
        // `doctor::fixes::apply`). `verify = false` keeps it local — no model
        // rehash, no provider network round-trip, no `tome mcp` subprocess.
        let report = crate::doctor::assemble_report(scope, paths, &home, false)?;
        let value = serde_json::to_value(&report).map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("serialise doctor report: {e}"))
        })?;
        Some(value)
    } else {
        None
    };

    Ok(Output { status, doctor })
}

fn internal(started: Instant, msg: String, category: ErrorCategory) -> McpError {
    let scrubbed = crate::catalog::git::scrub_to_string(msg.as_bytes());
    error!(
        target: "tome::mcp::tools::status",
        error_code = category.as_str(),
        error_message = %scrubbed,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "tool error",
    );
    McpError::internal_error(msg, Some(error_data(category)))
}

//! `list_catalogs` MCP tool — list enrolled catalogs and their metadata.
//!
//! Issue #497. Read-only discovery: mirrors `tome catalog list`. Kept as its
//! own tool (deliberately NOT folded into a generic `list`, to avoid confusion
//! with `list_plugins`).
//!
//! Reuses [`crate::index::workspace_catalogs::list_for_workspace`] (the sole
//! source of truth for catalog enrolment) and the shared catalog-manifest
//! reader for the plugin count. Opens the index READ-ONLY with NO advisory
//! lock; the sync work runs inside `spawn_blocking`.

use std::sync::Arc;
use std::time::Instant;

use rmcp::ErrorData as McpError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::error::{ErrorCategory, TomeError};
use crate::mcp::state::McpState;
use crate::mcp::tools::common::error_data;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {}

/// One enrolled catalog.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CatalogEntry {
    pub name: String,
    /// The catalog's source URL (credentials are never persisted here).
    pub url: String,
    /// The pinned ref (branch / tag / commit) the enrolment tracks.
    #[serde(rename = "ref")]
    pub ref_: String,
    /// Number of plugins the cached catalog manifest declares. `0` when the
    /// on-disk clone is absent or its manifest is unreadable.
    pub plugin_count: usize,
    /// The clone directory's last-modified time (RFC 3339), when resolvable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Output {
    /// The resolved workspace these catalogs are enrolled in.
    pub workspace: String,
    pub catalogs: Vec<CatalogEntry>,
}

pub async fn handle(state: Arc<McpState>, _input: Input) -> Result<Output, McpError> {
    let started = Instant::now();

    let paths = state.paths.clone();
    let scope = state.scope.scope.clone();

    let result = tokio::task::spawn_blocking(move || pipeline(&paths, &scope))
        .await
        .map_err(|e| {
            internal(
                started,
                format!("list_catalogs join: {e}"),
                ErrorCategory::Internal,
            )
        })?
        .map_err(|e| {
            crate::mcp::enqueue_tool_error(&state, e.category());
            internal(started, e.to_string(), e.category())
        })?;

    info!(
        target: "tome::mcp::tools::list_catalogs",
        catalog_count = result.catalogs.len(),
        result = "ok",
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );

    Ok(result)
}

/// Silent compute: list the resolved workspace's catalog enrolments and, for
/// each, best-effort the on-disk plugin count + clone mtime. No advisory lock.
fn pipeline(
    paths: &crate::paths::Paths,
    scope: &crate::workspace::Scope,
) -> Result<Output, TomeError> {
    let conn = crate::commands::plugin::open_index_for_read(paths, scope)?;
    let workspace_name = scope.name().as_str().to_owned();

    let enrolments = crate::index::workspace_catalogs::list_for_workspace(&conn, &workspace_name)?;

    let catalogs = enrolments
        .into_iter()
        .map(|e| {
            let cache_dir = paths.cache_dir_for(&e.url);
            let plugin_count = crate::catalog::manifest::read_catalog_manifest(&cache_dir)
                .map(|m| m.plugins.len())
                .unwrap_or(0);
            let last_synced = clone_mtime(&cache_dir);
            CatalogEntry {
                name: e.catalog_name,
                url: e.url,
                ref_: e.pinned_ref,
                plugin_count,
                last_synced,
            }
        })
        .collect();

    Ok(Output {
        workspace: workspace_name,
        catalogs,
    })
}

/// The clone directory's mtime as an RFC 3339 string, best-effort.
fn clone_mtime(cache_dir: &std::path::Path) -> Option<String> {
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    let meta = std::fs::metadata(cache_dir).ok()?;
    let systime = meta.modified().ok()?;
    OffsetDateTime::from(systime).format(&Rfc3339).ok()
}

fn internal(started: Instant, msg: String, category: ErrorCategory) -> McpError {
    let scrubbed = crate::catalog::git::scrub_to_string(msg.as_bytes());
    error!(
        target: "tome::mcp::tools::list_catalogs",
        error_code = category.as_str(),
        error_message = %scrubbed,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "tool error",
    );
    McpError::internal_error(msg, Some(error_data(category)))
}

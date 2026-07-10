//! `list_plugins` MCP tool — enumerate enabled plugins and their contents.
//!
//! Issue #497. Read-only discovery: mirrors `tome plugin list` +
//! `tome plugin show` so an agent can browse its full toolbox (skills /
//! commands / agents per plugin, with per-entry index status) rather than
//! reaching entries only through semantic search.
//!
//! Reuses the shared, read-only index helpers the CLI `plugin list` / `plugin
//! show` compute paths use ([`crate::commands::plugin::discoverable_plugin_ids`],
//! [`crate::index::skills::list_for_plugin`]) — the enumeration is opened
//! READ-ONLY with NO advisory lock (the `plugin list` invariant). The sync work
//! runs inside `spawn_blocking` per the async-island discipline.

use std::sync::Arc;
use std::time::Instant;

use rmcp::ErrorData as McpError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::{error, info};

use crate::error::{ErrorCategory, TomeError};
use crate::mcp::state::McpState;
use crate::mcp::tools::common::error_data;
use crate::plugin::identity::EntryKind;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    /// Restrict to one catalog by name (must match an enabled catalog in the
    /// resolved scope). When absent, every enrolled catalog is enumerated.
    #[serde(default)]
    pub catalog: Option<String>,
    /// When true (the default), only plugins with at least one enabled entry in
    /// the resolved workspace are returned. Set false to also list plugins that
    /// are discoverable but have nothing enabled.
    #[serde(default = "default_enabled_only")]
    pub enabled_only: bool,
    /// Restrict the enumerated ENTRIES to one kind (`skill`, `command`, or
    /// `agent`). Plugins with no entry of that kind are omitted. When absent,
    /// all kinds are listed.
    #[serde(default)]
    pub kind: Option<EntryKind>,
}

fn default_enabled_only() -> bool {
    true
}

/// One enumerated plugin plus its entries.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PluginEntry {
    pub catalog: String,
    pub plugin: String,
    /// The plugin's version as recorded in the index, when at least one of its
    /// entries is indexed (absent for a discoverable-but-never-indexed plugin).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The number of the plugin's entries enabled in the resolved workspace.
    pub enabled_entries: u32,
    /// Every entry the plugin ships (subject to the `enabled_only` / `kind`
    /// filters), each with its per-entry index + invocability state.
    pub entries: Vec<Entry>,
}

/// One skill / command / agent within a plugin.
#[derive(Debug, Serialize, JsonSchema)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    pub description: String,
    /// Whether this entry is enabled in the resolved workspace.
    pub enabled: bool,
    /// Whether this entry is searchable (feeds `search_skills`).
    pub searchable: bool,
    /// Whether this entry is user-invocable (exposed as an MCP prompt).
    pub user_invocable: bool,
    /// The entry's last-indexed timestamp (RFC 3339), when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_at: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Output {
    /// The resolved workspace these plugins are enumerated for.
    pub workspace: String,
    pub plugins: Vec<PluginEntry>,
}

pub async fn handle(state: Arc<McpState>, input: Input) -> Result<Output, McpError> {
    let started = Instant::now();

    let paths = state.paths.clone();
    let scope = state.scope.scope.clone();

    let result = tokio::task::spawn_blocking(move || pipeline(&paths, &scope, &input))
        .await
        .map_err(|e| {
            internal(
                started,
                format!("list_plugins join: {e}"),
                ErrorCategory::Internal,
            )
        })?
        .map_err(|e| {
            // C-L1: best-effort MCP-surface `tome.error` (closed category only).
            crate::mcp::enqueue_tool_error(&state, e.category());
            internal(started, e.to_string(), e.category())
        })?;

    info!(
        target: "tome::mcp::tools::list_plugins",
        plugin_count = result.plugins.len(),
        result = "ok",
        elapsed_ms = started.elapsed().as_millis() as u64,
        "call",
    );

    Ok(result)
}

/// Silent compute: enumerate the plugins for the resolved scope, join each with
/// its index-side entries, and apply the `catalog` / `enabled_only` / `kind`
/// filters. No I/O beyond the read-only index open; never the advisory lock.
///
/// The plugin SET is sourced primarily from the index (the enabled
/// `(catalog, plugin)` pairs in the resolved workspace — the SSOT for "enabled
/// plugins"). With `enabled_only == false` it is unioned with the catalog-
/// manifest-declared `discoverable_plugin_ids`, so a plugin that is declared but
/// has nothing enabled still appears. Deriving the enabled set from the index
/// (not the catalog manifest) means the enumeration works even when the on-disk
/// catalog manifest is absent/unreadable — the same lenient posture the rest of
/// the read-side tools take.
fn pipeline(
    paths: &crate::paths::Paths,
    scope: &crate::workspace::Scope,
    input: &Input,
) -> Result<Output, TomeError> {
    // Read-only index open (bootstraps a fresh DB if absent, then re-opens
    // read-only), never the advisory lock — the `plugin list` invariant.
    let conn = crate::commands::plugin::open_index_for_read(paths, scope)?;
    let workspace_name = scope.name().as_str().to_owned();

    // Base set: the enabled `(catalog, plugin)` pairs in this workspace (from the
    // index — the SSOT for "enabled plugins", sorted `(catalog, plugin)`).
    let mut ids: Vec<crate::plugin::PluginId> =
        crate::index::skills::enabled_plugins_for_workspace(&conn, &workspace_name)?
            .into_iter()
            .map(|(catalog, plugin)| crate::plugin::PluginId { catalog, plugin })
            .collect();

    // With `enabled_only == false`, also surface declared-but-unindexed plugins
    // (the catalog-manifest-declared set), unioned in without duplicating.
    if !input.enabled_only {
        let declared =
            crate::commands::plugin::discoverable_plugin_ids(&conn, paths, &workspace_name)?;
        for id in declared {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        ids.sort_by(|a, b| {
            a.catalog
                .cmp(&b.catalog)
                .then_with(|| a.plugin.cmp(&b.plugin))
        });
    }

    if let Some(catalog) = &input.catalog {
        ids.retain(|id| &id.catalog == catalog);
    }

    let mut plugins: Vec<PluginEntry> = Vec::new();
    for id in ids {
        // Every row for the plugin (all kinds, enabled + disabled), in the
        // shared `(kind, name)` order.
        let rows =
            crate::index::skills::list_for_plugin(&conn, &workspace_name, &id.catalog, &id.plugin)?;

        let version = rows.first().map(|r| r.plugin_version.clone());

        let mut entries: Vec<Entry> = Vec::new();
        let mut enabled_entries: u32 = 0;
        for row in rows {
            if row.enabled {
                enabled_entries = enabled_entries.saturating_add(1);
            }
            // Filters: kind (entry-level) and enabled_only (entry-level — a
            // disabled entry is dropped when enabled_only is set, mirroring the
            // CLI's `--enabled-only`).
            if let Some(kind) = input.kind
                && row.kind != kind
            {
                continue;
            }
            if input.enabled_only && !row.enabled {
                continue;
            }
            entries.push(Entry {
                name: row.name,
                kind: row.kind,
                description: row.description,
                enabled: row.enabled,
                searchable: row.searchable,
                user_invocable: row.user_invocable,
                indexed_at: (!row.indexed_at.is_empty()).then_some(row.indexed_at),
            });
        }

        // With enabled_only set, a plugin whose enabled entries were all
        // filtered out (or that has none) is omitted entirely — the CLI's
        // "a plugin that fails any filter is simply absent" rule.
        if input.enabled_only && enabled_entries == 0 {
            continue;
        }
        // A kind filter that leaves no entries also drops the plugin.
        if input.kind.is_some() && entries.is_empty() {
            continue;
        }

        plugins.push(PluginEntry {
            catalog: id.catalog,
            plugin: id.plugin,
            version,
            enabled_entries,
            entries,
        });
    }

    Ok(Output {
        workspace: workspace_name,
        plugins,
    })
}

fn internal(started: Instant, msg: String, category: ErrorCategory) -> McpError {
    let scrubbed = crate::catalog::git::scrub_to_string(msg.as_bytes());
    error!(
        target: "tome::mcp::tools::list_plugins",
        error_code = category.as_str(),
        error_message = %scrubbed,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "tool error",
    );
    McpError::internal_error(msg, Some(error_data(category)))
}

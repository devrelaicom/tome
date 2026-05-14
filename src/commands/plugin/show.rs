//! `tome plugin show <catalog>/<plugin>`.
//!
//! Renders one plugin's metadata + component breakdown + index state.
//!
//! Spec: `contracts/plugin-commands.md` §4.

use std::io::Write;
use std::str::FromStr;

use comfy_table::{Cell, CellAlignment};

use crate::catalog::store;
use crate::cli::PluginShowArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::paths::Paths;
use crate::plugin::components::count_components;
use crate::plugin::manifest::{manifest_path_for, parse_plugin_manifest};
use crate::plugin::{PluginId, PluginRecord, PluginStatus};
use crate::presentation::{colour, tables};
use crate::workspace::ResolvedScope;

use super::{aggregate_for_plugin, human_relative, open_index_for_read, resolve_plugin_dir};

pub fn run(args: PluginShowArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let id = PluginId::from_str(&args.id)
        .map_err(|e| TomeError::Usage(format!("invalid plugin id `{}`: {e}", args.id)))?;

    let paths = Paths::resolve()?;
    let config = store::load(&paths.config_file_for(&scope.scope))?;
    let plugin_dir = resolve_plugin_dir(&id, &config)?;

    // Strict failure here: the contract says exit 22 on a malformed manifest.
    let manifest = parse_plugin_manifest(&manifest_path_for(&plugin_dir))?;
    let component_counts = count_components(&plugin_dir);

    let conn = open_index_for_read(&paths, &scope.scope)?;
    let agg = aggregate_for_plugin(&conn, &id.catalog, &id.plugin)?;

    let status = if agg.total == 0 {
        PluginStatus::Disabled
    } else if agg.enabled > 0 {
        PluginStatus::Enabled
    } else {
        PluginStatus::Disabled
    };

    let last_indexed_at_dt = agg.last_indexed_at.as_deref().and_then(|s| {
        use time::OffsetDateTime;
        use time::format_description::well_known::Rfc3339;
        OffsetDateTime::parse(s, &Rfc3339).ok()
    });

    let record = PluginRecord {
        id: id.clone(),
        version: manifest.version.clone().unwrap_or_default(),
        author: manifest.author.as_ref().and_then(|a| a.display()),
        description: manifest.description.clone(),
        // Follow-up: derive from `git log -1 -- <plugin source>` against the
        // catalog cache. Phase 2 has no git-log integration yet.
        last_upstream_change: None,
        status,
        component_counts,
        last_indexed_at: last_indexed_at_dt,
    };

    match mode {
        Mode::Human => emit_human(&record, &agg),
        Mode::Json => crate::output::write_json(&record),
    }
}

fn emit_human(record: &PluginRecord, agg: &super::IndexAggregate) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "Plugin:       {}", record.id)?;
    writeln!(
        out,
        "Version:      {}",
        if record.version.is_empty() {
            "—".to_owned()
        } else {
            record.version.clone()
        }
    )?;

    let status_line = match record.status {
        PluginStatus::Enabled => {
            let when = agg
                .last_indexed_at
                .as_deref()
                .map(human_relative)
                .unwrap_or_else(|| "—".to_owned());
            format!("{} enabled (last indexed {})", colour::success("✓"), when)
        }
        PluginStatus::Disabled => format!("{} disabled", colour::error("✗")),
        PluginStatus::Unindexable => format!("{} unindexable", colour::warning("⚠")),
    };
    writeln!(out, "Status:       {}", status_line)?;

    let last_updated = record
        .last_upstream_change
        .map(|_| "—".to_owned())
        .unwrap_or_else(|| "—".to_owned());
    let author = record.author.clone().unwrap_or_else(|| "—".to_owned());
    writeln!(out, "Last updated: {} — {}", last_updated, author)?;

    if let Some(desc) = &record.description {
        writeln!(out, "Description:  {}", desc)?;
    }

    writeln!(out)?;
    writeln!(out, "Component breakdown:")?;

    let mut table = tables::new_table();
    table.set_header(vec![
        Cell::new("Component"),
        Cell::new("Count").set_alignment(CellAlignment::Right),
    ]);
    let counts = &record.component_counts;
    let rows = [
        ("Skills", counts.skills),
        ("Agents", counts.agents),
        ("Commands", counts.commands),
        ("Hooks", counts.hooks),
        ("MCP servers", counts.mcp_servers),
    ];
    for (label, n) in rows {
        table.add_row(vec![
            Cell::new(label),
            Cell::new(n.to_string()).set_alignment(CellAlignment::Right),
        ]);
    }
    writeln!(out, "{table}")?;
    Ok(())
}

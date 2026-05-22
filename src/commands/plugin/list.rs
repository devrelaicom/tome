//! `tome plugin list`.
//!
//! Walks every registered catalog (or just one when `--catalog` is given),
//! joins each declared plugin with index state, and renders a table or
//! NDJSON. No DB writes.
//!
//! Spec: `contracts/plugin-commands.md` §3.

use std::io::Write;

use comfy_table::{Cell, CellAlignment};
use serde::Serialize;

use crate::catalog::store;
use crate::cli::PluginListArgs;
use crate::config::Config;
use crate::error::TomeError;
use crate::output::Mode;
use crate::paths::Paths;
use crate::plugin::components::count_components;
use crate::plugin::manifest::{manifest_path_for, parse_plugin_manifest};
use crate::plugin::{PluginId, PluginRecord, PluginStatus};
use crate::presentation::{colour, tables};
use crate::workspace::ResolvedScope;

use super::{
    IndexAggregate, aggregate_for_plugin, human_relative, open_index_for_read,
    read_catalog_manifest,
};

pub fn run(args: PluginListArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    // F2a: single global config; F11b reintroduces workspace-aware view
    // of catalog enrolment. F11a already threads the resolved workspace
    // through the per-plugin aggregate (`workspace_skills` join).
    let config = store::load(&paths.global_config_file)?;

    let conn = open_index_for_read(&paths, &scope.scope)?;
    let rows = collect_rows(&config, &args, &conn, &paths, scope.scope.name().as_str())?;

    let filtered: Vec<Row> = if args.enabled_only {
        rows.into_iter()
            .filter(|r| r.status == PluginStatus::Enabled)
            .collect()
    } else {
        rows
    };

    match mode {
        Mode::Human => emit_human(&filtered),
        Mode::Json => emit_json(&filtered),
    }
}

/// One row in the human table / one NDJSON record. Stored separately from
/// `PluginRecord` so the two surfaces can diverge without churn.
struct Row {
    id: PluginId,
    version: Option<String>,
    status: PluginStatus,
    skill_count: Option<u32>,
    last_indexed_at: Option<String>,
    record: PluginRecord,
}

fn collect_rows(
    config: &Config,
    args: &PluginListArgs,
    conn: &rusqlite::Connection,
    _paths: &Paths,
    workspace_name: &str,
) -> Result<Vec<Row>, TomeError> {
    let mut out: Vec<Row> = Vec::new();

    let catalog_iter: Vec<&str> = match &args.catalog {
        Some(name) => {
            if !config.catalogs.contains_key(name) {
                return Err(TomeError::CatalogNotFound(name.clone()));
            }
            vec![name.as_str()]
        }
        None => config.catalogs.keys().map(String::as_str).collect(),
    };

    for catalog_name in catalog_iter {
        let Some(entry) = config.catalogs.get(catalog_name) else {
            continue;
        };
        let Some(manifest) = read_catalog_manifest(&entry.path) else {
            continue;
        };

        for plugin in &manifest.plugins {
            let id = PluginId {
                catalog: entry.name.clone(),
                plugin: plugin.name.clone(),
            };
            let plugin_dir = entry.path.join(&plugin.source);

            let row = build_row(&id, &plugin_dir, conn, workspace_name)?;
            out.push(row);
        }
    }

    out.sort_by(|a, b| {
        a.id.catalog
            .cmp(&b.id.catalog)
            .then_with(|| a.id.plugin.cmp(&b.id.plugin))
    });
    Ok(out)
}

fn build_row(
    id: &PluginId,
    plugin_dir: &std::path::Path,
    conn: &rusqlite::Connection,
    workspace_name: &str,
) -> Result<Row, TomeError> {
    // Lenient parse — failures fall through to `Unindexable`.
    let manifest = parse_plugin_manifest(&manifest_path_for(plugin_dir)).ok();
    let component_counts = count_components(plugin_dir);

    let agg: IndexAggregate = aggregate_for_plugin(conn, workspace_name, &id.catalog, &id.plugin)?;

    let (status, skill_count, version) = match &manifest {
        None => (PluginStatus::Unindexable, None, None),
        Some(m) => {
            let status = if agg.total == 0 {
                PluginStatus::Disabled
            } else if agg.enabled > 0 {
                PluginStatus::Enabled
            } else {
                PluginStatus::Disabled
            };
            let skill_count = u32::try_from(agg.total).ok();
            (status, skill_count, m.version.clone())
        }
    };

    // Build the JSON record alongside the table row so both surfaces share
    // a single source of truth. `last_upstream_change` is left as None
    // pending git log integration in a follow-up.
    let last_indexed_at_dt = agg.last_indexed_at.as_deref().and_then(|s| {
        use time::OffsetDateTime;
        use time::format_description::well_known::Rfc3339;
        OffsetDateTime::parse(s, &Rfc3339).ok()
    });
    let record = PluginRecord {
        id: id.clone(),
        version: version.clone().unwrap_or_default(),
        author: manifest
            .as_ref()
            .and_then(|m| m.author.as_ref().and_then(|a| a.display())),
        description: manifest.as_ref().and_then(|m| m.description.clone()),
        last_upstream_change: None,
        status,
        component_counts,
        last_indexed_at: last_indexed_at_dt,
    };

    Ok(Row {
        id: id.clone(),
        version,
        status,
        skill_count,
        last_indexed_at: agg.last_indexed_at,
        record,
    })
}

fn emit_human(rows: &[Row]) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    if rows.is_empty() {
        writeln!(out, "No plugins found.")?;
        return Ok(());
    }

    let mut table = tables::new_table();
    table.set_header(vec![
        Cell::new("Catalog"),
        Cell::new("Plugin"),
        Cell::new("Version"),
        Cell::new("Status"),
        Cell::new("Skills").set_alignment(CellAlignment::Right),
        Cell::new("Last indexed"),
    ]);

    for r in rows {
        let version = r.version.clone().unwrap_or_else(|| "—".to_owned());
        let status_cell = render_status(r.status);
        let skills = r
            .skill_count
            .map(|n| n.to_string())
            .unwrap_or_else(|| "—".to_owned());
        let last_indexed = r
            .last_indexed_at
            .as_deref()
            .map(human_relative)
            .unwrap_or_else(|| "—".to_owned());

        table.add_row(vec![
            Cell::new(&r.id.catalog),
            Cell::new(&r.id.plugin),
            Cell::new(version),
            Cell::new(status_cell),
            Cell::new(skills).set_alignment(CellAlignment::Right),
            Cell::new(last_indexed),
        ]);
    }

    writeln!(out, "{table}")?;
    Ok(())
}

fn render_status(status: PluginStatus) -> String {
    match status {
        PluginStatus::Enabled => format!("{} enabled", colour::success("✓")),
        PluginStatus::Disabled => format!("{} disabled", colour::error("✗")),
        PluginStatus::Unindexable => format!("{} unindexable", colour::warning("⚠")),
    }
}

#[derive(Serialize)]
struct JsonRow<'a> {
    #[serde(flatten)]
    record: &'a PluginRecord,
}

fn emit_json(rows: &[Row]) -> Result<(), TomeError> {
    for r in rows {
        let env = JsonRow { record: &r.record };
        crate::output::write_json(&env)?;
    }
    Ok(())
}

//! `tome plugin list`.
//!
//! Walks every registered catalog (or just one when `--catalog` is given),
//! joins each declared plugin with index state, and renders a table or
//! NDJSON. No DB writes.
//!
//! Spec: `contracts/plugin-commands.md` §3.

use std::io::Write;

use comfy_table::Cell;
use serde::Serialize;

use crate::cli::PluginListArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::paths::Paths;
use crate::plugin::components::count_components;
use crate::plugin::manifest::read_plugin_manifest;
use crate::plugin::{PluginId, PluginRecord, PluginStatus};
use crate::presentation::{colour, tables};
use crate::workspace::ResolvedScope;

use super::{
    IndexAggregate, PerKindCounts, aggregate_for_plugin, human_relative, open_index_for_read,
    per_kind_counts_for_plugin, read_catalog_manifest,
};

pub fn run(args: PluginListArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;

    // FF2: catalog enrolment is sourced from the `workspace_catalogs` DB,
    // not `config.toml [catalogs]` — the latter is never written in
    // production (`tome catalog add` enrols only into the DB), so reading it
    // here surfaced an empty list on a fresh install. F11a already threads
    // the resolved workspace through the per-plugin aggregate.
    let conn = open_index_for_read(&paths, &scope.scope)?;
    let rows = collect_rows(&args, &conn, &paths, scope.scope.name().as_str())?;

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
///
/// Phase 5 / US5.b: `per_kind` carries the split skills/commands
/// counts so the human-table renderer can produce
/// `(<n> skills, <m> commands)` per
/// `contracts/catalog-and-plugin-extensions-p5.md` § `tome plugin list`.
struct Row {
    id: PluginId,
    version: Option<String>,
    status: PluginStatus,
    per_kind: PerKindCounts,
    last_indexed_at: Option<String>,
    record: PluginRecord,
}

fn collect_rows(
    args: &PluginListArgs,
    conn: &rusqlite::Connection,
    paths: &Paths,
    workspace_name: &str,
) -> Result<Vec<Row>, TomeError> {
    use crate::index::workspace_catalogs;

    let mut out: Vec<Row> = Vec::new();

    // FF2: catalog enrolment now comes from `workspace_catalogs`. A
    // `--catalog <name>` filter resolves one enrolment (miss → exit 3); the
    // bare form iterates every enrolment in the resolved workspace. The
    // on-disk catalog root is the content-addressed clone dir derived from
    // the enrolment URL, replacing the old `config.catalogs[name].path`.
    let enrolments = match &args.catalog {
        Some(name) => vec![
            workspace_catalogs::find(conn, workspace_name, name)?
                .ok_or_else(|| TomeError::CatalogNotFound(name.clone()))?,
        ],
        None => workspace_catalogs::list_for_workspace(conn, workspace_name)?,
    };

    for enrolment in &enrolments {
        let clone_dir = paths.cache_dir_for(&enrolment.url);
        let Some(manifest) = read_catalog_manifest(&clone_dir) else {
            continue;
        };

        for plugin in &manifest.plugins {
            let id = PluginId {
                catalog: enrolment.catalog_name.clone(),
                plugin: plugin.name.clone(),
            };
            let plugin_dir = clone_dir.join(&plugin.source);

            let row = build_row(
                &id,
                &plugin_dir,
                &clone_dir,
                &plugin.source,
                conn,
                workspace_name,
            )?;
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
    clone_dir: &std::path::Path,
    rel_source: &str,
    conn: &rusqlite::Connection,
    workspace_name: &str,
) -> Result<Row, TomeError> {
    // Lenient parse — failures (unconverted or malformed) fall through to
    // `Unindexable` so `list` still renders the row.
    let manifest = read_plugin_manifest(plugin_dir).ok();
    let component_counts = count_components(plugin_dir);

    let agg: IndexAggregate = aggregate_for_plugin(conn, workspace_name, &id.catalog, &id.plugin)?;
    let per_kind = per_kind_counts_for_plugin(conn, workspace_name, &id.catalog, &id.plugin)?;

    let (status, version) = match &manifest {
        None => (PluginStatus::Unindexable, None),
        Some(m) => {
            let status = if agg.total == 0 {
                PluginStatus::Disabled
            } else if agg.enabled > 0 {
                PluginStatus::Enabled
            } else {
                PluginStatus::Disabled
            };
            (status, Some(m.version.clone()))
        }
    };

    // Build the JSON record alongside the table row so both surfaces share
    // a single source of truth. `last_upstream_change` is populated at DISPLAY
    // time from the catalog clone's git history (best-effort, degrades to None).
    let last_indexed_at_dt = agg.last_indexed_at.as_deref().and_then(|s| {
        use time::OffsetDateTime;
        use time::format_description::well_known::Rfc3339;
        OffsetDateTime::parse(s, &Rfc3339).ok()
    });
    let last_upstream_change =
        super::last_upstream_change_at_display(clone_dir, &id.catalog, rel_source);
    let record = PluginRecord {
        id: id.clone(),
        version: version.clone().unwrap_or_default(),
        author: manifest
            .as_ref()
            .and_then(|m| m.author.as_ref().and_then(|a| a.display())),
        description: manifest.as_ref().and_then(|m| m.description.clone()),
        last_upstream_change,
        status,
        component_counts,
        last_indexed_at: last_indexed_at_dt,
    };

    Ok(Row {
        id: id.clone(),
        version,
        status,
        per_kind,
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
        // Phase 5 / US5.b: was "Skills"; now reports both kinds.
        Cell::new("Entries"),
        Cell::new("Last indexed"),
        // #309: the upstream committer date for the plugin's subtree, computed
        // at display time from the catalog clone (best-effort; `—` when the
        // clone has no history for it / isn't a git repo).
        Cell::new("Last upstream change"),
    ]);

    for r in rows {
        let version = r.version.clone().unwrap_or_else(|| "—".to_owned());
        let status_cell = render_status(r.status);
        // Phase 5 / US5.b: emit `(N skills, M commands)` when both
        // kinds are present; `(N skills)` for skill-only plugins;
        // `(M commands)` for command-only plugins; `—` when neither.
        // The unindexable plugin case keeps `—`.
        let entries = match r.status {
            PluginStatus::Unindexable => "—".to_owned(),
            _ => format_entries_cell(&r.per_kind),
        };
        let last_indexed = r
            .last_indexed_at
            .as_deref()
            .map(human_relative)
            .unwrap_or_else(|| "—".to_owned());
        // #309: render the upstream change relative too, falling back to `—`.
        // The stored value is an `OffsetDateTime`; format it back to RFC3339
        // so it flows through the shared `human_relative` bucketer.
        let last_upstream = r
            .record
            .last_upstream_change
            .and_then(|dt| {
                dt.format(&time::format_description::well_known::Rfc3339)
                    .ok()
            })
            .map(|s| human_relative(&s))
            .unwrap_or_else(|| "—".to_owned());

        table.add_row(vec![
            Cell::new(&r.id.catalog),
            Cell::new(&r.id.plugin),
            Cell::new(version),
            Cell::new(status_cell),
            Cell::new(entries),
            Cell::new(last_indexed),
            Cell::new(last_upstream),
        ]);
    }
    writeln!(out, "{table}")?;
    Ok(())
}

/// Format the entries cell per
/// `contracts/catalog-and-plugin-extensions-p5.md` § `tome plugin list`,
/// widened in Phase 6 to surface agent-kind entries. Each present kind
/// contributes a `<n> <kind>s` fragment, joined by `, ` and wrapped in
/// parentheses; `—` when nothing is enrolled. Building the list of present
/// fragments avoids a combinatorial match over the three counts.
fn format_entries_cell(per_kind: &PerKindCounts) -> String {
    let mut parts: Vec<String> = Vec::with_capacity(3);
    if per_kind.skills > 0 {
        parts.push(format!("{} skills", per_kind.skills));
    }
    if per_kind.commands > 0 {
        parts.push(format!("{} commands", per_kind.commands));
    }
    if per_kind.agents > 0 {
        parts.push(format!("{} agents", per_kind.agents));
    }
    if parts.is_empty() {
        "—".to_owned()
    } else {
        format!("({})", parts.join(", "))
    }
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

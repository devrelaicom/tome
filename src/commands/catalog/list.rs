//! `tome catalog list`. See `contracts/catalog-list.md`.
//!
//! `plugin_count` comes from the cached manifest, not by re-running git. If
//! the cache is stale the count is the count at last sync — documented in
//! the contract.

use std::io::Write;

use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::catalog::manifest::CatalogManifest;
use crate::catalog::store;
use crate::cli::CatalogListArgs;
use crate::config::CatalogEntry;
use crate::error::TomeError;
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

pub fn run(_args: CatalogListArgs, _scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    // F2a: single global config; F11 reintroduces workspace-aware view.
    let config = store::load(&paths.global_config_file)?;

    if config.catalogs.is_empty() {
        if mode == Mode::Human {
            let mut out = std::io::stdout().lock();
            writeln!(
                out,
                "No catalogs registered. Use `tome catalog add <source>` to add one."
            )?;
        }
        return Ok(());
    }

    match mode {
        Mode::Human => emit_human(&config.catalogs),
        Mode::Json => emit_json(&config.catalogs),
    }
}

fn emit_human(
    catalogs: &std::collections::BTreeMap<String, CatalogEntry>,
) -> Result<(), TomeError> {
    let rows: Vec<Row> = catalogs.values().map(Row::from_entry).collect();
    let name_w = column_width(&rows, |r| r.name.len(), "NAME".len());
    let url_w = column_width(&rows, |r| r.url.len(), "URL".len()).min(60);
    let ref_w = column_width(&rows, |r| r.ref_.len(), "REF".len());
    let plugins_w = column_width(&rows, |r| r.plugins.to_string().len(), "PLUGINS".len());

    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "{:name_w$}  {:url_w$}  {:ref_w$}  {:plugins_w$}  LAST SYNCED",
        "NAME",
        "URL",
        "REF",
        "PLUGINS",
        name_w = name_w,
        url_w = url_w,
        ref_w = ref_w,
        plugins_w = plugins_w,
    )?;
    for r in &rows {
        let truncated_url = truncate_middle(&r.url, url_w);
        writeln!(
            out,
            "{:name_w$}  {:url_w$}  {:ref_w$}  {:plugins_w$}  {}",
            r.name,
            truncated_url,
            r.ref_,
            r.plugins,
            r.last_synced,
            name_w = name_w,
            url_w = url_w,
            ref_w = ref_w,
            plugins_w = plugins_w,
        )?;
    }
    Ok(())
}

fn emit_json(catalogs: &std::collections::BTreeMap<String, CatalogEntry>) -> Result<(), TomeError> {
    for entry in catalogs.values() {
        let plugin_count = read_plugin_count(entry).unwrap_or(0);
        let record = JsonRow {
            name: &entry.name,
            url: &entry.url,
            ref_: &entry.ref_,
            plugin_count,
            last_synced: entry.last_synced,
        };
        crate::output::write_json(&record)?;
    }
    Ok(())
}

fn read_plugin_count(entry: &CatalogEntry) -> Option<usize> {
    let manifest_path = entry.path.join("tome-catalog.toml");
    let bytes = std::fs::read(&manifest_path).ok()?;
    let m = CatalogManifest::parse_and_validate(&manifest_path, &entry.path, &bytes).ok()?;
    Some(m.plugins.len())
}

struct Row {
    name: String,
    url: String,
    ref_: String,
    plugins: usize,
    last_synced: String,
}

impl Row {
    fn from_entry(entry: &CatalogEntry) -> Self {
        Self {
            name: entry.name.clone(),
            url: entry.url.clone(),
            ref_: entry.ref_.clone(),
            plugins: read_plugin_count(entry).unwrap_or(0),
            last_synced: entry
                .last_synced
                .format(&Rfc3339)
                .unwrap_or_else(|_| "—".into()),
        }
    }
}

fn column_width<F: Fn(&Row) -> usize>(rows: &[Row], f: F, header_min: usize) -> usize {
    rows.iter().map(f).max().unwrap_or(0).max(header_min)
}

fn truncate_middle(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let keep_each_side = max.saturating_sub(1) / 2;
    let head: String = s.chars().take(keep_each_side).collect();
    let tail_count = max.saturating_sub(keep_each_side + 1);
    let tail: String = s
        .chars()
        .rev()
        .take(tail_count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{}…{}", head, tail)
}

#[derive(Serialize)]
struct JsonRow<'a> {
    name: &'a str,
    url: &'a str,
    #[serde(rename = "ref")]
    ref_: &'a str,
    plugin_count: usize,
    #[serde(with = "time::serde::rfc3339")]
    last_synced: OffsetDateTime,
}

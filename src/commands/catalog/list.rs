//! `tome catalog list`. See `contracts/catalog-list.md` and FR-364.
//!
//! Phase 4 / F11b: reports only the resolved workspace's enrolments from
//! `workspace_catalogs`. `plugin_count` is read from the cached manifest;
//! `last_synced` is the clone directory's mtime. Both may be `None` when
//! the on-disk clone is absent or unreadable — rendered as `—` in the
//! human table, `null` in JSON.

use std::io::Write;

use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::catalog::manifest::CatalogManifest;
use crate::cli::CatalogListArgs;
use crate::commands::plugin::registry_seeds;
use crate::error::TomeError;
use crate::index::workspace_catalogs::CatalogEnrolment;
use crate::index::{self, OpenOptions, workspace_catalogs};
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

pub fn run(_args: CatalogListArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let workspace_name = scope.scope.name().as_str();

    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
            summariser: summariser_seed,
        },
    )?;

    let enrolments = workspace_catalogs::list_for_workspace(&conn, workspace_name)?;

    if enrolments.is_empty() {
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
        Mode::Human => emit_human(&enrolments, &paths),
        Mode::Json => emit_json(&enrolments, &paths),
    }
}

fn emit_human(enrolments: &[CatalogEnrolment], paths: &Paths) -> Result<(), TomeError> {
    let rows: Vec<Row> = enrolments
        .iter()
        .map(|e| Row::from_enrolment(e, paths))
        .collect();
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

fn emit_json(enrolments: &[CatalogEnrolment], paths: &Paths) -> Result<(), TomeError> {
    for e in enrolments {
        let cache_dir = paths.cache_dir_for(&e.url);
        let plugin_count = read_plugin_count(&cache_dir).unwrap_or(0);
        let last_synced = clone_mtime(&cache_dir);
        let record = JsonRow {
            name: &e.catalog_name,
            url: &e.url,
            ref_: &e.pinned_ref,
            plugin_count,
            last_synced,
        };
        crate::output::write_json(&record)?;
    }
    Ok(())
}

fn read_plugin_count(cache_dir: &std::path::Path) -> Option<usize> {
    let manifest_path = cache_dir.join("tome-catalog.toml");
    // Third-party manifest, best-effort count: cap at PLUGIN_MANIFEST_MAX
    // (FR-006). An over-cap file is `Err` → `.ok()?` → None, the same
    // fallback an unreadable/unparsable manifest takes here.
    let bytes = crate::util::bounded_read(&manifest_path, crate::util::PLUGIN_MANIFEST_MAX).ok()?;
    let m = CatalogManifest::parse_and_validate(&manifest_path, cache_dir, &bytes).ok()?;
    Some(m.plugins.len())
}

fn clone_mtime(cache_dir: &std::path::Path) -> Option<OffsetDateTime> {
    let meta = std::fs::metadata(cache_dir).ok()?;
    let systime = meta.modified().ok()?;
    Some(OffsetDateTime::from(systime))
}

struct Row {
    name: String,
    url: String,
    ref_: String,
    plugins: usize,
    last_synced: String,
}

impl Row {
    fn from_enrolment(e: &CatalogEnrolment, paths: &Paths) -> Self {
        let cache_dir = paths.cache_dir_for(&e.url);
        let last_synced = clone_mtime(&cache_dir)
            .and_then(|ts| ts.format(&Rfc3339).ok())
            .unwrap_or_else(|| "—".into());
        Self {
            name: e.catalog_name.clone(),
            url: e.url.clone(),
            ref_: e.pinned_ref.clone(),
            plugins: read_plugin_count(&cache_dir).unwrap_or(0),
            last_synced,
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
    #[serde(with = "time::serde::rfc3339::option")]
    last_synced: Option<OffsetDateTime>,
}

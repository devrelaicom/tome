//! `tome catalog show`. See `contracts/catalog-show.md` and FR-364.
//!
//! Phase 4 / F11b: enrolment lives in `workspace_catalogs`. The cache
//! dir is derived from URL; `last_synced` from the clone directory's
//! mtime.

use std::io::Write;

use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::catalog::manifest::CatalogManifest;
use crate::cli::CatalogShowArgs;
use crate::commands::plugin::registry_seeds;
use crate::error::TomeError;
use crate::index::{self, OpenOptions, workspace_catalogs};
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

pub fn run(args: CatalogShowArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
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

    let enrolment = workspace_catalogs::find(&conn, workspace_name, &args.name)?
        .ok_or_else(|| TomeError::CatalogNotFound(args.name.clone()))?;

    let cache_dir = paths.cache_dir_for(&enrolment.url);
    let manifest_path = cache_dir.join("tome-catalog.toml");
    // Third-party manifest: cap the read at PLUGIN_MANIFEST_MAX (FR-006,
    // F-PLUGIN-MANIFEST-DOS). `bounded_read` returns `TomeError::Io` on both
    // genuine I/O failures and an over-cap file, preserving this site's
    // existing exit-7 contract without an unbounded read.
    let manifest_bytes =
        crate::util::bounded_read(&manifest_path, crate::util::PLUGIN_MANIFEST_MAX)?;
    let manifest = CatalogManifest::parse_and_validate(&manifest_path, &cache_dir, &manifest_bytes)
        .map_err(TomeError::ManifestInvalid)?;

    let last_synced = std::fs::metadata(&cache_dir)
        .ok()
        .and_then(|m| m.modified().ok())
        .map(OffsetDateTime::from);

    match mode {
        Mode::Human => emit_human(
            &manifest,
            last_synced,
            &enrolment.url,
            &enrolment.pinned_ref,
        ),
        Mode::Json => emit_json(
            &manifest,
            last_synced,
            &enrolment.url,
            &enrolment.pinned_ref,
        ),
    }
}

fn emit_human(
    m: &CatalogManifest,
    last_synced: Option<OffsetDateTime>,
    url: &str,
    ref_: &str,
) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{} (v{})", m.name, m.version)?;
    writeln!(out, "  {}", m.description)?;
    writeln!(out, "  Owner: {} <{}>", m.owner.name, m.owner.email)?;
    writeln!(out, "  Source: {} (ref: {})", url, ref_)?;
    let synced = last_synced
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_else(|| "—".into());
    writeln!(out, "  Last synced: {}", synced)?;
    if m.plugins.is_empty() {
        writeln!(out, "\nNo plugins declared.")?;
    } else {
        writeln!(out, "\nPlugins:")?;
        let name_w = m.plugins.iter().map(|p| p.name.len()).max().unwrap_or(0);
        for p in &m.plugins {
            writeln!(out, "  {:<name_w$}  {}", p.name, p.source, name_w = name_w)?;
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct ShowEnvelope<'a> {
    name: &'a str,
    description: &'a str,
    version: &'a str,
    owner: OwnerOut<'a>,
    registered: Registered<'a>,
    plugins: Vec<PluginOut<'a>>,
}

#[derive(Serialize)]
struct OwnerOut<'a> {
    name: &'a str,
    email: &'a str,
}

#[derive(Serialize)]
struct Registered<'a> {
    url: &'a str,
    #[serde(rename = "ref")]
    ref_: &'a str,
    #[serde(with = "time::serde::rfc3339::option")]
    last_synced: Option<OffsetDateTime>,
}

#[derive(Serialize)]
struct PluginOut<'a> {
    name: &'a str,
    source: &'a str,
}

fn emit_json(
    m: &CatalogManifest,
    last_synced: Option<OffsetDateTime>,
    url: &str,
    ref_: &str,
) -> Result<(), TomeError> {
    let env = ShowEnvelope {
        name: &m.name,
        description: &m.description,
        version: &m.version,
        owner: OwnerOut {
            name: &m.owner.name,
            email: &m.owner.email,
        },
        registered: Registered {
            url,
            ref_,
            last_synced,
        },
        plugins: m
            .plugins
            .iter()
            .map(|p| PluginOut {
                name: &p.name,
                source: &p.source,
            })
            .collect(),
    };
    crate::output::write_json(&env)
}

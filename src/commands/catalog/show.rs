//! `tome catalog show`. See `contracts/catalog-show.md`.

use std::io::Write;

use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::catalog::manifest::CatalogManifest;
use crate::catalog::store;
use crate::cli::CatalogShowArgs;
use crate::error::TomeError;
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

pub fn run(args: CatalogShowArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let config = store::load(&paths.config_file_for(&scope.scope))?;
    let entry = config
        .catalogs
        .get(&args.name)
        .ok_or_else(|| TomeError::CatalogNotFound(args.name.clone()))?;

    let manifest_path = entry.path.join("tome-catalog.toml");
    let manifest_bytes = std::fs::read(&manifest_path).map_err(TomeError::Io)?;
    let manifest =
        CatalogManifest::parse_and_validate(&manifest_path, &entry.path, &manifest_bytes)
            .map_err(TomeError::ManifestInvalid)?;

    match mode {
        Mode::Human => emit_human(&manifest, entry.last_synced, &entry.url, &entry.ref_),
        Mode::Json => emit_json(&manifest, entry.last_synced, &entry.url, &entry.ref_),
    }
}

fn emit_human(
    m: &CatalogManifest,
    last_synced: OffsetDateTime,
    url: &str,
    ref_: &str,
) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "{} (v{})", m.name, m.version)?;
    writeln!(out, "  {}", m.description)?;
    writeln!(out, "  Owner: {} <{}>", m.owner.name, m.owner.email)?;
    writeln!(out, "  Source: {} (ref: {})", url, ref_)?;
    writeln!(
        out,
        "  Last synced: {}",
        last_synced.format(&Rfc3339).unwrap_or_default()
    )?;
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
    #[serde(with = "time::serde::rfc3339")]
    last_synced: OffsetDateTime,
}

#[derive(Serialize)]
struct PluginOut<'a> {
    name: &'a str,
    source: &'a str,
}

fn emit_json(
    m: &CatalogManifest,
    last_synced: OffsetDateTime,
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

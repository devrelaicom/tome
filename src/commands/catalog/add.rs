//! `tome catalog add` — clone, parse, register. See
//! `specs/001-phase-1-foundations/contracts/catalog-add.md`.

use std::io::Write;
use std::path::Path;

use serde::Serialize;
use time::OffsetDateTime;

use crate::catalog::git::{Git, scrub_to_string};
use crate::catalog::manifest::CatalogManifest;
use crate::catalog::store;
use crate::cli::CatalogAddArgs;
use crate::config::CatalogEntry;
use crate::error::TomeError;
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

use super::source;

pub fn run(args: CatalogAddArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    // F2a: Phase 4 collapses every scope onto a single global
    // `config.toml`. F11 reintroduces per-workspace catalog enrolment via
    // the `workspace_catalogs` junction table; until then the
    // resolved-scope parameter is informational only.
    let _ = scope;
    let config_file = paths.global_config_file.clone();
    let url = source::resolve(&args.source)?;
    let cache_dir = paths.cache_dir_for(&url);

    let mut config = store::load(&config_file)?;
    let clone_ref = args.ref_.as_deref();

    // Phase 3: reference-counted clones (`catalog-extensions-p3.md`).
    // If the on-disk cache already exists, some other scope (or this
    // scope on a previous failed-write) brought it down. Reuse it
    // instead of erroring — same URL across scopes shares one clone.
    // The display-name collision check below still rejects the
    // duplicate-within-this-scope case.
    let reuse_existing = cache_dir.exists();

    let (manifest, _tempdir_guard) = if reuse_existing {
        let manifest_path = cache_dir.join("tome-catalog.toml");
        let manifest_bytes = std::fs::read(&manifest_path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                TomeError::ManifestInvalid(crate::error::ManifestInvalid::TomlParse {
                    file: manifest_path.clone(),
                    message: "cached catalog has no tome-catalog.toml".to_string(),
                })
            }
            _ => TomeError::Io(e),
        })?;
        let manifest =
            CatalogManifest::parse_and_validate(&manifest_path, &cache_dir, &manifest_bytes)
                .map_err(TomeError::ManifestInvalid)?;
        (manifest, None)
    } else {
        // Clone into a sibling tempdir of the final cache directory so
        // the atomic rename never crosses filesystem boundaries
        // (FR-017a). The `tempfile::TempDir` is dropped on every error
        // path via RAII.
        std::fs::create_dir_all(&paths.catalogs_dir).map_err(TomeError::Io)?;
        let tempdir = tempfile::Builder::new()
            .prefix(".tome-incoming-")
            .tempdir_in(&paths.catalogs_dir)
            .map_err(TomeError::Io)?;
        let clone_dest = tempdir.path().join("repo");

        // Clone label — scrubbed because a `git` failure embeds this in
        // `TomeError::GitFailed.catalog` (FR-024/025).
        let display_source = scrub_to_string(args.source.as_bytes());
        let git = Git::new(&display_source);
        git.clone_shallow(&url, &clone_dest, clone_ref)?;

        let manifest_path = clone_dest.join("tome-catalog.toml");
        let manifest_bytes = std::fs::read(&manifest_path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
                TomeError::ManifestInvalid(crate::error::ManifestInvalid::TomlParse {
                    file: manifest_path.clone(),
                    message: "no tome-catalog.toml at the catalog root".to_string(),
                })
            }
            _ => TomeError::Io(e),
        })?;
        let manifest =
            CatalogManifest::parse_and_validate(&manifest_path, &clone_dest, &manifest_bytes)
                .map_err(TomeError::ManifestInvalid)?;

        // Persist the tempdir as the final cache directory.
        persist_clone(&clone_dest, &cache_dir)?;
        (manifest, Some(tempdir))
    };

    let display_name = args.name.clone().unwrap_or_else(|| manifest.name.clone());
    if config.catalogs.contains_key(&display_name) {
        return Err(TomeError::CatalogAlreadyExists(display_name));
    }

    // Scrub credentials before persisting to `config.toml`: a user-supplied
    // URL of the form `https://user:token@host/repo` must not leave its
    // userinfo on disk (the resolved `url` is the same string the user
    // typed, modulo `source::resolve` normalisation; nothing else along
    // this path strips them).
    let scrubbed_url = scrub_to_string(url.as_bytes());
    let entry = CatalogEntry {
        name: display_name.clone(),
        url: scrubbed_url,
        ref_: clone_ref.unwrap_or("main").to_string(),
        path: cache_dir.clone(),
        last_synced: OffsetDateTime::now_utc(),
    };
    config.catalogs.insert(display_name.clone(), entry.clone());
    if let Err(e) = store::save(&config_file, &config) {
        // Roll back the cache directory ONLY if we actually cloned it
        // (we own it). If we reused an existing clone, another scope's
        // config still references it — don't delete what we didn't
        // create.
        if !reuse_existing {
            let _ = std::fs::remove_dir_all(&cache_dir);
        }
        return Err(e);
    }

    emit(mode, &entry, manifest.plugins.len())?;
    Ok(())
}

fn persist_clone(staged: &Path, final_dir: &Path) -> Result<(), TomeError> {
    if let Some(parent) = final_dir.parent() {
        std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    }
    std::fs::rename(staged, final_dir).map_err(TomeError::Io)?;
    Ok(())
}

#[derive(Serialize)]
struct AddedEnvelope<'a> {
    added: AddedRecord<'a>,
}

#[derive(Serialize)]
struct AddedRecord<'a> {
    name: &'a str,
    url: &'a str,
    #[serde(rename = "ref")]
    ref_: &'a str,
    plugin_count: usize,
    #[serde(with = "time::serde::rfc3339")]
    last_synced: OffsetDateTime,
}

fn emit(mode: Mode, entry: &CatalogEntry, plugin_count: usize) -> Result<(), TomeError> {
    match mode {
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            writeln!(
                out,
                "Added catalog `{}` from {} (ref: {}, plugins: {}).",
                entry.name, entry.url, entry.ref_, plugin_count
            )?;
        }
        Mode::Json => {
            let env = AddedEnvelope {
                added: AddedRecord {
                    name: &entry.name,
                    url: &entry.url,
                    ref_: &entry.ref_,
                    plugin_count,
                    last_synced: entry.last_synced,
                },
            };
            crate::output::write_json(&env)?;
        }
    }
    Ok(())
}

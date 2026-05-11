//! `tome catalog update`. See `contracts/catalog-update.md`.

use std::io::Write;

use serde::Serialize;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::catalog::git::{self, Git};
use crate::catalog::manifest::CatalogManifest;
use crate::catalog::store;
use crate::cli::CatalogUpdateArgs;
use crate::config::{CatalogEntry, Config};
use crate::error::TomeError;
use crate::output::Mode;
use crate::paths::Paths;

pub fn run(args: CatalogUpdateArgs, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let mut config = store::load(&paths.config_file)?;

    match args.name {
        Some(name) => {
            if !config.catalogs.contains_key(&name) {
                return Err(TomeError::CatalogNotFound(name));
            }
            refresh_one(&paths.config_file, &mut config, &name, mode)
        }
        None => {
            // Fail-fast on first error (FR-007). We iterate over a cloned key
            // list so we can mutate `config.catalogs` inside the loop.
            let names: Vec<String> = config.catalogs.keys().cloned().collect();
            for name in names {
                refresh_one(&paths.config_file, &mut config, &name, mode)?;
            }
            Ok(())
        }
    }
}

fn refresh_one(
    config_file: &std::path::Path,
    config: &mut Config,
    name: &str,
    mode: Mode,
) -> Result<(), TomeError> {
    let entry = config.catalogs.get(name).expect("caller checked");
    let entry = entry.clone();

    if git::looks_like_sha(&entry.ref_) {
        emit_pinned(mode, &entry.name, &entry.ref_)?;
        return Ok(());
    }

    let git = Git::new(&entry.name);
    let head_before = git.rev_parse_head(&entry.path).ok();
    git.fetch(&entry.path)?;

    // Resolve the target ref. Branches go through `origin/<ref>`; tags go
    // through `refs/tags/<ref>`. We don't know which up front, so try the
    // branch form first; if it fails, fall back to the tag form. Either
    // success advances HEAD; either failure surfaces via GitFailed.
    let branch_target = format!("origin/{}", entry.ref_);
    let result = git.reset_hard(&entry.path, &branch_target);
    if result.is_err() {
        let tag_target = format!("refs/tags/{}", entry.ref_);
        git.reset_hard(&entry.path, &tag_target)?;
    }

    let head_after = git.rev_parse_head(&entry.path).ok();
    let advanced = match (head_before, head_after) {
        (Some(a), Some(b)) if a != b => {
            Advance::Commits(count_commits_between(&entry.path, &a, &b))
        }
        (Some(a), Some(b)) if a == b => Advance::UpToDate,
        _ => Advance::Unknown,
    };

    let manifest_path = entry.path.join("tome-catalog.toml");
    let manifest_bytes = std::fs::read(&manifest_path).map_err(TomeError::Io)?;
    let manifest =
        CatalogManifest::parse_and_validate(&manifest_path, &entry.path, &manifest_bytes)
            .map_err(TomeError::ManifestInvalid)?;

    let now = OffsetDateTime::now_utc();
    let updated_entry = CatalogEntry {
        last_synced: now,
        ..entry.clone()
    };
    config
        .catalogs
        .insert(name.to_string(), updated_entry.clone());
    store::save(config_file, config)?;

    emit_refreshed(mode, &updated_entry, manifest.plugins.len(), advanced)?;
    Ok(())
}

enum Advance {
    Commits(usize),
    UpToDate,
    Unknown,
}

fn count_commits_between(repo: &std::path::Path, from: &str, to: &str) -> usize {
    // `git rev-list --count from..to` would be ideal, but we already have the
    // string SHAs. Re-shelling for one number is fine. If the count call
    // fails we report 0; the success of `update` does not depend on this.
    use std::process::Command;
    let output = Command::new("git")
        .args(["rev-list", "--count", &format!("{}..{}", from, to)])
        .current_dir(repo)
        .output();
    let Ok(out) = output else {
        return 0;
    };
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<usize>()
        .unwrap_or(0)
}

#[derive(Serialize)]
struct RefreshedEnvelope<'a> {
    refreshed: RefreshedRecord<'a>,
}

#[derive(Serialize)]
struct RefreshedRecord<'a> {
    name: &'a str,
    #[serde(rename = "ref")]
    ref_: &'a str,
    plugin_count: usize,
    advanced_commits: Option<usize>,
    #[serde(with = "time::serde::rfc3339")]
    last_synced: OffsetDateTime,
}

#[derive(Serialize)]
struct PinnedEnvelope<'a> {
    pinned: PinnedRecord<'a>,
}

#[derive(Serialize)]
struct PinnedRecord<'a> {
    name: &'a str,
    #[serde(rename = "ref")]
    ref_: &'a str,
}

fn emit_refreshed(
    mode: Mode,
    entry: &CatalogEntry,
    plugin_count: usize,
    advance: Advance,
) -> Result<(), TomeError> {
    match mode {
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            let tail = match advance {
                Advance::Commits(n) => {
                    format!("advanced {} commit{}", n, if n == 1 { "" } else { "s" })
                }
                Advance::UpToDate => "already up-to-date".to_string(),
                Advance::Unknown => "refreshed".to_string(),
            };
            writeln!(
                out,
                "Refreshed `{}` (ref: {}, plugins: {}, {}).",
                entry.name, entry.ref_, plugin_count, tail
            )?;
        }
        Mode::Json => {
            let advanced_commits = match advance {
                Advance::Commits(n) => Some(n),
                Advance::UpToDate => Some(0),
                Advance::Unknown => None,
            };
            let env = RefreshedEnvelope {
                refreshed: RefreshedRecord {
                    name: &entry.name,
                    ref_: &entry.ref_,
                    plugin_count,
                    advanced_commits,
                    last_synced: entry.last_synced,
                },
            };
            crate::output::write_json(&env)?;
        }
    }
    Ok(())
}

fn emit_pinned(mode: Mode, name: &str, ref_: &str) -> Result<(), TomeError> {
    match mode {
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            writeln!(
                out,
                "Catalog `{}` is pinned to {}; use `tome catalog add --ref` to change.",
                name, ref_
            )?;
        }
        Mode::Json => {
            let env = PinnedEnvelope {
                pinned: PinnedRecord { name, ref_ },
            };
            crate::output::write_json(&env)?;
        }
    }
    let _ = Rfc3339; // silence unused-import in this fn
    Ok(())
}

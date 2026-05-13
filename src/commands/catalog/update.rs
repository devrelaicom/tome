//! `tome catalog update`. See `contracts/catalog-update.md` and
//! `contracts/catalog-extensions.md` §"tome catalog update".

use std::io::Write;

use serde::Serialize;
use time::OffsetDateTime;

use crate::catalog::git::{self, Git};
use crate::catalog::manifest::CatalogManifest;
use crate::catalog::store;
use crate::cli::CatalogUpdateArgs;
use crate::config::{CatalogEntry, Config};
use crate::embedding::fastembed::FastembedEmbedder;
use crate::error::TomeError;
use crate::index::skills::ReindexSummary;
use crate::index::{self, OpenOptions, enabled_plugins_for_catalog};
use crate::output::Mode;
use crate::paths::Paths;
use crate::plugin::PluginId;
use crate::plugin::lifecycle::{self, LifecycleDeps};
use crate::presentation::colour;

use crate::commands::plugin::{embedder_entry, registry_seeds};

pub fn run(args: CatalogUpdateArgs, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let mut config = store::load(&paths.config_file)?;

    let names: Vec<String> = match args.name {
        Some(name) => {
            if !config.catalogs.contains_key(&name) {
                return Err(TomeError::CatalogNotFound(name));
            }
            vec![name]
        }
        None => config.catalogs.keys().cloned().collect(),
    };

    // The embedder is loaded lazily — only after the first catalog with at
    // least one enabled plugin needs it. `tome catalog update` against a
    // tome install with zero enabled plugins must never touch model files.
    let mut embedder: Option<FastembedEmbedder> = None;

    for name in names {
        let refreshed = refresh_one(&paths.config_file, &mut config, &name, mode)?;
        if !refreshed {
            // SHA-pinned catalogs (no git op happened). Skip the reindex
            // pass; pinned catalogs are intentionally frozen.
            continue;
        }

        // Skip the reindex pass entirely when no enabled plugin exists in
        // this catalog. The index open is cheap (idempotent bootstrap) but
        // a sync that involves no enabled plugins must not load the
        // embedder.
        let enabled = read_enabled_plugins(&paths, &name)?;
        if enabled.is_empty() {
            continue;
        }

        let embedder_ref =
            embedder.get_or_insert_with_result::<TomeError, _>(|| load_embedder(&paths))?;

        let (embedder_seed, reranker_seed) = registry_seeds();
        let deps = LifecycleDeps {
            paths: &paths,
            config: &config,
            embedder: embedder_ref,
            embedder_seed,
            reranker_seed,
            allow_model_download: false,
        };

        let outcome = reindex_catalog_plugins(&name, &enabled, &deps)?;
        emit_reindex_outcome(mode, &name, &outcome)?;
    }

    Ok(())
}

/// Helper trait so the lazy-embedder pattern reads cleanly. `Option::get_or_insert_with`
/// returns `&mut T`, not `Result<&mut T, E>`; this mirrors that for the fallible case.
trait GetOrInsertWithResult<T> {
    fn get_or_insert_with_result<E, F>(&mut self, f: F) -> Result<&mut T, E>
    where
        F: FnOnce() -> Result<T, E>;
}

impl<T> GetOrInsertWithResult<T> for Option<T> {
    fn get_or_insert_with_result<E, F>(&mut self, f: F) -> Result<&mut T, E>
    where
        F: FnOnce() -> Result<T, E>,
    {
        if self.is_none() {
            *self = Some(f()?);
        }
        Ok(self.as_mut().expect("just inserted"))
    }
}

fn load_embedder(paths: &Paths) -> Result<FastembedEmbedder, TomeError> {
    let entry = embedder_entry();
    let dir = paths.model_path(entry.name)?;
    FastembedEmbedder::load(entry, &dir)
}

fn read_enabled_plugins(paths: &Paths, catalog: &str) -> Result<Vec<String>, TomeError> {
    let (embedder_seed, reranker_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
        },
    )?;
    enabled_plugins_for_catalog(&conn, catalog)
}

/// Per-plugin row in the catalog-update summary. One of `summary` or
/// `auto_disabled` is set; the other is None.
#[derive(Debug, Clone)]
pub struct PluginChange {
    pub plugin: PluginId,
    pub summary: Option<ReindexSummary>,
    /// When the plugin was auto-disabled per FR-033, the reason text the
    /// CLI surfaces on stderr.
    pub auto_disabled: Option<String>,
    pub warnings: Vec<String>,
}

/// Aggregated outcome for one catalog's reindex pass.
#[derive(Debug, Clone, Default)]
pub struct CatalogReindexOutcome {
    pub plugins: Vec<PluginChange>,
}

/// Walk every enabled plugin in one catalog, reindexing it against the
/// freshly-refreshed on-disk source. Returns an aggregated outcome that
/// callers (`run` above, the integration test) inspect or print.
///
/// Per-plugin failure modes:
///
/// * `PluginNotFound` (the plugin dir vanished upstream) or
///   `PluginManifestParseError` (the plugin.json is gone or malformed) →
///   the plugin is auto-disabled per FR-033 inside the function. The
///   per-plugin `PluginChange.auto_disabled` field carries the reason.
/// * Any other error propagates immediately — `IndexBusy`, `ModelMissing`,
///   `EmbeddingGenerationFailure`, etc. are infrastructure failures that
///   would otherwise corrupt the per-plugin accounting if we tried to
///   continue.
pub fn reindex_catalog_plugins(
    catalog: &str,
    enabled_plugins: &[String],
    deps: &LifecycleDeps<'_>,
) -> Result<CatalogReindexOutcome, TomeError> {
    let mut outcome = CatalogReindexOutcome::default();
    for plugin_name in enabled_plugins {
        let id = PluginId {
            catalog: catalog.to_owned(),
            plugin: plugin_name.clone(),
        };
        match lifecycle::reindex_plugin(&id, deps, false) {
            Ok(reindex) => {
                outcome.plugins.push(PluginChange {
                    plugin: id,
                    summary: Some(reindex.summary),
                    auto_disabled: None,
                    warnings: reindex.warnings,
                });
            }
            Err(TomeError::PluginNotFound(_)) => {
                let reason = "plugin directory missing upstream";
                lifecycle::auto_disable_orphan(&id, deps)?;
                outcome.plugins.push(PluginChange {
                    plugin: id,
                    summary: None,
                    auto_disabled: Some(reason.to_owned()),
                    warnings: Vec::new(),
                });
            }
            Err(TomeError::PluginManifestParseError { message, .. }) => {
                let reason = format!("plugin.json malformed: {message}");
                lifecycle::auto_disable_orphan(&id, deps)?;
                outcome.plugins.push(PluginChange {
                    plugin: id,
                    summary: None,
                    auto_disabled: Some(reason),
                    warnings: Vec::new(),
                });
            }
            Err(e) => return Err(e),
        }
    }
    Ok(outcome)
}

/// Refresh one catalog's git working tree. Returns `Ok(true)` when a
/// refresh actually happened (and the caller should run the reindex pass);
/// `Ok(false)` when the catalog is SHA-pinned and was not touched.
fn refresh_one(
    config_file: &std::path::Path,
    config: &mut Config,
    name: &str,
    mode: Mode,
) -> Result<bool, TomeError> {
    let entry = config.catalogs.get(name).expect("caller checked");
    let entry = entry.clone();

    if git::looks_like_sha(&entry.ref_) {
        emit_pinned(mode, &entry.name, &entry.ref_)?;
        return Ok(false);
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
    Ok(true)
}

fn emit_reindex_outcome(
    mode: Mode,
    catalog: &str,
    outcome: &CatalogReindexOutcome,
) -> Result<(), TomeError> {
    if outcome.plugins.is_empty() {
        return Ok(());
    }
    match mode {
        Mode::Human => emit_reindex_human(catalog, outcome),
        Mode::Json => emit_reindex_json(catalog, outcome),
    }
}

fn emit_reindex_human(catalog: &str, outcome: &CatalogReindexOutcome) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "Reindexed plugins in `{}`:", catalog)?;
    for change in &outcome.plugins {
        match (&change.summary, &change.auto_disabled) {
            (Some(s), None) => writeln!(
                out,
                "  {} {}: added {} · modified {} · removed {} · unchanged {}",
                colour::success("✓"),
                change.plugin,
                s.added,
                s.modified,
                s.removed,
                s.unchanged,
            )?,
            (None, Some(reason)) => {
                let mut err = std::io::stderr().lock();
                writeln!(
                    err,
                    "{} {} auto-disabled: {}",
                    colour::warning("warning:"),
                    change.plugin,
                    reason,
                )?;
            }
            _ => {}
        }
        for w in &change.warnings {
            let mut err = std::io::stderr().lock();
            writeln!(err, "{} {}", colour::warning("warning:"), w)?;
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct PluginChangeRecord<'a> {
    catalog: &'a str,
    plugin: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    skills_added: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skills_modified: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skills_removed: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skills_unchanged: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_disabled_reason: Option<&'a str>,
}

#[derive(Serialize)]
struct PluginChangeEnvelope<'a> {
    plugin_change: PluginChangeRecord<'a>,
}

fn emit_reindex_json(catalog: &str, outcome: &CatalogReindexOutcome) -> Result<(), TomeError> {
    for change in &outcome.plugins {
        let record = match (&change.summary, &change.auto_disabled) {
            (Some(s), None) => PluginChangeRecord {
                catalog,
                plugin: &change.plugin.plugin,
                skills_added: Some(s.added),
                skills_modified: Some(s.modified),
                skills_removed: Some(s.removed),
                skills_unchanged: Some(s.unchanged),
                auto_disabled_reason: None,
            },
            (None, Some(reason)) => PluginChangeRecord {
                catalog,
                plugin: &change.plugin.plugin,
                skills_added: None,
                skills_modified: None,
                skills_removed: None,
                skills_unchanged: None,
                auto_disabled_reason: Some(reason.as_str()),
            },
            _ => continue,
        };
        crate::output::write_json(&PluginChangeEnvelope {
            plugin_change: record,
        })?;
    }
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
    Ok(())
}

//! `tome catalog update`. See `contracts/catalog-update.md`,
//! `contracts/catalog-extensions.md` §"tome catalog update", and FR-365.
//!
//! Phase 4 / F11b: refreshes every catalog **URL** that any workspace
//! enrols (per `workspace_catalogs.distinct_urls`), not just the
//! resolved workspace's. After each successful per-URL refresh the
//! reindex pass visits every workspace that enrols the refreshed
//! catalog (FR-365).
//!
//! When `args.name` is `Some`, the targeted refresh is scoped to the
//! resolved workspace's enrolment of that catalog (same look-up shape
//! as Phase 1, just sourced from the junction table).

use std::io::Write;
use std::path::Path;

use serde::Serialize;
use time::OffsetDateTime;

use crate::catalog::git::{self, Git};
use crate::catalog::manifest::CatalogManifest;
use crate::cli::CatalogUpdateArgs;
use crate::commands::plugin::{embedder_entry, registry_seeds};
use crate::config::Config;
use crate::embedding::fastembed::FastembedEmbedder;
use crate::error::TomeError;
use crate::index::skills::ReindexSummary;
use crate::index::{self, OpenOptions, enabled_plugins_for_catalog, workspace_catalogs};
use crate::output::Mode;
use crate::paths::Paths;
use crate::plugin::PluginId;
use crate::plugin::lifecycle::{self, LifecycleDeps};
use crate::presentation::colour;
use crate::workspace::{Scope, WorkspaceName};

pub fn run(args: CatalogUpdateArgs, scope: &ResolvedScopeArg, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;

    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed.clone(),
            reranker: reranker_seed.clone(),
            summariser: summariser_seed.clone(),
        },
    )?;

    // Resolve the URL/ref pairs to refresh. With `--name`, only the
    // resolved workspace's enrolment of that catalog. Without, the
    // distinct URLs across every workspace.
    let workspace_name = scope.scope.name().as_str().to_owned();
    let targets: Vec<RefreshTarget> = match args.name {
        Some(ref name) => {
            let enrolment = workspace_catalogs::find(&conn, &workspace_name, name)?
                .ok_or_else(|| TomeError::CatalogNotFound(name.clone()))?;
            vec![RefreshTarget {
                url: enrolment.url,
                pinned_ref: enrolment.pinned_ref,
            }]
        }
        None => workspace_catalogs::distinct_urls(&conn)?
            .into_iter()
            .map(|(url, pinned_ref)| RefreshTarget { url, pinned_ref })
            .collect(),
    };

    // Drop the read-side handle. The per-URL loop reopens under the
    // advisory lock for its mutations.
    drop(conn);

    // Lazy embedder — only paid for once we hit the first catalog with
    // at least one enabled plugin in any workspace.
    let mut embedder: Option<FastembedEmbedder> = None;

    for target in targets {
        let cache_dir = paths.cache_dir_for(&target.url);
        let refreshed = refresh_one(&target, &cache_dir, mode)?;
        if !refreshed {
            continue;
        }

        // Reindex every workspace that enrols this URL. The pass opens
        // its own connection — readers don't need the advisory lock for
        // the workspace+catalog list.
        let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
        let conn = index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: embedder_seed.clone(),
                reranker: reranker_seed.clone(),
                summariser: summariser_seed.clone(),
            },
        )?;
        let affected = workspace_catalogs::workspaces_with_catalog_url(&conn, &target.url)?;
        drop(conn);

        for (ws_name, catalog_name) in affected {
            let enabled = read_enabled_plugins_for(&paths, &ws_name, &catalog_name)?;
            if enabled.is_empty() {
                continue;
            }

            let embedder_ref =
                embedder.get_or_insert_with_result::<TomeError, _>(|| load_embedder(&paths))?;

            // LifecycleDeps still carries a `config: &Config` field for
            // back-compat — F11b makes catalog look-ups via the junction
            // table, but the lifecycle layer hasn't been re-plumbed yet.
            // Pass an empty Config; the lifecycle resolve_plugin_dir will
            // be updated to consult `workspace_catalogs` in a follow-up.
            // Until then, we synthesise a minimal Config with one entry
            // so the catalog look-up resolves.
            let synthetic_config = synthesise_config_for_catalog(&catalog_name, &cache_dir);
            let ws_scope =
                Scope(WorkspaceName::parse(&ws_name).unwrap_or_else(|_| WorkspaceName::global()));
            let (e_seed, r_seed, s_seed) = registry_seeds();
            let deps = LifecycleDeps {
                paths: &paths,
                scope: &ws_scope,
                config: &synthetic_config,
                embedder: embedder_ref,
                embedder_seed: e_seed,
                reranker_seed: r_seed,
                summariser_seed: s_seed,
                allow_model_download: false,
            };

            let outcome = reindex_catalog_plugins(&catalog_name, &enabled, &deps)?;
            emit_reindex_outcome(mode, &catalog_name, &outcome)?;
        }
    }

    Ok(())
}

/// Hand-rolled alias so tests can pull this through. `&ResolvedScope`
/// from the workspace module.
pub type ResolvedScopeArg = crate::workspace::ResolvedScope;

#[derive(Debug, Clone)]
struct RefreshTarget {
    url: String,
    pinned_ref: String,
}

/// Synthesise a minimal `Config` so `lifecycle::reindex_plugin` can
/// resolve the plugin directory via `resolve_plugin_dir`. F11b drops
/// `config.toml` as the registry; the lifecycle layer hasn't yet been
/// re-plumbed to consult `workspace_catalogs` directly. The synthesised
/// entry mirrors the on-disk cache; consumers only read `.path`.
fn synthesise_config_for_catalog(catalog_name: &str, cache_dir: &Path) -> Config {
    use crate::config::CatalogEntry;
    use std::collections::BTreeMap;
    let mut catalogs = BTreeMap::new();
    #[allow(deprecated)]
    catalogs.insert(
        catalog_name.to_owned(),
        CatalogEntry {
            name: catalog_name.to_owned(),
            url: String::new(),
            ref_: String::new(),
            path: cache_dir.to_path_buf(),
            last_synced: OffsetDateTime::now_utc(),
        },
    );
    #[allow(deprecated)]
    Config { catalogs }
}

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

/// Look up enabled plugins for one `(workspace, catalog)`. Opens its
/// own connection so the lock-state surface is clear.
fn read_enabled_plugins_for(
    paths: &Paths,
    workspace: &str,
    catalog: &str,
) -> Result<Vec<String>, TomeError> {
    let (e_seed, r_seed, s_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: e_seed,
            reranker: r_seed,
            summariser: s_seed,
        },
    )?;
    enabled_plugins_for_catalog(&conn, workspace, catalog)
}

/// Per-plugin row in the catalog-update summary. One of `summary` or
/// `auto_disabled` is set; the other is None.
#[derive(Debug, Clone)]
pub struct PluginChange {
    pub plugin: PluginId,
    pub summary: Option<ReindexSummary>,
    pub auto_disabled: Option<String>,
    pub warnings: Vec<String>,
}

/// Aggregated outcome for one catalog's reindex pass.
#[derive(Debug, Clone, Default)]
pub struct CatalogReindexOutcome {
    pub plugins: Vec<PluginChange>,
}

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

/// Refresh one catalog by URL. Returns `Ok(true)` when a refresh
/// actually happened (caller should run the reindex pass); `Ok(false)`
/// for SHA-pinned catalogs (intentionally frozen).
fn refresh_one(target: &RefreshTarget, cache_dir: &Path, mode: Mode) -> Result<bool, TomeError> {
    let display = display_name_from_cache(cache_dir).unwrap_or_else(|| "<unknown>".to_owned());

    if git::looks_like_sha(&target.pinned_ref) {
        emit_pinned(mode, &display, &target.pinned_ref)?;
        return Ok(false);
    }

    let git = Git::new(&display);
    let head_before = git.rev_parse_head(cache_dir).ok();
    git.fetch(cache_dir)?;

    let branch_target = format!("origin/{}", target.pinned_ref);
    let result = git.reset_hard(cache_dir, &branch_target);
    if result.is_err() {
        let tag_target = format!("refs/tags/{}", target.pinned_ref);
        git.reset_hard(cache_dir, &tag_target)?;
    }

    let head_after = git.rev_parse_head(cache_dir).ok();
    let advanced = match (head_before, head_after) {
        (Some(a), Some(b)) if a != b => Advance::Commits(count_commits_between(cache_dir, &a, &b)),
        (Some(a), Some(b)) if a == b => Advance::UpToDate,
        _ => Advance::Unknown,
    };

    let manifest_path = cache_dir.join("tome-catalog.toml");
    let manifest_bytes = std::fs::read(&manifest_path).map_err(TomeError::Io)?;
    let manifest = CatalogManifest::parse_and_validate(&manifest_path, cache_dir, &manifest_bytes)
        .map_err(TomeError::ManifestInvalid)?;

    emit_refreshed(
        mode,
        &display,
        &target.pinned_ref,
        manifest.plugins.len(),
        advanced,
    )?;
    Ok(true)
}

/// Read the manifest's declared catalog name from disk. Used as the
/// display name in human-mode messages — F11b doesn't carry the user's
/// chosen alias through the refresh loop (the URL is the key now).
fn display_name_from_cache(cache_dir: &Path) -> Option<String> {
    let manifest_path = cache_dir.join("tome-catalog.toml");
    let bytes = std::fs::read(&manifest_path).ok()?;
    let m = CatalogManifest::parse_and_validate(&manifest_path, cache_dir, &bytes).ok()?;
    Some(m.name)
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

fn count_commits_between(repo: &Path, from: &str, to: &str) -> usize {
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
    name: &str,
    ref_: &str,
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
                name, ref_, plugin_count, tail
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
                    name,
                    ref_,
                    plugin_count,
                    advanced_commits,
                    last_synced: OffsetDateTime::now_utc(),
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

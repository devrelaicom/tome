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
use crate::commands::plugin::registry_seeds;
use crate::config::Config;
use crate::error::TomeError;
use crate::index::meta::{self, ModelIdent};
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

    // Phase 12 / US2: load the global config strictly so the embedder resolves
    // remote-vs-bundled and the drift guard compares the right identity.
    let cfg = crate::config::load(&paths)?;

    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed.clone(),
            reranker: reranker_seed.clone(),
            summariser: summariser_seed.clone(),
            profile: None,
        },
    )?;

    // B3: refuse the catalog-update re-embed under embedder drift BEFORE doing
    // any git fetch or model load. `catalog update` re-embeds the affected
    // plugins (a PARTIAL re-embed), so landing new-dimension vectors against an
    // old-dimension index would corrupt it. The configured embedder is the
    // ACTIVE (remote-or-bundled) identity; reindex is the only path allowed to
    // switch it.
    let active_embedder = meta::active_embedder(&conn)?;
    let active_embedder_seed = crate::embedding::embedder_seed(&cfg, active_embedder)?;
    meta::guard_embedder_drift(
        &conn,
        &ModelIdent {
            name: active_embedder_seed.name.clone(),
            version: active_embedder_seed.version.clone(),
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
    // at least one enabled plugin in any workspace. `Box<dyn Embedder>` so the
    // remote-or-bundled choice (Phase 12) is uniform.
    let mut embedder: Option<Box<dyn crate::embedding::Embedder>> = None;

    for target in targets {
        let cache_dir = paths.cache_dir_for(&target.url);
        let refreshed = refresh_one(&target, &cache_dir, mode)?;
        if !refreshed {
            continue;
        }

        // best-effort: the stored target url is `file://` for a local-path
        // source; every remote shape is Git. One emit per catalog actually
        // refreshed (a SHA-pinned target returns `refreshed == false` above
        // and is skipped).
        let source_type = if target.url.starts_with("file://") {
            crate::telemetry::event::SourceType::Local
        } else {
            crate::telemetry::event::SourceType::Git
        };
        crate::telemetry::emit(crate::telemetry::event::CatalogActionEvent {
            action: crate::telemetry::event::CatalogAction::Updated,
            source_type,
        });

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
                profile: None,
            },
        )?;
        let affected = workspace_catalogs::workspaces_with_catalog_url(&conn, &target.url)?;
        drop(conn);

        for (ws_name, catalog_name) in affected {
            let enabled = read_enabled_plugins_for(&paths, &ws_name, &catalog_name)?;
            if enabled.is_empty() {
                continue;
            }

            let embedder_ref = embedder
                .get_or_insert_with_result::<TomeError, _>(|| load_embedder(&cfg, &paths))?;

            // F11b + FF1: `lifecycle::reindex_plugin` resolves the plugin
            // directory from the `workspace_catalogs` enrolment, so the
            // catalog root no longer has to be threaded through a synthetic
            // `Config`. The field is vestigial; pass the default.
            let config = Config::default();
            let ws_scope =
                Scope(WorkspaceName::parse(&ws_name).unwrap_or_else(|_| WorkspaceName::global()));
            let (_e_seed, r_seed, s_seed) = registry_seeds();
            let deps = LifecycleDeps {
                paths: &paths,
                scope: &ws_scope,
                config: &config,
                embedder: embedder_ref.as_ref(),
                // Phase 12: the meta seed reflects the ACTIVE (remote-or-bundled)
                // embedder identity, proven above to match the stored identity.
                embedder_seed: active_embedder_seed.clone(),
                reranker_seed: r_seed,
                summariser_seed: s_seed,
                allow_model_download: false,
            };

            // Capture each enabled plugin's CURRENT (pre-reindex) version so we
            // can detect a real version change after the reindex below — the
            // attributed `plugin_updated` event carries `from`/`to` (FR-056).
            // Read-only, best-effort: any miss yields an empty string and is
            // diffed normally (a blank→version transition still counts as a
            // change, which is correct for a freshly-indexed plugin).
            let old_versions = read_plugin_versions(&paths, &ws_name, &catalog_name, &enabled);

            let outcome = reindex_catalog_plugins(&catalog_name, &enabled, &deps)?;
            emit_reindex_outcome(mode, &catalog_name, &outcome)?;

            // FR-052 + FR-056: ALONGSIDE the anonymous `tome.catalog_action`
            // emitted once per refreshed catalog above, emit one attributed
            // `catalog.<id>.plugin_updated` per plugin whose version CHANGED —
            // but ONLY when this workspace's catalog resolves, by SOURCE at emit
            // time, to an allowlisted catalog. Resolved per (workspace, catalog)
            // so a multi-workspace refresh attributes correctly. Best-effort:
            // the attribution + version reads never lock and never fail the run.
            let ws_resolved = crate::workspace::ResolvedScope {
                scope: ws_scope.clone(),
                source: crate::workspace::ScopeSource::Flag,
                project_root: None,
            };
            if let Some(catalog_id) =
                crate::telemetry::resolve_attribution(&ws_resolved, &catalog_name)
            {
                let new_versions = read_plugin_versions(&paths, &ws_name, &catalog_name, &enabled);
                for plugin_name in &enabled {
                    let from = old_versions.get(plugin_name).cloned().unwrap_or_default();
                    let to = new_versions.get(plugin_name).cloned().unwrap_or_default();
                    if from == to {
                        continue;
                    }
                    crate::telemetry::emit(crate::telemetry::event::PluginUpdated {
                        catalog: catalog_id,
                        plugin_name: plugin_name.clone(),
                        from_version: from,
                        to_version: to,
                    });
                }
            }

            // FR-365 + FR-385 + FR-423: regenerate the workspace's
            // cached summary if any plugin in this catalog changed
            // identity. Walk the per-plugin changes and fire the
            // trigger once per workspace+catalog when any change
            // landed (added / modified / removed skills, or an
            // auto-disabled plugin).
            if catalog_reindex_changed_anything(&outcome) {
                let ws_name = ws_scope.name().clone();
                crate::summarise::regenerate_for_trigger(&ws_name, &paths)?;
            }
        }
    }

    Ok(())
}

/// FR-365 gate: did at least one plugin in this catalog actually
/// change skill identity (or get auto-disabled) for this workspace?
/// Reindex-of-an-unchanged-tree is a no-op for summarisation.
fn catalog_reindex_changed_anything(outcome: &CatalogReindexOutcome) -> bool {
    outcome.plugins.iter().any(|change| {
        if change.auto_disabled.is_some() {
            return true;
        }
        match &change.summary {
            Some(s) => s.added > 0 || s.modified > 0 || s.removed > 0,
            None => false,
        }
    })
}

/// Hand-rolled alias so tests can pull this through. `&ResolvedScope`
/// from the workspace module.
pub type ResolvedScopeArg = crate::workspace::ResolvedScope;

#[derive(Debug, Clone)]
struct RefreshTarget {
    url: String,
    pinned_ref: String,
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

fn load_embedder(
    cfg: &Config,
    paths: &Paths,
) -> Result<Box<dyn crate::embedding::Embedder>, TomeError> {
    // B4 / Phase 12: build the ACTIVE (remote-or-bundled) embedder. The B3 guard
    // above has already proven the configured embedder matches the stored one,
    // so this constructs the embedder that produced the index's vectors. On the
    // remote path the validator's expected dimension is seeded from the
    // persisted `meta.embedder_dimension`.
    let (e_seed, r_seed, s_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: e_seed,
            reranker: r_seed,
            summariser: s_seed,
            profile: None,
        },
    )?;
    let entry = meta::active_embedder(&conn)?;
    let persisted_dim =
        if crate::provider::resolve(cfg, crate::provider::Capability::Embedding)?.is_some() {
            meta::read_embedder_dimension(&conn)?
        } else {
            None
        };
    crate::embedding::build_embedder(cfg, paths, entry, persisted_dim)
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
            profile: None,
        },
    )?;
    enabled_plugins_for_catalog(&conn, workspace, catalog)
}

/// Best-effort read of each plugin's `plugin_version` from the index, keyed by
/// plugin name, for the catalog-attributed `plugin_updated` event (Phase 10 /
/// US4). Opens the central index READ-ONLY with NO advisory lock (NFR-009).
///
/// Infallible: any failure (missing DB, query error) yields an empty map, and a
/// plugin with no row is simply absent (its `from`/`to` default to empty at the
/// diff site). The versions are PUBLISHED manifest values (the FR-059 carve-out).
fn read_plugin_versions(
    paths: &Paths,
    workspace: &str,
    catalog: &str,
    plugins: &[String],
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    let Ok(conn) = index::open_read_only(&paths.index_db) else {
        return out;
    };
    for plugin in plugins {
        if let Ok(rows) = crate::index::skills::list_for_plugin(&conn, workspace, catalog, plugin)
            && let Some(row) = rows.into_iter().next()
        {
            out.insert(plugin.clone(), row.plugin_version);
        }
    }
    out
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
    // Third-party manifest: cap at PLUGIN_MANIFEST_MAX (FR-006,
    // F-PLUGIN-MANIFEST-DOS). `bounded_read` preserves the exit-7 `Io`
    // contract for both real I/O errors and an over-cap file.
    let manifest_bytes =
        crate::util::bounded_read(&manifest_path, crate::util::PLUGIN_MANIFEST_MAX)?;
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
    // Third-party manifest, best-effort display name: cap at
    // PLUGIN_MANIFEST_MAX (FR-006). An over-cap file is `Err` → `.ok()?` →
    // None, the same fallback an unreadable/unparsable manifest takes here.
    let bytes = crate::util::bounded_read(&manifest_path, crate::util::PLUGIN_MANIFEST_MAX).ok()?;
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

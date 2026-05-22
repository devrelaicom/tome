//! `tome catalog remove`. See `contracts/catalog-remove.md`, the Phase 2
//! extension at `contracts/catalog-extensions.md` §"tome catalog remove",
//! and the Phase 4 FR-363 / FR-366 / FR-367 flow.
//!
//! Phase 4 / F11b: the enrolment lives in `workspace_catalogs`; the
//! cache directory lives at `paths.cache_dir_for(&url)`. The
//! advisory-lock window covers the cascade-disable + DELETE + cache
//! cleanup (FR-366). Concurrent removes of the same `(workspace,
//! catalog)` serialise on the lock; the loser observes
//! `CatalogNotFound` (FR-367 benign-race outcome).

use std::io::{BufRead, Write};

use serde::Serialize;
use tracing::warn;

use crate::cli::CatalogRemoveArgs;
use crate::commands::plugin::registry_seeds;
use crate::error::TomeError;
use crate::index::{
    self, OpenOptions, acquire_lock, enabled_plugins_for_catalog, mark_all_disabled_for_plugin,
    workspace_catalogs,
};
use crate::output;
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

pub fn run(args: CatalogRemoveArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let workspace_name = scope.scope.name().as_str().to_owned();

    // Open the central DB read-side first to surface CatalogNotFound /
    // CatalogHasEnabledPlugins before the prompt. The advisory lock is
    // taken later once the user has confirmed.
    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: embedder_seed.clone(),
            reranker: reranker_seed.clone(),
            summariser: summariser_seed.clone(),
        },
    )?;

    let enrolment = workspace_catalogs::find(&conn, &workspace_name, &args.name)?
        .ok_or_else(|| TomeError::CatalogNotFound(args.name.clone()))?;
    let cache_path = paths.cache_dir_for(&enrolment.url);

    // Enabled-plugin pre-check is cheap (single SELECT DISTINCT). The
    // lock isn't taken yet — readers don't block writers, and the worst
    // outcome is reporting a stale enabled list. The cascade itself
    // re-reads under the lock.
    let enabled_plugins = enabled_plugins_for_catalog(&conn, &workspace_name, &args.name)?;
    if !enabled_plugins.is_empty() && !args.force {
        let plugins_qualified = enabled_plugins
            .iter()
            .map(|p| format!("{}/{}", args.name, p))
            .collect();
        return Err(TomeError::CatalogHasEnabledPlugins {
            catalog: args.name.clone(),
            plugins: plugins_qualified,
        });
    }

    if !args.force {
        if !output::stdin_is_tty() {
            return Err(TomeError::Usage(
                "'tome catalog remove' requires --force in non-interactive contexts".into(),
            ));
        }
        if !prompt_yes_no(&format!(
            "Remove catalog '{}' and its local cache at {}? [y/N]",
            args.name,
            cache_path.display()
        ))? {
            return Ok(());
        }
    }

    // Drop the unlocked connection before grabbing the lock — we'll
    // reopen under the lock so the cascade + DELETE + cache cleanup
    // all observe a consistent snapshot. FR-367: a concurrent remove
    // racing this one serialises on `index.lock`; the second observer
    // finds the row gone in its post-lock `find` and reports
    // `CatalogNotFound` from the explicit re-check below.
    drop(conn);

    let mut cascade_records: Vec<CascadeRecord> = Vec::new();
    let cache_path_for_lock = cache_path.clone();

    // Acquire the advisory lock across the cascade-disable + DELETE +
    // cache cleanup (FR-366). Per-step rusqlite work runs inline rather
    // than calling cascade_disable_for_catalog (which acquires its own
    // lock — recursive flock is unsafe across processes / fds).
    let lock = acquire_lock(&paths.index_lock)?;
    let outcome: Result<(), TomeError> = (|| {
        let conn = index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: embedder_seed,
                reranker: reranker_seed,
                summariser: summariser_seed,
            },
        )?;

        if !enabled_plugins.is_empty() {
            // Cascade-disable inline: drop each plugin's
            // workspace_skills enrolment. The underlying skills rows
            // are retained per FR-383. Output identical to the
            // cascade_disable_for_catalog helper's behaviour minus the
            // nested lock acquisition.
            cascade_records.reserve(enabled_plugins.len());
            for plugin in &enabled_plugins {
                let dropped =
                    mark_all_disabled_for_plugin(&conn, &workspace_name, &args.name, plugin)?;
                cascade_records.push(CascadeRecord {
                    plugin: format!("{}/{}", args.name, plugin),
                    skills_dropped: dropped,
                });
            }
            if mode == Mode::Human {
                let mut out = std::io::stdout().lock();
                writeln!(
                    out,
                    "Cascading disable of {} enabled plugin{}:",
                    enabled_plugins.len(),
                    if enabled_plugins.len() == 1 { "" } else { "s" },
                )?;
                for plugin in &enabled_plugins {
                    writeln!(out, "  ✓ {}/{}", args.name, plugin)?;
                }
            }
        }

        let removed = workspace_catalogs::delete(&conn, &workspace_name, &args.name)?;
        if !removed {
            // Lost the race with another remover. Report
            // `CatalogNotFound` per FR-367.
            return Err(TomeError::CatalogNotFound(args.name.clone()));
        }

        // Cache cleanup: only when no other workspace still references
        // this URL. Errors are warn + continue — the enrolment row is
        // gone, which is the source-of-truth atomicity.
        let refs = workspace_catalogs::refcount_by_url(&conn, &enrolment.url)?;
        if refs == 0 {
            if let Err(e) = std::fs::remove_dir_all(&cache_path_for_lock)
                && e.kind() != std::io::ErrorKind::NotFound
            {
                warn!(
                    cache_path = %cache_path_for_lock.display(),
                    error = %e,
                    "cache directory could not be removed; junction row already updated",
                );
            }
        } else {
            tracing::debug!(
                cache_path = %cache_path_for_lock.display(),
                still_referenced_by = refs,
                "cache directory retained; still referenced by other workspaces",
            );
        }

        Ok(())
    })();

    drop(lock);
    outcome?;

    let removed_rec = RemovedView {
        name: args.name.clone(),
        url: enrolment.url.clone(),
        cache_path: cache_path.display().to_string(),
    };
    emit(mode, &removed_rec, &cascade_records)?;
    Ok(())
}

fn prompt_yes_no(prompt: &str) -> Result<bool, TomeError> {
    let mut stderr = std::io::stderr().lock();
    write!(stderr, "{} ", prompt)?;
    stderr.flush()?;
    drop(stderr);

    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let answer = line.trim().to_ascii_lowercase();
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

struct RemovedView {
    name: String,
    url: String,
    cache_path: String,
}

#[derive(Serialize)]
struct RemovedEnvelope<'a> {
    removed: RemovedRecord<'a>,
}

#[derive(Serialize)]
struct RemovedRecord<'a> {
    name: &'a str,
    url: &'a str,
    cache_path: &'a str,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cascade: Vec<CascadeRecord>,
}

#[derive(Serialize, Clone)]
struct CascadeRecord {
    plugin: String,
    skills_dropped: u32,
}

fn emit(mode: Mode, rec: &RemovedView, cascade: &[CascadeRecord]) -> Result<(), TomeError> {
    match mode {
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            writeln!(
                out,
                "Removed catalog `{}` (cache cleared at {}).",
                rec.name, rec.cache_path
            )?;
        }
        Mode::Json => {
            let env = RemovedEnvelope {
                removed: RemovedRecord {
                    name: &rec.name,
                    url: &rec.url,
                    cache_path: &rec.cache_path,
                    cascade: cascade.to_vec(),
                },
            };
            crate::output::write_json(&env)?;
        }
    }
    Ok(())
}

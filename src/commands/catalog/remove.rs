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
use std::sync::RwLock;

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
            profile: None,
        },
    )?;

    let enrolment = workspace_catalogs::find(&conn, &workspace_name, &args.name)?
        .ok_or_else(|| TomeError::CatalogNotFound(args.name.clone()))?;
    let cache_path = paths.cache_dir_for(&enrolment.url);

    // Enabled-plugin pre-check is cheap (single SELECT DISTINCT). The
    // lock isn't taken yet — readers don't block writers, and the worst
    // outcome is reporting a stale enabled list in the advisory
    // `CatalogHasEnabledPlugins` error / `--force` prompt. This snapshot
    // is used ONLY for that pre-lock advisory; the cascade below
    // re-derives the enabled set from the connection opened under the
    // lock (F-REMOVE-TOCTOU) so a `plugin enable` racing into the window
    // between here and the lock is not missed.
    let prelock_enabled_plugins = enabled_plugins_for_catalog(&conn, &workspace_name, &args.name)?;
    if !prelock_enabled_plugins.is_empty() && !args.force {
        let plugins_qualified = prelock_enabled_plugins
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

    // Test-only seam (no-op in production): a concurrent `plugin enable`
    // may land here — after the stale-tolerant pre-lock read, before the
    // lock is taken. The cascade below must observe such an enable; see
    // F-REMOVE-TOCTOU and `tests/catalog_remove_toctou.rs`.
    fire_after_prelock_read_hook();

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
                profile: None,
            },
        )?;

        // Re-derive the cascade input UNDER THE LOCK (F-REMOVE-TOCTOU).
        // The pre-lock snapshot drove the advisory `--force` decision but
        // is stale by now: a concurrent `plugin enable` (also serialising
        // on `index.lock`) may have enrolled a NEW plugin in this catalog
        // since. Cascading the stale Vec would leave that plugin enabled
        // against a catalog row we are about to delete — a ghost-enabled
        // plugin. Re-reading from the under-lock connection makes the
        // cascade input the current enabled set: either the enable landed
        // first (and we disable it here) or it lands after our DELETE (and
        // observes the row gone) — never a ghost.
        let enabled_plugins = enabled_plugins_for_catalog(&conn, &workspace_name, &args.name)?;

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

    // R-M6 (US4.d-1): after a successful cascade-disable of one-or-more
    // enabled plugins, regenerate the workspace's cached summary so
    // the MCP tool description and `RULES.md` reflect the (shrunk)
    // enabled set. Mirrors the pattern used by `plugin disable` (which
    // triggers regen unconditionally on every disable). `ModelMissing`
    // is a silent no-op per the contract's trigger-callers carve-out;
    // any other summariser failure bubbles as exit 24.
    //
    // The regen call is OUTSIDE the advisory lock: `regenerate_for_trigger`
    // takes its own lock internally via `regen_summary::regen`, and
    // nesting advisory locks across processes is unsafe.
    if !cascade_records.is_empty() {
        crate::summarise::regenerate_for_trigger(scope.scope.name(), &paths)?;
    }

    let removed_rec = RemovedView {
        name: args.name.clone(),
        url: enrolment.url.clone(),
        cache_path: cache_path.display().to_string(),
    };
    emit(mode, &removed_rec, &cascade_records)?;

    // best-effort: the stored enrolment url is `file://` for a local-path
    // source; every remote shape is Git.
    let source_type = if enrolment.url.starts_with("file://") {
        crate::telemetry::event::SourceType::Local
    } else {
        crate::telemetry::event::SourceType::Git
    };
    crate::telemetry::emit(crate::telemetry::event::CatalogActionEvent {
        action: crate::telemetry::event::CatalogAction::Removed,
        source_type,
    });

    Ok(())
}

/// Test-only seam fired exactly once, **after** the stale-tolerant
/// pre-lock enabled-plugin read and **before** [`acquire_lock`] — i.e.
/// inside the TOCTOU window this command must close (F-REMOVE-TOCTOU).
///
/// Integration tests under `tests/` cannot reach `#[cfg(test)]` hooks
/// (they consume the library as an external crate). Per the project
/// convention for test injection — `#[doc(hidden)] pub static` + an RAII
/// guard in the consuming test — this slot lets a test deterministically
/// simulate a concurrent `plugin enable` landing in the window (enrolling
/// a *new* plugin in the catalog) so the cascade's re-read under the lock
/// can be asserted without a flaky cross-process race.
///
/// Production never sets the slot; [`fire_after_prelock_read_hook`]
/// collapses to a no-op on every real invocation, so the single-process
/// happy path is byte-for-byte unchanged.
#[doc(hidden)]
#[allow(clippy::type_complexity)]
pub static AFTER_PRELOCK_READ_HOOK: RwLock<Option<Box<dyn Fn() + Send + Sync>>> = RwLock::new(None);

/// Invoke [`AFTER_PRELOCK_READ_HOOK`] if a test installed one. A poisoned
/// lock is recovered rather than propagated — a panicking hook in one
/// test must not wedge the slot for the next.
fn fire_after_prelock_read_hook() {
    let guard = AFTER_PRELOCK_READ_HOOK
        .read()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(hook) = guard.as_ref() {
        hook();
    }
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

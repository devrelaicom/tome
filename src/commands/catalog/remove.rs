//! `tome catalog remove`. See `contracts/catalog-remove.md` and the
//! Phase 2 extension at `contracts/catalog-extensions.md` §"tome catalog
//! remove".
//!
//! Cache removal is best-effort: failures are logged at WARN and do not
//! propagate. The registry write is the source-of-truth atomicity guarantee.
//!
//! Phase 2 extension: when the catalog has enabled plugins in the index,
//! `tome catalog remove` refuses with exit 53 (`CatalogHasEnabledPlugins`).
//! Passing `--force` cascades disable + row drop for each enabled plugin
//! inside a single index-lock window, then proceeds with the Phase 1 flow.

use std::io::{BufRead, Write};

use serde::Serialize;
use tracing::warn;

use crate::catalog::store;
use crate::cli::CatalogRemoveArgs;
use crate::config::CatalogEntry;
use crate::error::TomeError;
use crate::index::{self, OpenOptions, enabled_plugins_for_catalog};
use crate::output;
use crate::output::Mode;
use crate::paths::Paths;
use crate::plugin::lifecycle::cascade_disable_for_catalog;
use crate::workspace::{ResolvedScope, Scope};

use crate::commands::plugin::registry_seeds;

pub fn run(args: CatalogRemoveArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    // F2a: single global config; F11 will reintroduce workspace-aware scope.
    let config_file = paths.global_config_file.clone();
    let mut config = store::load(&config_file)?;

    let entry = config
        .catalogs
        .get(&args.name)
        .ok_or_else(|| TomeError::CatalogNotFound(args.name.clone()))?
        .clone();

    // Phase 2 pre-check: enabled plugins in this catalog refuse the remove
    // unless `--force` is set. The query is cheap (single SELECT DISTINCT)
    // and runs without the advisory lock — readers don't block writers,
    // and the worst case is we report a stale enabled list that the
    // cascade itself will then act on consistently.
    let enabled_plugins = read_enabled_plugins(&paths, &scope.scope, &args.name)?;
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
            entry.name,
            entry.path.display()
        ))? {
            // Declined — exit 0 with no mutation.
            return Ok(());
        }
    }

    // Cascade disable, if we have any enabled plugins. Only reached on
    // `--force`; the no-force-but-enabled path errored above.
    let mut cascade_records: Vec<CascadeRecord> = Vec::new();
    if !enabled_plugins.is_empty() {
        let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
        let breakdown = cascade_disable_for_catalog(
            &paths,
            scope.scope.name().as_str(),
            &args.name,
            &enabled_plugins,
            embedder_seed,
            reranker_seed,
            summariser_seed,
        )?;
        cascade_records.reserve(breakdown.len());
        for (plugin, dropped) in &breakdown {
            cascade_records.push(CascadeRecord {
                plugin: format!("{}/{}", args.name, plugin),
                skills_dropped: *dropped,
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

    config.catalogs.remove(&args.name);
    store::save(&config_file, &config)?;

    // Phase 3 reference-counting: only delete the on-disk clone when no
    // other scope still references the same URL. The check runs AFTER
    // the config write per `catalog-extensions-p3.md`, so a crash
    // between the two steps leaves the clone alive — the user re-adds.
    let refs = store::reference_count(&entry.url, &paths);
    if refs.is_empty() {
        if let Err(e) = std::fs::remove_dir_all(&entry.path) {
            warn!(
                cache_path = %entry.path.display(),
                error = %e,
                "cache directory could not be removed; registry already updated"
            );
        }
    } else {
        tracing::debug!(
            cache_path = %entry.path.display(),
            still_referenced_by = ?refs,
            "cache directory retained; still referenced by other scope(s)",
        );
    }

    emit(mode, &entry, &cascade_records)?;
    Ok(())
}

/// Read the distinct enabled plugin names for one catalog under the
/// resolved workspace. Returns an empty vector when the index database
/// has not been bootstrapped yet (the `catalog remove` flow must still
/// work on a fresh install).
fn read_enabled_plugins(
    paths: &Paths,
    scope: &Scope,
    catalog: &str,
) -> Result<Vec<String>, TomeError> {
    let index_db = paths.index_db.clone();
    if !index_db.is_file() {
        return Ok(Vec::new());
    }
    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let conn = index::open(
        &index_db,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
            summariser: summariser_seed,
        },
    )?;
    enabled_plugins_for_catalog(&conn, scope.name().as_str(), catalog)
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

#[derive(Serialize)]
struct RemovedEnvelope<'a> {
    removed: RemovedRecord<'a>,
}

#[derive(Serialize)]
struct RemovedRecord<'a> {
    name: &'a str,
    url: &'a str,
    cache_path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    cascade: Vec<CascadeRecord>,
}

#[derive(Serialize, Clone)]
struct CascadeRecord {
    plugin: String,
    skills_dropped: u32,
}

fn emit(mode: Mode, entry: &CatalogEntry, cascade: &[CascadeRecord]) -> Result<(), TomeError> {
    match mode {
        Mode::Human => {
            let mut out = std::io::stdout().lock();
            writeln!(
                out,
                "Removed catalog `{}` (cache cleared at {}).",
                entry.name,
                entry.path.display()
            )?;
        }
        Mode::Json => {
            let env = RemovedEnvelope {
                removed: RemovedRecord {
                    name: &entry.name,
                    url: &entry.url,
                    cache_path: entry.path.display().to_string(),
                    cascade: cascade.to_vec(),
                },
            };
            crate::output::write_json(&env)?;
        }
    }
    Ok(())
}

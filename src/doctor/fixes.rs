//! `tome doctor --fix` — automatic repairs.
//!
//! Three repair classes per `contracts/doctor.md` §`--fix` semantics:
//!
//! 1. **Model missing / corrupt / checksum-mismatched** → re-download via
//!    `embedding::download::download_model`.
//! 2. **Catalog cache missing / not-a-repo** → re-clone via
//!    `catalog::git::Git::clone_shallow` at the recorded URL + ref.
//! 3. **Schema older than expected** → `index::migrations::apply_pending`
//!    under the resolved scope's advisory lock.
//!
//! Each repair runs in order; if one fails, doctor records the failure
//! and continues with the next. The affected subsystem's check
//! function is re-run after each repair so the report reflects
//! post-repair state.
//!
//! Repairs marked `auto_fixable = false` in the suggested-fix list
//! (manifest invalid, drift, schema-too-new, orphan clones) are NOT
//! attempted by `--fix`; they remain in the post-repair report and
//! drive the exit-75 path.

use tracing::warn;

use crate::catalog::git::Git;
use crate::catalog::store as catalog_store;
use crate::commands::plugin::{embedder_entry, registry_seeds, reranker_entry};
use crate::commands::status::{check_index, check_model};
use crate::doctor::checks::check_catalogs;
use crate::doctor::report::{DoctorReport, SuggestedFix};
use crate::embedding::download::download_model;
use crate::embedding::registry::ModelEntry;
use crate::error::TomeError;
use crate::index::{self, OpenOptions, acquire_lock, migrations};
use crate::paths::Paths;
use crate::workspace::Scope;

/// Attempt every `auto_fixable: true` suggested fix in `report`. On
/// success, the affected subsystem's check is re-run and the report's
/// matching field is updated in place. Failures are logged and the
/// report's pre-repair state is preserved for that subsystem (so the
/// developer sees what doctor tried + what remained broken).
///
/// Returns the number of attempted repairs (succeeded or failed). The
/// caller re-classifies + re-emits.
pub fn apply(report: &mut DoctorReport, paths: &Paths, scope: &Scope) -> Result<usize, TomeError> {
    let mut attempts = 0;

    // Snapshot the auto-fixable suggestions before mutating the report,
    // because the post-repair check functions mutate `report.embedder` /
    // `report.reranker` / `report.catalogs` / `report.index` in place.
    let fixes: Vec<SuggestedFix> = report
        .suggested_fixes
        .iter()
        .filter(|f| f.auto_fixable)
        .cloned()
        .collect();

    for fix in fixes {
        attempts += 1;
        let result = apply_one(&fix, report, paths, scope);
        if let Err(e) = result {
            warn!(
                subsystem = %fix.subsystem,
                error = %e,
                "doctor --fix: repair attempt failed; report retained pre-repair state",
            );
        }
    }

    Ok(attempts)
}

fn apply_one(
    fix: &SuggestedFix,
    report: &mut DoctorReport,
    paths: &Paths,
    scope: &Scope,
) -> Result<(), TomeError> {
    if fix.subsystem == "embedder" {
        let entry = embedder_entry();
        repair_model(entry, paths)?;
        report.embedder = check_model(paths, entry, false)?;
        return Ok(());
    }
    if fix.subsystem == "reranker" {
        let entry = reranker_entry();
        repair_model(entry, paths)?;
        report.reranker = check_model(paths, entry, false)?;
        return Ok(());
    }
    if let Some(name) = fix.subsystem.strip_prefix("catalog:") {
        repair_catalog(name, paths, scope)?;
        report.catalogs = check_catalogs(paths, scope)?;
        return Ok(());
    }
    if fix.subsystem == "schema" {
        repair_schema(paths, scope)?;
        report.index = check_index(paths, scope)?;
        return Ok(());
    }
    // Unknown auto_fixable subsystem — shouldn't happen but log + skip.
    warn!(
        subsystem = %fix.subsystem,
        "doctor --fix: no repair implementation for subsystem; skipping",
    );
    Ok(())
}

fn repair_model(entry: &ModelEntry, paths: &Paths) -> Result<(), TomeError> {
    // Clear any partial install so `download_model`'s rename-into-place
    // can land cleanly. `download_model` itself handles the
    // partial-suffix dir; we additionally remove the final-named dir
    // because corruption (e.g. wrong-size primary file) leaves the
    // manifest+files in place — a fresh download wouldn't replace
    // them.
    let model_dir = paths.model_path(entry.name)?;
    if model_dir.exists() {
        std::fs::remove_dir_all(&model_dir).map_err(TomeError::Io)?;
    }
    download_model(entry, &paths.models_dir)?;
    Ok(())
}

fn repair_catalog(name: &str, paths: &Paths, scope: &Scope) -> Result<(), TomeError> {
    let config = catalog_store::load(&paths.config_file_for(scope))?;
    let entry = config
        .catalogs
        .get(name)
        .ok_or_else(|| TomeError::CatalogNotFound(name.to_owned()))?
        .clone();

    // The clone destination must not exist. Remove any half-broken
    // cache before re-cloning. This is the same best-effort cleanup
    // pattern Phase 1's `catalog add` uses on rollback.
    if entry.path.exists() {
        std::fs::remove_dir_all(&entry.path).map_err(TomeError::Io)?;
    }
    if let Some(parent) = entry.path.parent() {
        std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    }

    let git = Git::new(&entry.name);
    git.clone_shallow(&entry.url, &entry.path, Some(&entry.ref_))?;
    Ok(())
}

fn repair_schema(paths: &Paths, scope: &Scope) -> Result<(), TomeError> {
    let db_path = paths.index_db_for(scope);
    if !db_path.is_file() {
        // No DB on disk → nothing to migrate. Not an error.
        return Ok(());
    }
    let (embedder_seed, reranker_seed) = registry_seeds();
    let lock_path = paths.index_lock_for(scope);
    let _lock = acquire_lock(&lock_path)?;
    let mut conn = index::open(
        &db_path,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
        },
    )?;
    let current = migrations::current_schema_version(&conn)?.unwrap_or(index::SCHEMA_VERSION);
    let _ = migrations::apply_pending(&mut conn, current, index::SCHEMA_VERSION)?;
    Ok(())
}

/// `true` when the report still has `auto_fixable: false` suggestions
/// after `--fix` ran. Drives the exit-75 path.
pub fn has_remaining_manual_fixes(report: &DoctorReport) -> bool {
    report.suggested_fixes.iter().any(|f| !f.auto_fixable)
}

/// Re-derive the suggested-fix list + classification after `--fix` has
/// mutated the per-subsystem fields. The caller assembles the initial
/// report once; this entry point produces the post-repair version
/// without re-running the catalog or harness probes (they're already
/// up-to-date from `apply_one`).
pub fn re_assemble(report: &mut DoctorReport) {
    use crate::doctor::{build_suggested_fixes_pub, classify_pub};
    report.suggested_fixes = build_suggested_fixes_pub(
        &report.embedder,
        &report.reranker,
        &report.index,
        &report.drift,
        &report.catalogs,
    );
    report.overall = classify_pub(
        &report.embedder,
        &report.reranker,
        &report.index,
        &report.drift,
        &report.catalogs,
    );
}

//! `tome doctor --fix` — automatic repairs.
//!
//! Repair classes per `contracts/doctor.md` §`--fix` semantics +
//! `contracts/doctor-extensions-p4.md` §Fix classes:
//!
//! 1. **Model missing / corrupt / checksum-mismatched** → re-download via
//!    `embedding::download::download_model`.
//! 2. **Catalog cache missing / not-a-repo** → re-clone via
//!    `catalog::git::Git::clone_shallow` at the recorded URL + ref.
//! 3. **Schema older than expected** → `index::migrations::apply_pending`
//!    under the resolved scope's advisory lock.
//! 4. **Summariser missing** → re-download via
//!    `summarise::download::download_summariser_model`.
//! 5. **BindingRulesCopy missing / drift** → re-copy
//!    `<root>/workspaces/<name>/RULES.md` to every bound project's
//!    marker via `workspace::sync::sync_one`.
//! 6. **HarnessRules / HarnessMcp drift / broken** → re-run
//!    `harness::sync::sync_project` for the project (the orchestrator is
//!    idempotent per FR-525, so re-running rewrites only the drifted /
//!    broken files; healthy files land in `leave_alones`). The full-sync
//!    invocation is simpler than a per-harness slice and benefits from
//!    the orchestrator's clash + cleanup discipline.
//!
//! Each repair runs in order; if one fails, doctor records the failure
//! and continues with the next. The affected subsystem's check
//! function is re-run after each repair so the report reflects
//! post-repair state.
//!
//! Repairs marked `auto_fixable = false` in the suggested-fix list
//! (manifest invalid, drift, schema-too-new, orphan clones, binding
//! broken, user-owned MCP without `--force`) are NOT attempted by plain
//! `--fix`; they remain in the post-repair report and drive the exit-75
//! path. The US5.b `--force` flag overrides the user-owned MCP refusal
//! only; binding-broken remains a developer-choice gate.

use std::path::Path;

use tracing::warn;

use crate::catalog::git::Git;
use crate::commands::plugin::{embedder_entry, registry_seeds, reranker_entry};
use crate::commands::status::{check_index, check_model};
use crate::doctor::binding::check_binding;
use crate::doctor::checks::check_catalogs;
use crate::doctor::harness_integration::check_harness_integration;
use crate::doctor::report::{DoctorReport, Subsystem, SuggestedFix};
use crate::embedding::download::download_model;
use crate::embedding::registry::ModelEntry;
use crate::error::TomeError;
use crate::index::{self, OpenOptions, acquire_lock, migrations, workspace_catalogs};
use crate::paths::Paths;
use crate::summarise::download::download_summariser_model;
use crate::summarise::registry::summariser_entry;
use crate::workspace::{ResolvedScope, Scope};

/// Carries the inputs every per-subsystem repair needs. Grouped into a
/// struct because the Phase 4 set of repairs (binding-rules-copy,
/// harness-rules, harness-mcp) needs the project root + home root in
/// addition to the Phase 3 `(paths, scope)` pair. Bundling avoids a
/// `fn apply_one(fix, report, paths, scope, home, force, ...)` signature
/// that would only grow as we add subsystems.
#[derive(Debug)]
pub struct FixContext<'a> {
    pub paths: &'a Paths,
    pub scope: &'a ResolvedScope,
    /// `$HOME` for harness-integration probes. Caller-supplied so
    /// integration tests can isolate against a tempdir.
    pub home: &'a Path,
    /// When `true`, harness-mcp repairs override developer-authored
    /// `tome` entries (the US5.b `--force` path). When `false`, those
    /// entries are NOT rewritten — they remain `auto_fixable: false`
    /// in the post-repair report and drive the exit-75 path.
    pub force: bool,
}

/// Attempt every `auto_fixable: true` suggested fix in `report`. With
/// `force = true`, additionally retry every `harness-mcp:*` fix that
/// was classified `auto_fixable: false` because of a user-owned entry —
/// the `--force` flag is the explicit override path. On success, the
/// affected subsystem's check is re-run and the report's matching field
/// is updated in place. Failures are logged and the report's pre-repair
/// state is preserved for that subsystem (so the developer sees what
/// doctor tried + what remained broken).
///
/// Returns the number of attempted repairs (succeeded or failed). The
/// caller re-classifies + re-emits.
///
/// FR-M-DOC-4: the signature is infallible by design — every per-fix
/// failure is downgraded to a `warn!` and reflected in the post-pass
/// report's residual `suggested_fixes`. Returning `Result` was
/// misleading and tempted future callers to add `?` to `apply_one`,
/// which would silently break the "continue on failure" contract.
pub fn apply(report: &mut DoctorReport, ctx: &FixContext<'_>) -> usize {
    let mut attempts = 0;

    // Snapshot the auto-fixable suggestions before mutating the report,
    // because the post-repair check functions mutate `report.embedder` /
    // `report.reranker` / `report.catalogs` / `report.index` in place.
    // Additionally, when `force` is set, pick up the user-owned MCP
    // fixes that were intentionally classified non-auto-fixable: the
    // override path runs the same dispatch.
    let fixes: Vec<SuggestedFix> = report
        .suggested_fixes
        .iter()
        .filter(|f| {
            f.auto_fixable || (ctx.force && matches!(&f.subsystem, Subsystem::HarnessMcp(_)))
        })
        .cloned()
        .collect();

    for fix in fixes {
        attempts += 1;
        if let Err(e) = apply_one(&fix, report, ctx) {
            warn!(
                subsystem = %fix.subsystem,
                error = %e,
                "doctor --fix: repair attempt failed; report retained pre-repair state",
            );
        }
    }

    attempts
}

fn apply_one(
    fix: &SuggestedFix,
    report: &mut DoctorReport,
    ctx: &FixContext<'_>,
) -> Result<(), TomeError> {
    let paths = ctx.paths;
    let scope = &ctx.scope.scope;
    match &fix.subsystem {
        Subsystem::Embedder => {
            let entry = embedder_entry();
            repair_model(entry, paths)?;
            report.embedder = check_model(paths, entry, false)?;
            Ok(())
        }
        Subsystem::Reranker => {
            let entry = reranker_entry();
            repair_model(entry, paths)?;
            report.reranker = check_model(paths, entry, false)?;
            Ok(())
        }
        Subsystem::Catalog(name) => {
            repair_catalog(name, paths, scope)?;
            report.catalogs = check_catalogs(paths, scope)?;
            Ok(())
        }
        Subsystem::Schema => {
            repair_schema(paths, scope)?;
            report.index = check_index(paths, scope)?;
            Ok(())
        }
        Subsystem::Summariser => {
            repair_summariser(paths)?;
            report.summariser = check_model(paths, summariser_entry(), false)?;
            Ok(())
        }
        Subsystem::BindingRulesCopy => {
            repair_binding_rules_copy(ctx)?;
            // Re-run the binding probe so `rules_file_drift` reflects
            // the re-copy.
            report.project_binding = check_binding(ctx.scope, paths);
            Ok(())
        }
        Subsystem::HarnessRules(_) | Subsystem::HarnessMcp(_) => {
            // Single sync pass repairs both subsystems for every harness
            // in the effective list. Idempotent (FR-525); we let the
            // orchestrator do the per-harness write decisions.
            repair_harness_sync(ctx)?;
            // Re-run the per-harness integration probe to refresh
            // both `harness_rules` and `harness_mcp` in the report.
            if let (Some(list), Some(binding)) = (
                report.effective_harness_list.as_ref(),
                report.project_binding.as_ref(),
            ) {
                let (rules, mcp) =
                    check_harness_integration(&binding.project_root, list, ctx.home, scope.name());
                report.harness_rules = rules;
                report.harness_mcp = mcp;
            }
            Ok(())
        }
        // Subsystems that should never carry `auto_fixable: true` (Drift,
        // Index, Binding, Catalog::Orphan, …). If one slips through it's
        // a build-time defect — warn loud and continue.
        other => {
            warn!(
                subsystem = %other,
                "doctor --fix: no auto-repair handler is registered for this subsystem; \
                 this is a Tome bug — please report it. Leaving pre-repair state in place.",
            );
            Ok(())
        }
    }
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
    download_model(entry, &paths.models_dir, None)?;
    Ok(())
}

fn repair_catalog(name: &str, paths: &Paths, scope: &Scope) -> Result<(), TomeError> {
    let workspace_name = scope.name().as_str();
    let (e_seed, r_seed, s_seed) = registry_seeds();
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: e_seed,
            reranker: r_seed,
            summariser: s_seed,
        },
    )?;
    let enrolment = workspace_catalogs::find(&conn, workspace_name, name)?
        .ok_or_else(|| TomeError::CatalogNotFound(name.to_owned()))?;
    drop(conn);

    let cache_path = paths.cache_dir_for(&enrolment.url);

    // The clone destination must not exist. Remove any half-broken
    // cache before re-cloning. This is the same best-effort cleanup
    // pattern Phase 1's `catalog add` uses on rollback.
    if cache_path.exists() {
        std::fs::remove_dir_all(&cache_path).map_err(TomeError::Io)?;
    }
    if let Some(parent) = cache_path.parent() {
        std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    }

    let git = Git::new(name);
    git.clone_shallow(&enrolment.url, &cache_path, Some(&enrolment.pinned_ref))?;
    Ok(())
}

fn repair_summariser(paths: &Paths) -> Result<(), TomeError> {
    // Phase 4 / F6 surfaces the byte-progress callback seam on
    // `download_summariser_model`; doctor passes `None` because the
    // doctor pass is not the interactive context where indicatif lives
    // (regen-summary's CLI surface owns that). Mirror the embedder /
    // reranker repair: clear the on-disk dir first so the rename
    // lands cleanly.
    let entry = summariser_entry();
    let model_dir = paths.model_path(entry.name)?;
    if model_dir.exists() {
        std::fs::remove_dir_all(&model_dir).map_err(TomeError::Io)?;
    }
    download_summariser_model(paths, None)?;
    Ok(())
}

fn repair_binding_rules_copy(ctx: &FixContext<'_>) -> Result<(), TomeError> {
    // Re-copy `<root>/workspaces/<name>/RULES.md` → every bound project's
    // `<project>/.tome/RULES.md`. `workspace::sync::sync_one` is the
    // existing idempotent surface; doctor calls it for the resolved
    // workspace and the bound project will be picked up by the
    // `workspace_projects` walk inside.
    //
    // When the suggested fix fires we know `ctx.scope.scope` carries
    // the workspace whose RULES.md the project is supposed to mirror.
    let _ = crate::workspace::sync::sync_one(ctx.scope.scope.name(), ctx.paths)?;
    Ok(())
}

fn repair_harness_sync(ctx: &FixContext<'_>) -> Result<(), TomeError> {
    // Resolve the project root from the report's binding; if there is
    // none, there's nothing to sync — the harness suggested fix
    // shouldn't have fired in the first place.
    let Some(project_root) = ctx.scope.project_root.as_deref() else {
        warn!(
            "doctor --fix: harness sync requested but the resolved scope \
             has no project root; skipping",
        );
        return Ok(());
    };
    let sync_deps =
        crate::harness::sync::build_deps(ctx.paths, ctx.home, ctx.scope.scope.name(), ctx.force);
    crate::harness::sync::sync_project(project_root, &sync_deps)?;
    Ok(())
}

fn repair_schema(paths: &Paths, _scope: &Scope) -> Result<(), TomeError> {
    let db_path = paths.index_db.clone();
    if !db_path.is_file() {
        // No DB on disk → nothing to migrate. Not an error.
        return Ok(());
    }
    let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
    let lock_path = paths.index_lock.clone();
    let _lock = acquire_lock(&lock_path)?;
    let mut conn = index::open(
        &db_path,
        &OpenOptions {
            embedder: embedder_seed,
            reranker: reranker_seed,
            summariser: summariser_seed,
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
        &report.summariser,
        &report.index,
        &report.drift,
        &report.catalogs,
        report.project_binding.as_ref(),
        &report.harness_rules,
        &report.harness_mcp,
    );
    report.overall = classify_pub(
        &report.embedder,
        &report.reranker,
        &report.summariser,
        &report.index,
        &report.drift,
        &report.catalogs,
        report.project_binding.as_ref(),
        &report.harness_rules,
        &report.harness_mcp,
    );
}

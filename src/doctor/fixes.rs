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
use crate::commands::plugin::registry_seeds;

/// B4: the ACTIVE profile's embedder for the doctor `--fix` path. Read-only
/// `meta` resolution when the DB exists; default profile on a fresh install.
fn active_embedder_for_fix(
    paths: &crate::paths::Paths,
) -> Result<&'static crate::embedding::registry::ModelEntry, TomeError> {
    if paths.index_db.is_file() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        crate::index::meta::active_embedder(&conn)
    } else {
        Ok(crate::embedding::profile::embedder_for(
            crate::embedding::profile::Profile::DEFAULT,
        ))
    }
}

/// B4: the ACTIVE profile's reranker for the doctor `--fix` path. Companion to
/// [`active_embedder_for_fix`].
fn active_reranker_for_fix(
    paths: &crate::paths::Paths,
) -> Result<&'static crate::embedding::registry::ModelEntry, TomeError> {
    if paths.index_db.is_file() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        crate::index::meta::active_reranker(&conn)
    } else {
        Ok(crate::embedding::profile::reranker_for(
            crate::embedding::profile::Profile::DEFAULT,
        ))
    }
}
use crate::commands::status::{check_index, check_model};
use crate::doctor::binding::check_binding;
use crate::doctor::checks::check_catalogs;
use crate::doctor::harness_integration::check_harness_integration;
use crate::doctor::report::{DoctorReport, Subsystem, SubsystemHealth, SuggestedFix};
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
    //
    // R-M2: harness syncs are coalesced. `HarnessRules(_)` and
    // `HarnessMcp(_)` fixes both dispatch to the same per-project
    // `harness::sync::sync_project` orchestrator (which is idempotent
    // per FR-525 — it rewrites only the drifted/broken slices). Running
    // it once per fix is 10 redundant passes when 1 would do. Collect
    // those fixes separately, dispatch once, and account each as one
    // attempt for the residual-classification ledger.
    let mut harness_fixes: Vec<SuggestedFix> = Vec::new();
    let mut other_fixes: Vec<SuggestedFix> = Vec::new();
    for fix in report.suggested_fixes.iter() {
        if !(fix.auto_fixable || (ctx.force && matches!(&fix.subsystem, Subsystem::HarnessMcp(_))))
        {
            continue;
        }
        match &fix.subsystem {
            Subsystem::HarnessRules(_) | Subsystem::HarnessMcp(_) => {
                harness_fixes.push(fix.clone());
            }
            _ => other_fixes.push(fix.clone()),
        }
    }

    // S-M2: the `--force` path rewrites user-owned MCP entries by
    // (re-)running the per-project sync orchestrator with `force = true`.
    // We must only apply force to harnesses that have an outstanding
    // `UserOwned`-class fix in THIS pass — not blanket-rewrite every
    // user-owned entry across every declared harness. The orchestrator
    // itself operates per-harness, so we capture the set here and gate
    // the sync invocation on whether ANY user-owned fix participated.
    let user_owned_harnesses_in_play: std::collections::HashSet<String> = harness_fixes
        .iter()
        .filter_map(|f| match &f.subsystem {
            Subsystem::HarnessMcp(name) => report
                .harness_mcp
                .iter()
                .find(|h| h.harness == *name && h.health == SubsystemHealth::UserOwned)
                .map(|h| h.harness.clone()),
            _ => None,
        })
        .collect();

    for fix in other_fixes {
        attempts += 1;
        if let Err(e) = apply_one(&fix, report, ctx) {
            warn!(
                subsystem = %fix.subsystem,
                error = %e,
                "doctor --fix: repair attempt failed; report retained pre-repair state",
            );
        }
    }

    // Dedup: dispatch the harness sync exactly once if any harness fix
    // was queued. Every queued harness fix counts as one attempt so the
    // residual-classification accounting matches the per-fix model.
    //
    // C5-1: track whether ANY project-context `sync_project` succeeded
    // during this pass — from EITHER the harness-fixes branch below OR the
    // Phase 6 branch. The Phase 6 project surfaces are refreshed once after
    // the dispatch, gated on that flag, so a HarnessRules/HarnessMcp fix
    // (which already re-renders guardrails + re-emits agents through the
    // same orchestrator) doesn't leave `report.hooks`/`guardrails`/`agents`
    // showing stale pre-repair state (FR-091).
    let mut project_sync_succeeded = false;
    if !harness_fixes.is_empty() {
        attempts += harness_fixes.len();
        // S-M2: only enable the force path when one of the in-play
        // fixes is user-owned. Otherwise stick with the no-force sync
        // even if the caller passed `force = true` — `--force` without
        // a user-owned fix is a no-op intent.
        let effective_force = ctx.force && !user_owned_harnesses_in_play.is_empty();
        if let Err(e) = repair_harness_sync_with(ctx, effective_force) {
            warn!(
                subsystem = "harness-sync",
                error = %e,
                "doctor --fix: harness sync attempt failed; report retained pre-repair state",
            );
        } else {
            project_sync_succeeded = true;
            // Re-run the per-harness probe so both rules + mcp reflect
            // the post-sync state.
            if let (Some(list), Some(binding)) = (
                report.effective_harness_list.as_ref(),
                report.project_binding.as_ref(),
            ) {
                let (rules, mcp) = check_harness_integration(
                    &binding.project_root,
                    list,
                    ctx.home,
                    ctx.scope.scope.name(),
                );
                report.harness_rules = rules;
                report.harness_mcp = mcp;
            }
        }
    }

    // FR-091: Phase 6 safe repairs (re-render stale guardrails regions,
    // re-emit missing agent files, remove orphaned `<plugin>__*` agent
    // files). These surfaces are informational — they never produce a
    // `SuggestedFix`, so they don't trigger the harness-fixes branch above.
    // But `--fix` still repairs them by re-running the per-project sync
    // orchestrator (idempotent per FR-525): it re-renders drifted
    // guardrails, re-emits missing agents, and removes orphans, while the
    // hooks merge only ever appends/removes structural matches — it NEVER
    // strips a non-matching/user-edited hook (NFR-003) nor deletes
    // user-authored content (rules-file text outside Tome markers,
    // hand-written agents not matching `<plugin>__*`). Hooks DRIFT
    // (expected-but-missing) is reported, not auto-fixed — re-merge on the
    // next sync is the remediation. Run the sync once if it has not already
    // run for the harness-fixes branch and there is repairable Phase 6
    // drift in a project context. NEVER force here (the Phase 6 surfaces do
    // not consent to overriding user-owned MCP entries).
    if !project_sync_succeeded
        && ctx.scope.project_root.is_some()
        && phase6_has_repairable_drift(report)
    {
        attempts += 1;
        if let Err(e) = repair_harness_sync_with(ctx, false) {
            warn!(
                subsystem = "harness-sync",
                error = %e,
                "doctor --fix: Phase 6 guardrails/agents sync attempt failed; \
                 report retained pre-repair state",
            );
        } else {
            project_sync_succeeded = true;
        }
    }

    // C5-1: refresh the three project-relative Phase 6 surfaces once after
    // ANY successful project-context sync — whether it was the
    // harness-fixes branch (HarnessRules/HarnessMcp) or the Phase 6 branch
    // that triggered it. The orchestrator re-renders guardrails, re-emits
    // agents, and re-merges hooks regardless of which fix class invoked it,
    // so the post-`--fix` report must reflect that post-repair state. The
    // full re-assemble path is heavier than needed; refresh just the three
    // project-relative surfaces via their read-only checks.
    if project_sync_succeeded {
        refresh_phase6_project_surfaces(report, ctx);
    }

    attempts
}

/// `true` when the report's Phase 6 surfaces carry drift that the safe
/// re-sync repairs: any present/orphaned guardrails region, any agent file
/// present-or-orphaned. Hooks drift is intentionally excluded (reported, not
/// auto-fixed). A surface that is `None` (outside-project) contributes
/// nothing.
fn phase6_has_repairable_drift(report: &DoctorReport) -> bool {
    let guardrails_drift = report
        .guardrails
        .as_ref()
        .is_some_and(|g| g.files.iter().any(|f| !f.present.is_empty()));
    let agents_drift = report.agents.as_ref().is_some_and(|a| {
        a.harnesses
            .iter()
            .any(|h| !h.present.is_empty() || !h.orphaned.is_empty())
    });
    // Even with no on-disk regions/files yet, a workspace with enabled
    // plugins that ship guardrails/agents should have them emitted. The
    // surfaces only list what is already on disk, so also re-sync when the
    // index has enabled agents (the report's entry_counts captures this).
    let pending_agent_emission = report.entry_counts.as_ref().is_some_and(|c| c.agents > 0);
    guardrails_drift || agents_drift || pending_agent_emission
}

/// Re-build the three project-relative Phase 6 surfaces (hooks / guardrails
/// / agents) after a `--fix` re-sync so the post-repair report reflects the
/// re-rendered / re-emitted / orphan-removed state. Read-only — the same
/// check functions the assembler runs. A re-open / per-surface failure
/// leaves the pre-repair surface in place.
fn refresh_phase6_project_surfaces(report: &mut DoctorReport, ctx: &FixContext<'_>) {
    let Some(project_root) = ctx.scope.project_root.as_deref() else {
        return;
    };
    if !ctx.paths.index_db.is_file() {
        return;
    }
    let conn = match index::open_read_only(&ctx.paths.index_db) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "doctor --fix: re-open for Phase 6 refresh failed; retaining surfaces");
            return;
        }
    };
    let workspace = ctx.scope.scope.name();
    if let Ok(r) =
        crate::doctor::checks::build_hooks_report(ctx.paths, project_root, workspace, &conn)
    {
        report.hooks = Some(r);
    }
    if let Ok(r) =
        crate::doctor::checks::build_guardrails_report(ctx.paths, project_root, workspace, &conn)
    {
        report.guardrails = Some(r);
    }
    if let Ok(r) =
        crate::doctor::checks::build_agents_report(ctx.paths, project_root, workspace, &conn)
    {
        report.agents = Some(r);
    }
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
            // B4: repair the ACTIVE profile's embedder (resolved from `meta`;
            // default profile on a fresh install), not the hard-coded default.
            let entry = active_embedder_for_fix(paths)?;
            repair_model(entry, paths)?;
            report.embedder = check_model(paths, entry, false)?;
            Ok(())
        }
        Subsystem::Reranker => {
            let entry = active_reranker_for_fix(paths)?;
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
        Subsystem::Index => {
            // Phase 12 / US4 (FR-017): the ONLY `auto_fixable` Index fix is the
            // corrupt-remote-index repair on a BUNDLED-local embedder — re-run
            // the idempotent `reindex --force` (which acquires the advisory lock
            // itself). A REMOTE-embedder corrupt-index fix is `auto_fixable:
            // false` (it would incur paid API cost) and so never reaches this
            // handler. `index::check_index` is re-run so the report reflects the
            // rebuilt index.
            repair_corrupt_index(ctx)?;
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
            // Unreachable: `apply()` coalesces all harness fixes into a
            // single `repair_harness_sync_with` invocation outside the
            // `apply_one` dispatch (R-M2). If a caller dispatches one
            // directly we still want safe behaviour, so fall through to
            // the (one-shot) sync.
            repair_harness_sync_with(ctx, ctx.force)?;
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
    // R-M7: enrolment lookup is read-only — use `open_read_only` so we
    // don't request a writer's WAL pragmas / advisory lock just to query
    // the URL+ref. The subsequent `Git::clone_shallow` doesn't touch the
    // DB.
    let conn = index::open_read_only(&paths.index_db)?;
    let enrolment = workspace_catalogs::find(&conn, workspace_name, name)?
        .ok_or_else(|| TomeError::CatalogNotFound(name.to_owned()))?;
    drop(conn);

    let cache_path = paths.cache_dir_for(&enrolment.url);

    // S-M4: defence-in-depth invariant — `cache_dir_for` deterministically
    // maps URL → `<catalogs_dir>/<sha256-of-url>/`. The path MUST be a
    // descendant of `paths.catalogs_dir`. If it ever isn't (a future
    // refactor of `cache_dir_for` that breaks the invariant, or a
    // malformed paths struct in a test), the `remove_dir_all` below
    // becomes a foot-gun against arbitrary FS state. Crash the test
    // build via `debug_assert!` so the bug surfaces locally; release
    // builds keep the assertion out of the hot path but the invariant
    // is documented for future readers.
    debug_assert!(
        cache_path.starts_with(&paths.catalogs_dir),
        "cache_dir must be under catalogs_dir for safety: {} not under {}",
        cache_path.display(),
        paths.catalogs_dir.display(),
    );

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
    // C-M3: re-copy `<root>/workspaces/<name>/RULES.md` → THIS project's
    // `<project>/.tome/RULES.md` ONLY. The legacy `workspace::sync::sync_one`
    // walks every bound project of the workspace; for a project-local
    // doctor pass that is wrong — a hand-edited sibling project would be
    // silently overwritten. We target the resolved scope's `project_root`
    // exclusively via `sync_one_project`.
    let Some(project_root) = ctx.scope.project_root.as_deref() else {
        // The suggested fix should never have fired without a project
        // root (it's gated on `project_binding.is_some()`). Surface a
        // warn and no-op; the residual fix stays in the list.
        warn!(
            "doctor --fix: binding-rules-copy fix requested but the \
             resolved scope has no project root; skipping",
        );
        return Ok(());
    };
    let _ =
        crate::workspace::sync::sync_one_project(ctx.scope.scope.name(), ctx.paths, project_root)?;
    Ok(())
}

/// Single-pass harness sync that lets the caller override the `force`
/// bit independently of `ctx.force`. R-M2 / S-M2 lean on this to:
/// 1. dispatch the orchestrator exactly once per `apply()` invocation
///    even when multiple harnesses surfaced fixes, and
/// 2. only enable the force path when an in-play user-owned-MCP fix
///    actually consented to it.
fn repair_harness_sync_with(ctx: &FixContext<'_>, force: bool) -> Result<(), TomeError> {
    let Some(project_root) = ctx.scope.project_root.as_deref() else {
        warn!(
            "doctor --fix: harness sync requested but the resolved scope \
             has no project root; skipping",
        );
        return Ok(());
    };
    let sync_deps =
        crate::harness::sync::build_deps(ctx.paths, ctx.home, ctx.scope.scope.name(), force);
    crate::harness::sync::sync_project(project_root, &sync_deps)?;
    Ok(())
}

/// Phase 12 / US4 (FR-017): repair a corrupt-remote-index on a BUNDLED-local
/// embedder by re-running the existing `tome reindex --force` over the resolved
/// scope. This re-derives every stored vector from the bundled model, so the
/// stored dimension realigns with the index's expectations. `reindex::run`
/// acquires the advisory lock itself (the doctor read-only-projection +
/// idempotent-op pattern — we don't re-implement reindex). Human mode is used
/// because the doctor `--fix` is a foreground command; the reindex progress
/// surfaces inline.
fn repair_corrupt_index(ctx: &FixContext<'_>) -> Result<(), TomeError> {
    use crate::cli::ReindexArgs;
    crate::commands::reindex::run(
        ReindexArgs {
            scope: None,
            force: true,
        },
        ctx.scope,
        crate::output::Mode::Human,
    )
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
            profile: None,
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

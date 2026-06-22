//! `tome doctor` — broad diagnostic.
//!
//! Library-API entry point `assemble_report` is the silent-compute path;
//! the CLI wrapper (slice US4.b — `commands::doctor`) adds emit + exit
//! semantics. Tests target `assemble_report` directly so they don't have
//! to spawn the binary.
//!
//! Doctor classification overlaps with `tome status` for the
//! models / index / drift subsystems and adds two new ones: catalog
//! caches + harness presence. The status helpers (`check_model`,
//! `check_index`, `check_drift`) are reused — single source of truth.

pub mod binding;
pub mod checks;
pub mod cutover;
pub mod fixes;
pub mod harness_detect;
pub mod harness_integration;
pub mod meta_drift;
pub mod orphan_cleanup;
pub mod report;
pub mod telemetry;

use std::path::Path;

use crate::commands::status::{check_drift, check_index, check_model};
use crate::commands::workspace::info::assemble as assemble_workspace_info;
use crate::error::TomeError;
use crate::index::meta::DriftStatus;
use crate::paths::Paths;
use crate::settings::{GlobalSettings, ProjectMarkerConfig, WorkspaceSettings};
use crate::summarise::registry::summariser_entry;

/// B4: resolve the ACTIVE profile's `(embedder, reranker)` registry entries.
/// Opens the index read-only when present; on a fresh install (no DB yet) it
/// falls back to the default profile, which the bootstrap will stamp.
fn active_models(
    paths: &Paths,
) -> Result<
    (
        &'static crate::embedding::registry::ModelEntry,
        &'static crate::embedding::registry::ModelEntry,
    ),
    TomeError,
> {
    if paths.index_db.is_file() {
        let conn = crate::index::open_read_only(&paths.index_db)?;
        Ok((
            crate::index::meta::active_embedder(&conn)?,
            crate::index::meta::active_reranker(&conn)?,
        ))
    } else {
        use crate::embedding::profile::{Profile, embedder_for, reranker_for};
        Ok((
            embedder_for(Profile::DEFAULT),
            reranker_for(Profile::DEFAULT),
        ))
    }
}
use crate::workspace::ResolvedScope;

pub use report::{
    CatalogCacheHealth, CatalogCacheState, DoctorClassification, DoctorReport, EntryCountsByKind,
    HarnessPresence, HarnessSubsystemReport, MetaSkillDrift, OrphanDataDirReport,
    ProjectBindingState, PromptsReport, RulesCopyState, Subsystem, SubsystemHealth, SuggestedFix,
    TelemetryAllowlistEntry, TelemetryFlushReport, TelemetryIdReport, TelemetryQueueReport,
    TelemetrySection,
};

/// Build a [`DoctorReport`] from the on-disk state. Read-only; never
/// acquires the advisory lock. `verify = true` rehashes the primary
/// embedder + reranker artefacts against the registry SHA-256s.
///
/// `home` is the directory under which `harness_detect::probe` looks
/// for `~/.claude/` etc. Tests can substitute an isolated temp dir;
/// production passes `$HOME`.
pub fn assemble_report(
    scope: &ResolvedScope,
    paths: &Paths,
    home: &Path,
    verify: bool,
) -> Result<DoctorReport, TomeError> {
    let tome_version = env!("CARGO_PKG_VERSION").to_owned();

    // `assemble_workspace_info` errors with `WorkspaceNotFound` when the
    // resolved scope names a workspace that no longer has a row in the
    // central registry — exactly the orphan-binding case the doctor
    // pass is meant to surface. Catch that one variant and fall through
    // with a synthetic workspace block; every other error still bubbles
    // because they're real DB / integrity failures that doctor itself
    // shouldn't paper over.
    let workspace = match assemble_workspace_info(scope, paths) {
        Ok(info) => info,
        Err(TomeError::WorkspaceNotFound { .. }) => {
            use crate::workspace::{ScopeKind, WorkspaceInfo};
            WorkspaceInfo {
                scope: if scope.scope.is_global() {
                    ScopeKind::Global
                } else {
                    ScopeKind::Workspace
                },
                path: scope.project_root.clone(),
                source: scope.source,
                catalogs: 0,
                plugins_total: 0,
                plugins_enabled: 0,
                skills_indexed: 0,
                schema_version: None,
                embedder: None,
                enrolled_catalogs: Vec::new(),
                enabled_plugins: Vec::new(),
                bound_projects: Vec::new(),
                summary_cache: None,
                plugin_details: None,
            }
        }
        Err(e) => return Err(e),
    };

    // B4: the doctor checks only the ACTIVE profile's models (resolved from
    // the index `meta`; default profile on a fresh install). Reporting every
    // profile's models would surface spurious "missing" rows after Phase 2.
    let (embedder_e, reranker_e) = active_models(paths)?;
    let summariser_e = summariser_entry();
    let embedder = check_model(paths, embedder_e, verify)?;
    let reranker = check_model(paths, reranker_e, verify)?;
    let summariser = check_model(paths, summariser_e, verify)?;

    // C-M2 / FR-561: doctor never crashes. `check_index` can return
    // `SchemaTooNew` (exit 52) or `IndexIntegrityCheckFailure` (exit 51)
    // from `index::open_read_only`; both are user-actionable failure
    // surfaces that doctor should *report*, not propagate. Collapse to
    // a `present: true, integrity_ok: false` `IndexHealth` so the
    // overall classifier flips to Unhealthy and the report still emits.
    let index = check_index(paths, &scope.scope).unwrap_or_else(|err| {
        tracing::warn!(error = %err, "doctor: check_index failed; reporting Broken state");
        // R-m5 (US5.c): when `check_index` errors but the file IS on
        // disk (e.g. `SchemaTooNew`, parse failure on `meta` row), the
        // file's byte size is still observable cheaply. The prior
        // `size_bytes: 0` was misleading because it conflated
        // "errored" with "absent". `present` already encodes presence.
        let size_bytes = std::fs::metadata(&paths.index_db)
            .map(|m| m.len())
            .unwrap_or(0);
        crate::commands::status::IndexHealth {
            present: true,
            schema_version: None,
            plugins_enabled: 0,
            skills_indexed: 0,
            size_bytes,
            integrity_ok: false,
        }
    });
    let drift = check_drift(paths, &scope.scope, embedder_e, reranker_e)?;
    let catalogs = checks::check_catalogs(paths, &scope.scope)?;
    let workspace_registry = checks::check_workspace_registry(paths);
    let harnesses = harness_detect::probe(home);

    // ---- Phase 4 / US5.a additions ----------------------------------
    let project_binding = binding::check_binding(scope, paths);

    // Resolve the effective harness list from the layered settings.
    // FR-564: from outside any project marker the project layer is empty
    // and we fall through to the global declarations.
    let effective_harness_list = build_effective_harness_list(scope, paths).unwrap_or(None);

    // Per-harness rules + MCP integration state.
    let (harness_rules, harness_mcp) = match (&effective_harness_list, &project_binding) {
        (Some(list), Some(binding)) => harness_integration::check_harness_integration(
            &binding.project_root,
            list,
            home,
            &scope.scope.name().clone(),
        ),
        // C-M1: harnesses ARE declared but we have no project context to
        // resolve project-relative paths against. Emit per-harness
        // `NotApplicable` entries (one per declared harness, in source
        // order) so JSON consumers can distinguish "no harnesses declared
        // globally" (effective_harness_list = None → empty Vec) from
        // "harnesses declared but no project context" (empty list-of-
        // Some-harness). Classification stays unaffected per FR-561.
        (Some(list), None) => {
            let rules: Vec<HarnessSubsystemReport> = list
                .harnesses
                .iter()
                .map(|h| HarnessSubsystemReport {
                    harness: h.name.clone(),
                    health: SubsystemHealth::NotApplicable,
                })
                .collect();
            let mcp = rules.clone();
            (rules, mcp)
        }
        // No declared harnesses at all → empty vectors.
        _ => (Vec::new(), Vec::new()),
    };

    // FR-560: harnesses present on the local machine via
    // `HarnessModule::detect` but NOT in the effective list.
    let detected_uninstalled_harnesses =
        collect_detected_uninstalled(home, effective_harness_list.as_ref());

    // ---- Phase 5 / US5.b additions ----------------------------------
    //
    // R-M5 (US5.c): the three Phase 5 surfaces emit `(None, None,
    // None)` ONLY for `ScopeSource::GlobalFallback` — i.e. the
    // implicit fallback to the privileged `global` workspace when no
    // `--workspace`, env var, or `.tome/config.toml` was found.
    // Explicit `--workspace global` resolves through `ScopeSource::Flag`
    // and DOES populate the surfaces (a user inspecting global has
    // intent; an unbound shell-out does not). Preserving this
    // distinction keeps the Phase 4 byte-stable JSON shape of the
    // existing `doctor_json_shape_is_byte_stable_for_minimal_report`
    // test pin.
    //
    // FR-124 read-only invariant: none of these functions lazy-create
    // plugin-data / workspace-data dirs. `build_prompts_report` reuses
    // the same registry walk the MCP server runs at startup;
    // `detect_orphan_data_dirs` is `fs::read_dir` only;
    // `count_entries_by_kind` is pure SQL + `fs::metadata`.
    let (prompts, orphan_data_dirs, entry_counts) =
        build_phase5_surfaces(scope, paths).unwrap_or((None, None, None));

    // ---- Phase 6 / US5 additions ------------------------------------
    //
    // Same `GlobalFallback`/no-DB gating as Phase 5 (mirrors
    // `build_phase5_surfaces`). The three project-relative surfaces
    // (hooks / guardrails / agents) additionally require a resolved
    // project root; the privilege-escalation + persona surfaces only
    // need the DB + workspace. Persona is `None` when
    // `expose_agents_as_personas` resolves false at the doctor scope.
    //
    // FR-124 read-only invariant: every check function under
    // `build_phase6_surfaces` only `fs::read`s / `read_dir`s / queries the
    // index. Persona names are derived from frontmatter + entry rows
    // without invoking substitution or creating any directory.
    let Phase6Surfaces {
        hooks,
        guardrails,
        agents,
        privilege_escalation,
        personas,
    } = build_phase6_surfaces(scope, paths).unwrap_or_default();

    let suggested_fixes = build_suggested_fixes(
        &embedder,
        &reranker,
        &summariser,
        &index,
        &drift,
        &catalogs,
        project_binding.as_ref(),
        &harness_rules,
        &harness_mcp,
    );
    let overall = classify(
        &embedder,
        &reranker,
        &summariser,
        &index,
        &drift,
        &catalogs,
        project_binding.as_ref(),
        &harness_rules,
        &harness_mcp,
    );

    // Phase 8 cutover surfaces (read-only). The migration of any legacy model
    // `manifest.json` runs under `--fix` (in the command layer); the report
    // here only surfaces what would be migrated / converted.
    let legacy_model_manifests = cutover::legacy_model_manifests(paths);
    let catalog_cache_roots: Vec<std::path::PathBuf> =
        catalogs.iter().map(|c| c.cache_path.clone()).collect();
    let unconverted_plugins = cutover::unconverted_plugins(&catalog_cache_roots)
        .iter()
        .map(|p| p.display().to_string())
        .collect();

    // Phase 9 / US4 meta-skill drift (read-only, FR-031). Re-derives the
    // installer's (harness × scope) candidate set from the supported-harness
    // set and probes each via the shared `meta::drift_probe`. Surfaces only
    // `stale` rows (missing is "not installed", not drift — empty when clean →
    // the wire shape stays byte-stable). `--fix` repair lives in the command
    // layer (refreshes in place; never creates new installs).
    let meta_skills = meta_drift::check(home, scope);

    // Phase 10 / US5 (FR-064): the read-only telemetry subsystem projection.
    // Infallible by construction (a malformed config is reported, not
    // propagated); read-only (FR-124 — mints nothing, creates no dir). Always
    // populated by `assemble_report`; the `Option` + `skip_serializing_if`
    // keeps the byte-stable minimal-report pin (which builds the struct literal
    // with `telemetry: None`) unchanged.
    let telemetry = Some(telemetry::assemble(paths));

    Ok(DoctorReport {
        tome_version,
        workspace,
        project_binding,
        embedder,
        reranker,
        summariser,
        index,
        drift,
        catalogs,
        workspace_registry,
        harnesses,
        effective_harness_list,
        harness_rules,
        harness_mcp,
        detected_uninstalled_harnesses,
        prompts,
        orphan_data_dirs,
        entry_counts,
        hooks,
        guardrails,
        agents,
        privilege_escalation,
        personas,
        legacy_model_manifests,
        unconverted_plugins,
        meta_skills,
        telemetry,
        overall,
        suggested_fixes,
    })
}

/// The five Phase 6 / US5 doctor surfaces. All `None` under
/// `GlobalFallback` / no-DB; the three project-relative surfaces are also
/// `None` without a resolved project root; `personas` is additionally
/// `None` when `expose_agents_as_personas` resolves false at the scope.
#[derive(Default)]
struct Phase6Surfaces {
    hooks: Option<report::HooksReport>,
    guardrails: Option<report::GuardrailsReport>,
    agents: Option<report::AgentsReport>,
    privilege_escalation: Option<report::PrivilegeEscalationReport>,
    personas: Option<report::PersonaReport>,
}

/// Resolve the five Phase 6 surfaces for the active scope, mirroring
/// [`build_phase5_surfaces`]'s `GlobalFallback` + no-DB + read-only-DB
/// gating. Returns the all-`None` default when the scope is `GlobalFallback`
/// or the index DB is absent / unopenable.
fn build_phase6_surfaces(
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<Phase6Surfaces, TomeError> {
    use crate::workspace::ScopeSource;
    if matches!(scope.source, ScopeSource::GlobalFallback) {
        return Ok(Phase6Surfaces::default());
    }
    if !paths.index_db.is_file() {
        return Ok(Phase6Surfaces::default());
    }

    let conn = match crate::index::open_read_only(&paths.index_db) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "doctor phase 6 surfaces: open_read_only failed; emitting None");
            return Ok(Phase6Surfaces::default());
        }
    };

    let workspace_name = scope.scope.name();
    let mut out = Phase6Surfaces::default();

    // Project-relative surfaces (hooks / guardrails / agents) require a
    // resolved project root — without one there is no `.claude/` or
    // harness target to inspect, so they stay `None` (outside-project
    // mode). Each surface degrades to `None` on its own error rather than
    // failing the whole doctor pass.
    if let Some(project_root) = scope.project_root.as_deref() {
        out.hooks = match checks::build_hooks_report(paths, project_root, workspace_name, &conn) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!(error = %e, "doctor: build_hooks_report failed; emitting None");
                None
            }
        };
        out.guardrails = match checks::build_guardrails_report(
            paths,
            project_root,
            workspace_name,
            &conn,
        ) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!(error = %e, "doctor: build_guardrails_report failed; emitting None");
                None
            }
        };
        out.agents = match checks::build_agents_report(paths, project_root, workspace_name, &conn) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!(error = %e, "doctor: build_agents_report failed; emitting None");
                None
            }
        };
    }

    out.privilege_escalation = match checks::build_privilege_escalation_report(
        paths,
        workspace_name,
        &conn,
    ) {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::warn!(error = %e, "doctor: build_privilege_escalation_report failed; emitting None");
            None
        }
    };

    // Persona surface only when `expose_agents_as_personas` resolves true.
    // The resolver swallows malformed-settings errors to `false` here so
    // doctor never crashes; the dedicated settings/binding surfaces
    // classify a malformed marker.
    let expose = crate::mcp::resolve_expose_personas(scope, paths).unwrap_or(false);
    if expose {
        out.personas = match checks::build_persona_report(workspace_name, &conn) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::warn!(error = %e, "doctor: build_persona_report failed; emitting None");
                None
            }
        };
    }

    Ok(out)
}

/// Resolve the three Phase 5 surfaces (`prompts`, `orphan_data_dirs`,
/// `entry_counts`) for the active scope. Returns `(None, None, None)`
/// when:
/// - The scope is `GlobalFallback` (no explicit workspace context).
/// - The index DB doesn't exist on disk yet (fresh install).
/// - Opening the DB fails (a doctor surface that cannot itself observe
///   the workspace state degrades gracefully; the embedder/index
///   subsystem checks already classify the underlying failure).
///
/// All three surfaces are READ-ONLY per FR-124. None of them
/// lazy-create plugin-data, workspace-data, or workspace settings
/// directories.
type Phase5Surfaces = (
    Option<report::PromptsReport>,
    Option<report::OrphanDataDirReport>,
    Option<report::EntryCountsByKind>,
);

fn build_phase5_surfaces(
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<Phase5Surfaces, TomeError> {
    use crate::workspace::ScopeSource;
    if matches!(scope.source, ScopeSource::GlobalFallback) {
        return Ok((None, None, None));
    }
    if !paths.index_db.is_file() {
        return Ok((None, None, None));
    }

    // Read-only DB handle — the same convention as the Phase 4
    // surfaces. The bootstrap-on-first-open path is NOT taken here
    // because we already checked `is_file()`; this prevents the
    // doctor pass from writing meta seeds when the user's DB hasn't
    // been bootstrapped yet.
    let conn = match crate::index::open_read_only(&paths.index_db) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "doctor phase 5 surfaces: open_read_only failed; emitting None");
            return Ok((None, None, None));
        }
    };

    let workspace_name = scope.scope.name();

    let prompts = match checks::build_prompts_report(workspace_name, paths, &conn) {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::warn!(error = %e, "doctor: build_prompts_report failed; emitting None");
            None
        }
    };

    let orphan_data_dirs = match checks::detect_orphan_data_dirs(paths, &conn) {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::warn!(error = %e, "doctor: detect_orphan_data_dirs failed; emitting None");
            None
        }
    };

    let entry_counts = match checks::count_entries_by_kind(workspace_name, paths, &conn) {
        Ok(r) => Some(r),
        Err(e) => {
            tracing::warn!(error = %e, "doctor: count_entries_by_kind failed; emitting None");
            None
        }
    };

    Ok((prompts, orphan_data_dirs, entry_counts))
}

/// Resolve the effective harness list using the same layered walk as
/// `tome harness sync`, but tolerant of every parse / composition
/// failure: doctor is the diagnostic that surfaces those failures, so
/// it must not itself crash on them. Failures collapse to `None` (no
/// effective list) and the harness-integration checks degrade to
/// `NotApplicable`. The detected-uninstalled-harnesses note is still
/// produced regardless.
fn build_effective_harness_list(
    scope: &ResolvedScope,
    paths: &Paths,
) -> Result<Option<crate::settings::resolver::EffectiveHarnessList>, TomeError> {
    use crate::commands::harness::CentralDbScopeProvider;
    use crate::settings::parser::{parse_global, parse_workspace, read_project_marker};
    use crate::settings::resolver::resolve_effective_list;

    // Polish R-M5: route through canonical reader and discard any
    // error (the doctor surface intentionally swallows malformed
    // project markers here — the project_binding check is the place
    // where parse failures classify as `Binding::Broken`).
    let project_marker: Option<ProjectMarkerConfig> = scope
        .project_root
        .as_deref()
        .and_then(|root| read_project_marker(&Paths::project_marker_config(root)).ok());

    let workspace_settings: Option<WorkspaceSettings> = {
        let path = paths.workspace_settings_file(scope.scope.name());
        crate::util::bounded_read_to_string(&path, crate::util::TOME_CONFIG_MAX)
            .ok()
            .and_then(|body| parse_workspace(&body).ok())
    };

    let global_settings: GlobalSettings = crate::util::bounded_read_to_string(
        &paths.global_settings_file,
        crate::util::TOME_CONFIG_MAX,
    )
    .ok()
    .and_then(|body| parse_global(&body).ok())
    .unwrap_or_default();

    let scope_provider = CentralDbScopeProvider::new(paths);
    match resolve_effective_list(
        project_marker.as_ref(),
        workspace_settings.as_ref(),
        &global_settings,
        &scope_provider,
    ) {
        Ok(list) if list.harnesses.is_empty() && list.excluded.is_empty() => Ok(None),
        Ok(list) => Ok(Some(list)),
        Err(_) => Ok(None),
    }
}

/// Per FR-560: harnesses whose per-user dir exists on the local machine
/// (via `HarnessModule::detect`) but who are NOT in the effective list.
/// Reported informationally; never affects classification.
fn collect_detected_uninstalled(
    home: &Path,
    effective: Option<&crate::settings::resolver::EffectiveHarnessList>,
) -> Vec<String> {
    use crate::harness::with_effective_modules;

    let live: std::collections::HashSet<String> = effective
        .map(|l| l.harnesses.iter().map(|h| h.name.clone()).collect())
        .unwrap_or_default();

    with_effective_modules(|modules| {
        let mut out: Vec<String> = modules
            .iter()
            .filter(|m| m.detect(home))
            .map(|m| m.name().to_owned())
            .filter(|n| !live.contains(n))
            .collect();
        out.sort();
        out.dedup();
        out
    })
}

/// `pub(crate)` so `doctor::fixes::re_assemble` can call it after
/// repairs mutate the per-subsystem fields. Not part of the public API.
#[allow(clippy::too_many_arguments)]
pub(crate) fn classify_pub(
    embedder: &crate::commands::status::ModelHealth,
    reranker: &crate::commands::status::ModelHealth,
    summariser: &crate::commands::status::ModelHealth,
    index: &crate::commands::status::IndexHealth,
    drift: &DriftStatus,
    catalogs: &[CatalogCacheHealth],
    binding: Option<&ProjectBindingState>,
    harness_rules: &[HarnessSubsystemReport],
    harness_mcp: &[HarnessSubsystemReport],
) -> DoctorClassification {
    classify(
        embedder,
        reranker,
        summariser,
        index,
        drift,
        catalogs,
        binding,
        harness_rules,
        harness_mcp,
    )
}

/// Same `pub(crate)` re-export for `build_suggested_fixes`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_suggested_fixes_pub(
    embedder: &crate::commands::status::ModelHealth,
    reranker: &crate::commands::status::ModelHealth,
    summariser: &crate::commands::status::ModelHealth,
    index: &crate::commands::status::IndexHealth,
    drift: &DriftStatus,
    catalogs: &[CatalogCacheHealth],
    binding: Option<&ProjectBindingState>,
    harness_rules: &[HarnessSubsystemReport],
    harness_mcp: &[HarnessSubsystemReport],
) -> Vec<SuggestedFix> {
    build_suggested_fixes(
        embedder,
        reranker,
        summariser,
        index,
        drift,
        catalogs,
        binding,
        harness_rules,
        harness_mcp,
    )
}

/// Per-classification rules from `contracts/doctor.md` /
/// `contracts/doctor-extensions-p4.md` and data-model §15 / FR-561.
///
/// Unhealthy:
/// - Embedder missing/corrupt
/// - Index integrity failure
/// - Embedder drift (stored vectors invalidated)
/// - Schema too new (folds into embedder/index failure paths)
/// - Summariser missing/corrupt (US5.a — summarisation is the new pillar
///   of the rules-file pipeline; failure surfaces RULES.md regressions)
/// - Binding broken (project marker names a workspace that doesn't exist
///   in the central registry; ambiguity requires developer choice)
///
/// Degraded:
/// - Reranker missing/corrupt or reranker drift
/// - Any catalog cache broken (Missing / NotARepo / ManifestInvalid)
/// - Summariser drift (cached summaries stale)
/// - BindingRulesCopy Missing or Drift
/// - Any `harness_rules` entry with `Drift` or `Broken`
/// - Any `harness_mcp` entry with `Drift`, `Broken`, or `UserOwned`
///
/// `NotApplicable` harness subsystems (empty effective list) do NOT
/// affect overall classification (FR-561). `detected_uninstalled_harnesses`
/// is informational only.
#[allow(clippy::too_many_arguments)]
fn classify(
    embedder: &crate::commands::status::ModelHealth,
    reranker: &crate::commands::status::ModelHealth,
    summariser: &crate::commands::status::ModelHealth,
    index: &crate::commands::status::IndexHealth,
    drift: &DriftStatus,
    catalogs: &[CatalogCacheHealth],
    binding: Option<&ProjectBindingState>,
    harness_rules: &[HarnessSubsystemReport],
    harness_mcp: &[HarnessSubsystemReport],
) -> DoctorClassification {
    if embedder.state != "ok" {
        return DoctorClassification::Unhealthy;
    }
    if index.present && !index.integrity_ok {
        return DoctorClassification::Unhealthy;
    }
    if matches!(
        drift,
        DriftStatus::EmbedderNameDrift { .. } | DriftStatus::EmbedderVersionDrift { .. }
    ) {
        return DoctorClassification::Unhealthy;
    }
    if summariser.state != "ok" {
        return DoctorClassification::Unhealthy;
    }
    // Binding broken (marker names a missing workspace) — FR-561 / US5.a.
    if let Some(b) = binding
        && !b.config_well_formed
    {
        return DoctorClassification::Unhealthy;
    }

    if reranker.state != "ok" {
        return DoctorClassification::Degraded;
    }
    if matches!(drift, DriftStatus::RerankerDrift { .. }) {
        return DoctorClassification::Degraded;
    }
    if matches!(drift, DriftStatus::SummariserDrift { .. }) {
        return DoctorClassification::Degraded;
    }
    // Orphan clones are informational per contract — they don't trip
    // Degraded. Only the "broken" cache states do (Missing / NotARepo /
    // ManifestInvalid).
    if catalogs.iter().any(|c| {
        matches!(
            c.state,
            CatalogCacheState::Missing
                | CatalogCacheState::NotARepo
                | CatalogCacheState::ManifestInvalid
        )
    }) {
        return DoctorClassification::Degraded;
    }
    // BindingRulesCopy Missing / Drift → Degraded.
    if let Some(b) = binding
        && !matches!(b.rules_file_drift, RulesCopyState::Match)
    {
        return DoctorClassification::Degraded;
    }
    // Per-harness states. `NotApplicable` is the no-op.
    if harness_rules
        .iter()
        .any(|h| matches!(h.health, SubsystemHealth::Drift | SubsystemHealth::Broken))
    {
        return DoctorClassification::Degraded;
    }
    if harness_mcp.iter().any(|h| {
        matches!(
            h.health,
            SubsystemHealth::Drift | SubsystemHealth::Broken | SubsystemHealth::UserOwned
        )
    }) {
        return DoctorClassification::Degraded;
    }
    DoctorClassification::Ok
}

/// Produce the per-subsystem repair suggestions per data-model.md §6 / §15.
/// Items with `auto_fixable = true` are the classes `--fix` handles
/// automatically; everything else is surfaced for the developer to action.
#[allow(clippy::too_many_arguments)]
fn build_suggested_fixes(
    embedder: &crate::commands::status::ModelHealth,
    reranker: &crate::commands::status::ModelHealth,
    summariser: &crate::commands::status::ModelHealth,
    index: &crate::commands::status::IndexHealth,
    drift: &DriftStatus,
    catalogs: &[CatalogCacheHealth],
    binding: Option<&ProjectBindingState>,
    harness_rules: &[HarnessSubsystemReport],
    harness_mcp: &[HarnessSubsystemReport],
) -> Vec<SuggestedFix> {
    let mut out = Vec::new();
    if let Some(fix) = model_fix(Subsystem::Embedder, embedder) {
        out.push(fix);
    }
    if let Some(fix) = model_fix(Subsystem::Reranker, reranker) {
        out.push(fix);
    }
    if let Some(fix) = model_fix(Subsystem::Summariser, summariser) {
        out.push(fix);
    }
    for c in catalogs {
        if let Some(fix) = catalog_fix(c) {
            out.push(fix);
        }
    }
    // FR-M-DOC-5: when the on-disk schema is older than the compiled
    // SCHEMA_VERSION, emit an auto-fixable "schema" SuggestedFix so
    // `doctor::fixes::repair_schema` actually fires under `--fix`.
    if let Some(v) = index.schema_version
        && v < crate::index::SCHEMA_VERSION
    {
        out.push(SuggestedFix {
            subsystem: Subsystem::Schema,
            diagnosis: format!(
                "schema needs forward migration from v{v} to v{compiled}",
                compiled = crate::index::SCHEMA_VERSION,
            ),
            command: "tome doctor --fix".to_owned(),
            auto_fixable: true,
        });
    }
    match drift {
        DriftStatus::EmbedderNameDrift { stored, configured }
        | DriftStatus::EmbedderVersionDrift { stored, configured } => {
            out.push(SuggestedFix {
                subsystem: Subsystem::Drift,
                diagnosis: format!(
                    "embedder: stored vectors are from `{stored}`, configured is `{configured}`"
                ),
                command: "tome reindex --force".to_owned(),
                auto_fixable: false,
            });
        }
        DriftStatus::RerankerDrift { stored, configured } => {
            out.push(SuggestedFix {
                subsystem: Subsystem::Drift,
                diagnosis: format!("reranker stored as `{stored}`, configured as `{configured}`"),
                command: "tome reindex --force".to_owned(),
                auto_fixable: false,
            });
        }
        DriftStatus::SummariserDrift { stored, configured } => {
            out.push(SuggestedFix {
                subsystem: Subsystem::Drift,
                diagnosis: format!("summariser stored as `{stored}`, configured as `{configured}`"),
                command: "tome doctor --fix  # regenerates cached summaries".to_owned(),
                auto_fixable: false,
            });
        }
        DriftStatus::None => {}
    }
    // Binding: marker malformed or names a workspace that doesn't exist.
    if let Some(b) = binding
        && !b.config_well_formed
    {
        // Two distinct cases share this diagnosis: the marker TOML is
        // malformed (parse failed) OR the marker is well-formed but the
        // workspace it names is missing from the central registry. The
        // remediation in both cases is the same shape — developer
        // chooses to rebind or recreate. `--fix` deliberately does NOT
        // auto-rebind: choosing a target workspace is a destructive
        // product decision the user owns, not a safe repair.
        //
        // Polish C-M12: split into two `SuggestedFix` entries so JSON
        // consumers parsing `command` as one runnable shell line get a
        // single executable string each, rather than a compound prose
        // line embedding two alternatives. The two entries together
        // communicate the same "rebind OR recreate" choice that the
        // prior compound `command` string did.
        let diagnosis = format!(
            "project marker at {} is malformed or names a workspace that does not exist",
            b.project_root.display(),
        );
        out.push(SuggestedFix {
            subsystem: Subsystem::Binding,
            diagnosis: diagnosis.clone(),
            command: "tome workspace use <existing-name>  # rebind to an existing workspace"
                .to_owned(),
            auto_fixable: false,
        });
        out.push(SuggestedFix {
            subsystem: Subsystem::Binding,
            diagnosis,
            command: format!(
                "tome workspace init {}  # or recreate the named workspace",
                b.bound_workspace.as_str(),
            ),
            auto_fixable: false,
        });
    }
    // BindingRulesCopy drift / missing — auto-fixable by re-copy.
    if let Some(b) = binding {
        match b.rules_file_drift {
            RulesCopyState::Match => {}
            RulesCopyState::Missing => {
                out.push(SuggestedFix {
                    subsystem: Subsystem::BindingRulesCopy,
                    diagnosis: format!(
                        "<project>/.tome/RULES.md is missing for project at {}",
                        b.project_root.display(),
                    ),
                    command: "tome doctor --fix".to_owned(),
                    auto_fixable: true,
                });
            }
            RulesCopyState::Drift => {
                out.push(SuggestedFix {
                    subsystem: Subsystem::BindingRulesCopy,
                    diagnosis: format!(
                        "<project>/.tome/RULES.md differs from the workspace's RULES.md ({})",
                        b.project_root.display(),
                    ),
                    command: "tome doctor --fix".to_owned(),
                    auto_fixable: true,
                });
            }
            // R-M5: workspace's canonical RULES.md is absent. Re-copying
            // nothing is a no-op that would re-fire forever, so this
            // suggestion is NOT auto-fixable; the user runs
            // `tome workspace regen-summary` to re-author the source.
            RulesCopyState::SourceMissing => {
                out.push(SuggestedFix {
                    subsystem: Subsystem::BindingRulesCopy,
                    diagnosis: format!(
                        "workspace `{}`'s RULES.md is empty or missing — cannot copy to {}",
                        b.bound_workspace.as_str(),
                        b.project_root.display(),
                    ),
                    command: format!(
                        "tome workspace regen-summary {}  # re-author the source RULES.md first",
                        b.bound_workspace.as_str(),
                    ),
                    auto_fixable: false,
                });
            }
        }
    }
    // Per-harness rules-file integration.
    for hr in harness_rules {
        match hr.health {
            SubsystemHealth::Ok | SubsystemHealth::NotApplicable => {}
            SubsystemHealth::Drift => {
                out.push(SuggestedFix {
                    subsystem: Subsystem::HarnessRules(hr.harness.clone()),
                    diagnosis: format!(
                        "rules-file integration for `{}` differs from Tome's expected body",
                        hr.harness,
                    ),
                    command: "tome sync".to_owned(),
                    auto_fixable: true,
                });
            }
            SubsystemHealth::Broken => {
                out.push(SuggestedFix {
                    subsystem: Subsystem::HarnessRules(hr.harness.clone()),
                    diagnosis: format!(
                        "rules-file integration for `{}` is missing (file absent or block removed)",
                        hr.harness,
                    ),
                    command: "tome sync".to_owned(),
                    auto_fixable: true,
                });
            }
            // UserOwned doesn't apply to rules-file integration —
            // unreachable in practice; defensively skip. Manual/Unverified
            // are MCP-only (Phase 11) — likewise unreachable here; skip.
            SubsystemHealth::UserOwned | SubsystemHealth::Manual | SubsystemHealth::Unverified => {}
        }
    }
    // Per-harness MCP-config integration.
    for hm in harness_mcp {
        match hm.health {
            SubsystemHealth::Ok | SubsystemHealth::NotApplicable => {}
            SubsystemHealth::Drift => {
                out.push(SuggestedFix {
                    subsystem: Subsystem::HarnessMcp(hm.harness.clone()),
                    diagnosis: format!(
                        "MCP config for `{}` carries a stale `--workspace` argument",
                        hm.harness,
                    ),
                    command: "tome sync".to_owned(),
                    auto_fixable: true,
                });
            }
            SubsystemHealth::Broken => {
                out.push(SuggestedFix {
                    subsystem: Subsystem::HarnessMcp(hm.harness.clone()),
                    diagnosis: format!(
                        "MCP config for `{}` is missing the `tome` entry",
                        hm.harness,
                    ),
                    command: "tome sync".to_owned(),
                    auto_fixable: true,
                });
            }
            SubsystemHealth::UserOwned => {
                out.push(SuggestedFix {
                    subsystem: Subsystem::HarnessMcp(hm.harness.clone()),
                    diagnosis: format!(
                        "MCP config for `{}` has a developer-authored `tome` entry; \
                         Tome refuses to overwrite without explicit force",
                        hm.harness,
                    ),
                    // `tome sync` deliberately omits a `--force` flag, so
                    // the override path is the doctor-scoped one: `tome
                    // doctor --fix --force` runs an end-to-end repair pass
                    // (including the clash-overriding harness reconcile) in
                    // one invocation.
                    command: "tome doctor --fix --force".to_owned(),
                    auto_fixable: false,
                });
            }
            // Phase 11 / US5: Manual (jetbrains-ai, no file written) and
            // Unverified (pi, adapter-dependent) are informational, NOT
            // failures — no suggested fix; the recovery artifact is
            // `tome harness info <name>` / `tome harness use <name>`.
            SubsystemHealth::Manual | SubsystemHealth::Unverified => {}
        }
    }
    out
}

fn model_fix(
    subsystem: Subsystem,
    h: &crate::commands::status::ModelHealth,
) -> Option<SuggestedFix> {
    match h.state.as_str() {
        "missing" => Some(SuggestedFix {
            subsystem,
            diagnosis: format!("model `{}` is not installed", h.name),
            command: "tome models download".to_owned(),
            auto_fixable: true,
        }),
        "corrupt" => Some(SuggestedFix {
            subsystem,
            diagnosis: format!(
                "model `{}` is corrupt (files missing or wrong size)",
                h.name
            ),
            command: "tome models download --force".to_owned(),
            auto_fixable: true,
        }),
        "checksum_mismatched" => Some(SuggestedFix {
            subsystem,
            diagnosis: format!("model `{}` SHA-256 mismatch", h.name),
            command: "tome models download --force".to_owned(),
            auto_fixable: true,
        }),
        _ => None,
    }
}

fn catalog_fix(c: &CatalogCacheHealth) -> Option<SuggestedFix> {
    let subsystem = Subsystem::Catalog(c.name.clone());
    match c.state {
        CatalogCacheState::Missing => Some(SuggestedFix {
            subsystem,
            diagnosis: "cache directory not on disk".to_owned(),
            command: format!("tome catalog update {}", c.name),
            auto_fixable: true,
        }),
        CatalogCacheState::NotARepo => Some(SuggestedFix {
            subsystem,
            diagnosis: "cache directory is not a git repo".to_owned(),
            command: format!("tome catalog update {}", c.name),
            auto_fixable: true,
        }),
        CatalogCacheState::ManifestInvalid => Some(SuggestedFix {
            subsystem,
            diagnosis: "catalog manifest is missing or invalid".to_owned(),
            command: format!("tome catalog show {}", c.name),
            auto_fixable: false,
        }),
        CatalogCacheState::Orphan => Some(SuggestedFix {
            subsystem,
            diagnosis: format!(
                "cache directory at {} is not referenced by any registered catalog",
                c.cache_path.display()
            ),
            command: format!("rm -rf {}", c.cache_path.display()),
            auto_fixable: false,
        }),
        CatalogCacheState::Ok => None,
    }
}

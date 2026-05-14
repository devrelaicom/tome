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

pub mod checks;
pub mod harness_detect;
pub mod report;

use crate::commands::plugin::{embedder_entry, reranker_entry};
use crate::commands::status::{check_drift, check_index, check_model};
use crate::commands::workspace::info::assemble as assemble_workspace_info;
use crate::error::TomeError;
use crate::index::meta::DriftStatus;
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

pub use report::{
    CatalogCacheHealth, CatalogCacheState, DoctorClassification, DoctorReport, HarnessPresence,
    SuggestedFix,
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
    home: &std::path::Path,
    verify: bool,
) -> Result<DoctorReport, TomeError> {
    let tome_version = env!("CARGO_PKG_VERSION").to_owned();

    let workspace = assemble_workspace_info(scope, paths)?;

    let embedder_e = embedder_entry();
    let reranker_e = reranker_entry();
    let embedder = check_model(paths, embedder_e, verify)?;
    let reranker = check_model(paths, reranker_e, verify)?;

    let index = check_index(paths, &scope.scope)?;
    let drift = check_drift(paths, &scope.scope, embedder_e, reranker_e)?;
    let catalogs = checks::check_catalogs(paths, &scope.scope)?;
    let harnesses = harness_detect::probe(home);

    let suggested_fixes = build_suggested_fixes(&embedder, &reranker, &index, &drift, &catalogs);
    let overall = classify(&embedder, &reranker, &index, &drift, &catalogs);

    Ok(DoctorReport {
        tome_version,
        workspace,
        embedder,
        reranker,
        index,
        drift,
        catalogs,
        harnesses,
        overall,
        suggested_fixes,
    })
}

/// Per-classification rules from `contracts/doctor.md` and
/// data-model.md §5:
/// - Unhealthy: embedder missing/corrupt, index integrity fail, embedder
///   drift, schema too new (surfaces as embedder/index failure here).
/// - Degraded: reranker missing/corrupt, reranker drift, any catalog
///   cache broken (Missing / NotARepo / ManifestInvalid).
/// - Ok otherwise.
fn classify(
    embedder: &crate::commands::status::ModelHealth,
    reranker: &crate::commands::status::ModelHealth,
    index: &crate::commands::status::IndexHealth,
    drift: &DriftStatus,
    catalogs: &[CatalogCacheHealth],
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
    if reranker.state != "ok" {
        return DoctorClassification::Degraded;
    }
    if matches!(drift, DriftStatus::RerankerDrift { .. }) {
        return DoctorClassification::Degraded;
    }
    if catalogs
        .iter()
        .any(|c| !matches!(c.state, CatalogCacheState::Ok))
    {
        return DoctorClassification::Degraded;
    }
    DoctorClassification::Ok
}

/// Produce the per-subsystem repair suggestions per `data-model.md §6`.
/// Items with `auto_fixable = true` are the three classes `--fix`
/// handles (re-download, re-clone, forward-migrate); everything else is
/// surfaced for the developer to action.
fn build_suggested_fixes(
    embedder: &crate::commands::status::ModelHealth,
    reranker: &crate::commands::status::ModelHealth,
    _index: &crate::commands::status::IndexHealth,
    drift: &DriftStatus,
    catalogs: &[CatalogCacheHealth],
) -> Vec<SuggestedFix> {
    let mut out = Vec::new();
    if let Some(fix) = model_fix("embedder", embedder) {
        out.push(fix);
    }
    if let Some(fix) = model_fix("reranker", reranker) {
        out.push(fix);
    }
    for c in catalogs {
        if let Some(fix) = catalog_fix(c) {
            out.push(fix);
        }
    }
    match drift {
        DriftStatus::EmbedderNameDrift { stored, configured }
        | DriftStatus::EmbedderVersionDrift { stored, configured } => {
            out.push(SuggestedFix {
                subsystem: "embedder_drift".to_owned(),
                diagnosis: format!(
                    "stored vectors are from `{stored}`, configured is `{configured}`"
                ),
                command: "tome reindex --force".to_owned(),
                auto_fixable: false,
            });
        }
        DriftStatus::RerankerDrift { stored, configured } => {
            out.push(SuggestedFix {
                subsystem: "reranker_drift".to_owned(),
                diagnosis: format!("reranker stored as `{stored}`, configured as `{configured}`"),
                command: "tome reindex --force".to_owned(),
                auto_fixable: false,
            });
        }
        DriftStatus::None => {}
    }
    out
}

fn model_fix(name: &str, h: &crate::commands::status::ModelHealth) -> Option<SuggestedFix> {
    match h.state.as_str() {
        "missing" => Some(SuggestedFix {
            subsystem: name.to_owned(),
            diagnosis: format!("model `{}` is not installed", h.name),
            command: "tome models download".to_owned(),
            auto_fixable: true,
        }),
        "corrupt" => Some(SuggestedFix {
            subsystem: name.to_owned(),
            diagnosis: format!(
                "model `{}` is corrupt (files missing or wrong size)",
                h.name
            ),
            command: "tome models download --force".to_owned(),
            auto_fixable: true,
        }),
        "checksum_mismatched" => Some(SuggestedFix {
            subsystem: name.to_owned(),
            diagnosis: format!("model `{}` SHA-256 mismatch", h.name),
            command: "tome models download --force".to_owned(),
            auto_fixable: true,
        }),
        _ => None,
    }
}

fn catalog_fix(c: &CatalogCacheHealth) -> Option<SuggestedFix> {
    let subsystem = format!("catalog:{}", c.name);
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
        CatalogCacheState::Ok => None,
    }
}

//! Serialisable types for `tome doctor`'s report. Data-model §5 / §6.
//!
//! Emit-only — these types are never deserialised, so no
//! `#[serde(deny_unknown_fields)]`. The wire JSON shape is contract
//! `contracts/doctor.md`; an integration test pins byte-stability.

use std::path::PathBuf;

use serde::Serialize;

use crate::commands::status::{IndexHealth, ModelHealth};
use crate::index::meta::DriftStatus;
use crate::workspace::WorkspaceInfo;

/// Three-state overall classification used by `tome doctor`. Matches the
/// shape of `OverallHealth` from Phase 2 status but lives here so the
/// doctor report's `overall` field is wire-distinct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DoctorClassification {
    Ok,
    Degraded,
    Unhealthy,
}

/// Per-catalog on-disk cache classification. The `state` field uses
/// snake_case so the JSON wire matches the contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogCacheHealth {
    pub name: String,
    pub url: String,
    pub cache_path: PathBuf,
    pub state: CatalogCacheState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogCacheState {
    /// Directory exists, is a git repo, and the catalog manifest parses.
    Ok,
    /// Cache directory not on disk.
    Missing,
    /// Cache directory exists but lacks `.git/`.
    NotARepo,
    /// Cache + `.git/` present but `tome-catalog.toml` is missing or
    /// unparsable.
    ManifestInvalid,
    /// Cache directory exists, is a valid catalog clone, but no
    /// `config.toml` in the resolved scope references its URL. Created
    /// when a `tome catalog remove` left a sibling-scope reference
    /// behind, or when a registry edit dropped the entry without
    /// removing the clone. The orphan record is informational —
    /// `auto_fixable` is `false`; the user removes it by hand once
    /// they've verified nothing else needs the clone. Contract
    /// `catalog-extensions-p3.md` §"Doctor reporting" bullet 4.
    Orphan,
}

impl CatalogCacheState {
    pub fn as_str(self) -> &'static str {
        match self {
            CatalogCacheState::Ok => "ok",
            CatalogCacheState::Missing => "missing",
            CatalogCacheState::NotARepo => "not_a_repo",
            CatalogCacheState::ManifestInvalid => "manifest_invalid",
            CatalogCacheState::Orphan => "orphan",
        }
    }
}

/// Workspace-registry status block. Contract
/// `catalog-extensions-p3.md` §"Doctor reporting" calls for one line
/// summarising the opt-in registry file. `present = false` is the
/// default fresh-install state; `present = true` means the file is
/// opt-in-touched and `tracked` is the count of registered workspaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceRegistryStatus {
    pub present: bool,
    pub tracked: u32,
}

/// One probed agentic-coding harness. The well-known harness names are a
/// fixed list (research §R-7); the value of `present` is what doctor
/// actually checks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HarnessPresence {
    pub name: String,
    pub path: PathBuf,
    pub present: bool,
}

/// A user-actionable repair suggestion. `auto_fixable = true` items are
/// the three classes `--fix` handles automatically; everything else is
/// surfaced as a copy-pasteable command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SuggestedFix {
    pub subsystem: String,
    pub diagnosis: String,
    pub command: String,
    pub auto_fixable: bool,
}

/// Full doctor report. Field order matches `contracts/doctor.md`
/// §"Output (`--json`)" so the rendered JSON is deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub tome_version: String,
    pub workspace: WorkspaceInfo,
    pub embedder: ModelHealth,
    pub reranker: ModelHealth,
    pub index: IndexHealth,
    pub drift: DriftStatus,
    pub catalogs: Vec<CatalogCacheHealth>,
    /// FR-M-DOC-2 / `catalog-extensions-p3.md` §"Doctor reporting":
    /// status of the opt-in workspace registry file (presence + count).
    pub workspace_registry: WorkspaceRegistryStatus,
    pub harnesses: Vec<HarnessPresence>,
    pub overall: DoctorClassification,
    pub suggested_fixes: Vec<SuggestedFix>,
}

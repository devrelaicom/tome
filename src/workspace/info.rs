//! `WorkspaceInfo` — the serialisable record describing the resolved scope
//! and its on-disk index state. Consumed by `tome workspace info` (the
//! direct surface) and by `tome doctor` (which embeds the same record
//! verbatim under its `workspace` field, per data-model §5).
//!
//! Emit-only — these types never participate in deserialisation, so no
//! `#[serde(deny_unknown_fields)]`. The wire JSON shape is contract; the
//! `tests/workspace_info.rs` `--json` byte-stability test pins it.

use std::path::PathBuf;

use serde::Serialize;

use crate::workspace::scope::ScopeSource;

/// Two-state scope kind as serialised in `--json` output ("global" /
/// "workspace"). Distinct from the in-process `Scope` enum which carries
/// the workspace path inside the variant; the JSON record splits the kind
/// from the path into separate fields per `contracts/workspace-info.md`.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ScopeKind {
    Global,
    Workspace,
}

/// Model identity (name + version) as it appears in the JSON record's
/// `embedder` field. Named structurally rather than re-using the
/// `commands::status::ModelHealth` so the wire shape is exactly the two
/// fields the contract pins — no health-state leakage.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ModelIdentity {
    pub name: String,
    pub version: String,
}

/// The wire record for `tome workspace info`. Field order matches
/// `contracts/workspace-info.md` §"Output (`--json`)" so the human-readable
/// `serde_json::to_string` rendering is deterministic.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorkspaceInfo {
    pub scope: ScopeKind,
    pub path: Option<PathBuf>,
    pub source: ScopeSource,
    pub catalogs: u32,
    pub plugins_total: u32,
    pub plugins_enabled: u32,
    pub skills_indexed: u32,
    pub schema_version: Option<u32>,
    pub embedder: Option<ModelIdentity>,
}

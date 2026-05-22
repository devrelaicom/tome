//! Active workspace context for a single Tome invocation.
//!
//! Phase 4 / F10 collapses the Phase 3 `Scope::Global | Scope::Workspace(PathBuf)`
//! enum into a [`Scope`] tuple struct wrapping a validated
//! [`WorkspaceName`]. The on-disk path of the bound project (when any)
//! is carried separately on [`ResolvedScope::project_root`] — workspace
//! identity is the name; the path is provenance.

use std::path::PathBuf;

use serde::Serialize;

use crate::workspace::WorkspaceName;

/// The active workspace for this invocation. Just a validated name —
/// the on-disk path (if any) lives on [`ResolvedScope::project_root`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scope(pub WorkspaceName);

impl Scope {
    /// Borrow the inner [`WorkspaceName`].
    pub fn name(&self) -> &WorkspaceName {
        &self.0
    }

    /// True iff the active scope is the privileged `global` workspace.
    pub fn is_global(&self) -> bool {
        self.0.is_reserved()
    }
}

/// How the scope was determined. Serialised in snake_case for the
/// `tome workspace info --json` `source` field. The Phase 3 variants
/// `GlobalFlag` and `CwdWalk` are gone — Phase 4 has no `--global`
/// flag, and the cwd walk is now a project-marker walk (modern name).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScopeSource {
    /// `--workspace <name>` on the CLI.
    Flag,
    /// `TOME_WORKSPACE` env var.
    Env,
    /// `.tome/config.toml` project marker discovered by walking parents
    /// from the current working directory.
    ProjectMarker,
    /// No input found; defaulted to `global`.
    GlobalFallback,
}

/// The resolver's output: scope + provenance + (when `source == ProjectMarker`)
/// the project root directory that triggered the binding.
#[derive(Debug, Clone)]
pub struct ResolvedScope {
    pub scope: Scope,
    pub source: ScopeSource,
    /// The directory whose `.tome/config.toml` named the bound workspace
    /// (when `source == ProjectMarker`). `None` otherwise.
    pub project_root: Option<PathBuf>,
}

impl ResolvedScope {
    /// The privileged-default resolution: no flag, no env, no marker.
    pub fn global_fallback() -> Self {
        Self {
            scope: Scope(WorkspaceName::global()),
            source: ScopeSource::GlobalFallback,
            project_root: None,
        }
    }

    /// True iff the resolved scope is a named (non-`global`) workspace.
    /// Replaces the Phase 3 `is_workspace()` predicate.
    pub fn is_named_workspace(&self) -> bool {
        !self.scope.is_global()
    }
}

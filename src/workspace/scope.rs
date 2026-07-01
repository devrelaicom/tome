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
    /// `[workspace] default` in `~/.tome/config.toml`.
    Config,
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
    /// Issue #302: the directory of a project marker that a `[workspace] default`
    /// resolution *shadowed*. Populated ONLY when `source == Config` AND a
    /// `.tome/config.toml` project marker actually exists in the CWD ancestry —
    /// i.e. the config default won resolution even though the user has a
    /// per-project binding here. `None` in every other case (including a
    /// `Config` win with no marker present).
    ///
    /// This is DETECTION ONLY: the resolved scope and `project_root: None` are
    /// unchanged by its presence. The CLI foreground boundary reads it to print
    /// a one-line stderr notice; the MCP island and tests never emit from it.
    pub overridden_project_marker: Option<PathBuf>,
}

impl ResolvedScope {
    /// The privileged-default resolution: no flag, no env, no marker.
    pub fn global_fallback() -> Self {
        Self {
            scope: Scope(WorkspaceName::global()),
            source: ScopeSource::GlobalFallback,
            project_root: None,
            overridden_project_marker: None,
        }
    }

    /// True iff the resolved scope is a named (non-`global`) workspace.
    /// Replaces the Phase 3 `is_workspace()` predicate.
    pub fn is_named_workspace(&self) -> bool {
        !self.scope.is_global()
    }
}

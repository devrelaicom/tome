//! `Scope` is the result of resolving the active workspace context for a
//! single Tome invocation. The resolution algorithm itself lives in
//! `workspace::resolution` (slice F3); this file just defines the data
//! shapes so the rest of the codebase can refer to them without depending
//! on the resolver.
//!
//! `Workspace(path)` always carries an **absolute, canonicalised** path
//! pointing at the directory that contains `.tome/`. The path's
//! absoluteness is a load-bearing invariant — every `Paths` accessor
//! method assumes it can join straight onto the path without
//! re-canonicalising.

use std::path::PathBuf;

use serde::Serialize;

/// Which install-scope a command operates against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// The global install — config under `${XDG_CONFIG_HOME}/tome/`,
    /// data under `${XDG_DATA_HOME}/tome/`. Phase 1 + Phase 2 default.
    Global,
    /// A workspace rooted at `path` (an absolute, canonicalised path to
    /// the directory **containing** `.tome/`).
    Workspace(PathBuf),
}

/// How the scope was determined for this invocation. Carried in
/// `ResolvedScope` so commands and tests can assert provenance (e.g.
/// `tome workspace info` reports the source verbatim).
///
/// The enum is `Copy` because every variant is a unit — `match` arms
/// elsewhere benefit from cheap pattern duplication.
///
/// Serialised in snake_case to match the `source` field values pinned by
/// `contracts/workspace-info.md`:
/// `"flag" | "global_flag" | "env" | "cwd_walk" | "global_fallback"`.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScopeSource {
    /// `--workspace <path>` on the CLI.
    Flag,
    /// `--global` on the CLI.
    GlobalFlag,
    /// `TOME_WORKSPACE` env var.
    Env,
    /// Found by walking parents from CWD until `.tome/` was hit.
    CwdWalk,
    /// No workspace found anywhere; defaulted to global.
    GlobalFallback,
}

/// The resolver's output: the scope plus the input that picked it.
///
/// Constructed by `workspace::resolution::resolve` (slice F3) and
/// threaded into every command's `run()` from there.
#[derive(Debug, Clone)]
pub struct ResolvedScope {
    pub scope: Scope,
    pub source: ScopeSource,
}

impl ResolvedScope {
    /// Sugar for the global-fallback case, used by tests and by the
    /// pre-F3 default while the resolver isn't wired yet.
    pub fn global_fallback() -> Self {
        Self {
            scope: Scope::Global,
            source: ScopeSource::GlobalFallback,
        }
    }

    /// Convenience: is this resolution pointing at a workspace?
    pub fn is_workspace(&self) -> bool {
        matches!(self.scope, Scope::Workspace(_))
    }
}

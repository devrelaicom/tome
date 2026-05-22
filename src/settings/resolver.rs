//! Composition resolver — turns layered settings declarations into a
//! flat effective harness list.
//!
//! ## Algorithm (mirrors `contracts/settings-composition.md`)
//!
//! 1. **Priority walk** (FR-441): the FIRST scope whose `harnesses` key
//!    is `Some(...)` (regardless of whether the list is empty)
//!    terminates the priority walk. Other scopes are consulted ONLY via
//!    composition references inside the first declarer's list.
//! 2. **Recursive descent**: each entry in the declared list parses to
//!    a [`CompositionRef`]. Bare names are added to the inclusion /
//!    exclusion set; bracketed references recurse into the referenced
//!    scope's **directly-declared** list (FR-449 — composition refs
//!    resolve to as-written lists, NOT computed effective lists).
//! 3. **Cycle detection**: a DFS visited set of `(ScopeKind, key)`
//!    pairs. On re-visit the resolver returns
//!    [`CompositionErrorKind::Cycle`] with the path.
//! 4. **Final subtraction**: exclusions are removed from the inclusion
//!    set; the result is ordered by first-included-from chain.
//!
//! F8 ships the resolver skeleton against a [`ScopeProvider`] trait so
//! tests can hand-roll fixtures without on-disk files. US3.a wires the
//! production [`ScopeProvider`] impl against the central SQLite
//! database (which carries the workspace-name → directly-declared-list
//! mapping under the workspaces table).

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::error::CompositionErrorKind;
use crate::workspace::WorkspaceName;

use super::composition::CompositionRef;
use super::{GlobalSettings, ProjectMarkerConfig, WorkspaceSettings};

/// Composition scope classification.
///
/// Distinct from [`crate::workspace::ScopeKind`] (which is the
/// `tome workspace info` two-state global/workspace classifier) — the
/// settings layer is a strictly richer concept (a project scope exists
/// inside the workspace it binds to).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    Project,
    Workspace,
    Global,
}

/// One harness in the effective list, tagged with the scope chain that
/// contributed it. Multiple contributions collapse to the first chain
/// observed during DFS (so the chain narrates origin, not history).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveHarness {
    pub name: String,
    pub source_chain: Vec<ScopeKind>,
}

/// The resolved effective harness list.
///
/// `harnesses` is ordered by first-included-from chain. `excluded`
/// surfaces the names subtracted by `!`-prefixes, for
/// `tome harness list` to report transparently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveHarnessList {
    pub harnesses: Vec<EffectiveHarness>,
    pub excluded: Vec<String>,
}

/// Trait abstraction over the workspace registry, allowing the resolver
/// to be exercised against in-memory fixtures during F8 and against the
/// central SQLite database in US3.a.
///
/// `directly_declared_harnesses` returns the workspace's `harnesses`
/// field **as written** in its settings.toml — `None` if the workspace
/// exists but its file omits the key, `Err(UnknownWorkspace)` if the
/// workspace is not registered.
pub trait ScopeProvider {
    fn directly_declared_harnesses(
        &self,
        name: &WorkspaceName,
    ) -> Result<Option<Vec<String>>, CompositionErrorKind>;
}

/// In-memory [`ScopeProvider`] for tests. Construct via
/// [`StubScope::new`] and register workspaces with
/// [`StubScope::with_workspace`]. Workspaces absent from the map are
/// reported as [`CompositionErrorKind::UnknownWorkspace`].
#[derive(Debug, Default, Clone)]
pub struct StubScope {
    /// `None` = workspace exists but didn't declare `harnesses`;
    /// `Some(vec)` = workspace exists and declared the list verbatim.
    workspaces: HashMap<String, Option<Vec<String>>>,
}

impl StubScope {
    /// Empty registry — every `[workspaces.<name>]` reference fails.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a workspace's directly-declared harness list.
    /// Pass `None` to register the workspace as existing-but-undeclared.
    #[must_use]
    pub fn with_workspace(mut self, name: &str, harnesses: Option<Vec<String>>) -> Self {
        self.workspaces.insert(name.to_owned(), harnesses);
        self
    }
}

impl ScopeProvider for StubScope {
    fn directly_declared_harnesses(
        &self,
        name: &WorkspaceName,
    ) -> Result<Option<Vec<String>>, CompositionErrorKind> {
        self.workspaces
            .get(name.as_str())
            .cloned()
            .ok_or_else(|| CompositionErrorKind::UnknownWorkspace(name.as_str().to_owned()))
    }
}

/// Resolve the effective harness list per `contracts/settings-composition.md`.
///
/// `central_db` is the workspace registry (any [`ScopeProvider`]). The
/// production caller in US3.a passes a wrapper around the central
/// SQLite handle; F8 tests pass [`StubScope`].
///
/// Returns [`EffectiveHarnessList`] with `harnesses` empty when no
/// scope declares `harnesses`; this is not an error.
pub fn resolve_effective_list<P: ScopeProvider>(
    project_marker: Option<&ProjectMarkerConfig>,
    bound_workspace: Option<&WorkspaceSettings>,
    global_settings: &GlobalSettings,
    central_db: &P,
) -> Result<EffectiveHarnessList, CompositionErrorKind> {
    // Priority walk: stop at the FIRST scope that declares `harnesses`.
    let (start_scope, start_list) = if let Some(pm) = project_marker
        && let Some(list) = pm.harnesses.as_ref()
    {
        (ScopeKind::Project, list.clone())
    } else if let Some(ws) = bound_workspace
        && let Some(list) = ws.harnesses.as_ref()
    {
        (ScopeKind::Workspace, list.clone())
    } else if let Some(list) = global_settings.harnesses.as_ref() {
        (ScopeKind::Global, list.clone())
    } else {
        return Ok(EffectiveHarnessList {
            harnesses: Vec::new(),
            excluded: Vec::new(),
        });
    };

    let start_key = scope_key(start_scope, project_marker, bound_workspace);

    let mut state = ResolveState::default();
    state.visit(start_scope, &start_key)?;
    resolve_list(
        &start_list,
        start_scope,
        vec![start_scope],
        project_marker,
        bound_workspace,
        global_settings,
        central_db,
        &mut state,
    )?;

    let excluded: Vec<String> = state.exclusions.iter().cloned().collect();
    let mut effective: Vec<EffectiveHarness> = state
        .inclusions
        .into_iter()
        .filter(|h| !state.exclusions.contains(&h.name))
        .collect();

    // Preserve insertion-order in `effective` (Vec already does); make
    // `excluded` deterministic for the wire report. Order within
    // exclusions doesn't affect semantics — sort alphabetically.
    let mut excluded_sorted = excluded;
    excluded_sorted.sort();
    excluded_sorted.dedup();

    // Dedupe inclusions by name preserving first occurrence and its
    // source_chain.
    let mut seen = HashSet::new();
    effective.retain(|h| seen.insert(h.name.clone()));

    Ok(EffectiveHarnessList {
        harnesses: effective,
        excluded: excluded_sorted,
    })
}

/// Compute the `(ScopeKind, key)` cycle-detection key for the entry
/// point of the priority walk. Project keys off the bound workspace
/// name (the project marker's `workspace` field) because the project
/// itself is path-shaped and a cycle ultimately bottoms out at the
/// workspace it's bound to.
fn scope_key(
    scope: ScopeKind,
    project_marker: Option<&ProjectMarkerConfig>,
    bound_workspace: Option<&WorkspaceSettings>,
) -> String {
    match scope {
        ScopeKind::Project => project_marker
            .map(|pm| pm.workspace.as_str().to_owned())
            .unwrap_or_default(),
        ScopeKind::Workspace => bound_workspace
            .map(|ws| ws.name.as_str().to_owned())
            .unwrap_or_default(),
        ScopeKind::Global => "<global>".to_owned(),
    }
}

/// Mutable DFS state threaded through recursive scope resolutions.
#[derive(Debug, Default)]
struct ResolveState {
    visited: HashSet<(ScopeKind, String)>,
    inclusions: Vec<EffectiveHarness>,
    exclusions: HashSet<String>,
}

impl ResolveState {
    fn visit(&mut self, scope: ScopeKind, key: &str) -> Result<(), CompositionErrorKind> {
        let pair = (scope, key.to_owned());
        if !self.visited.insert(pair) {
            // Re-visit: cycle detected. Render the path as the
            // accumulated visited list in deterministic order.
            let mut path: Vec<String> = self
                .visited
                .iter()
                .map(|(_, k)| k.clone())
                .collect::<Vec<_>>();
            path.sort();
            path.push(key.to_owned());
            return Err(CompositionErrorKind::Cycle { path });
        }
        Ok(())
    }
}

/// Walk one scope's declared list, dispatching each entry to
/// inclusions / exclusions / recursive scope resolution.
#[allow(clippy::too_many_arguments)]
fn resolve_list<P: ScopeProvider>(
    list: &[String],
    current_scope: ScopeKind,
    source_chain: Vec<ScopeKind>,
    project_marker: Option<&ProjectMarkerConfig>,
    bound_workspace: Option<&WorkspaceSettings>,
    global_settings: &GlobalSettings,
    central_db: &P,
    state: &mut ResolveState,
) -> Result<(), CompositionErrorKind> {
    for raw in list {
        match CompositionRef::parse(raw)? {
            CompositionRef::Include(name) => {
                state.inclusions.push(EffectiveHarness {
                    name,
                    source_chain: source_chain.clone(),
                });
            }
            CompositionRef::Exclude(name) => {
                state.exclusions.insert(name);
            }
            CompositionRef::CurrentWorkspace => {
                // `[workspace]` is only valid in project scope (FR-449).
                if current_scope != ScopeKind::Project {
                    return Err(CompositionErrorKind::WorkspaceRefOutsideProject {
                        // Map our richer settings::ScopeKind onto
                        // the closed-set `workspace::ScopeKind` the
                        // error variant carries (Workspace or
                        // Global only — Project is excluded by the
                        // gate above).
                        found_in: workspace_scope_for(current_scope),
                    });
                }
                let Some(pm) = project_marker else {
                    // A project marker with `[workspace]` MUST have a
                    // workspace pointer — it's the marker's only
                    // required field. Unreachable in well-formed code.
                    return Err(CompositionErrorKind::WorkspaceRefOutsideProject {
                        found_in: workspace_scope_for(current_scope),
                    });
                };
                let sub_name = pm.workspace.as_str().to_owned();
                let declared = central_db.directly_declared_harnesses(&pm.workspace)?;
                let mut sub_chain = source_chain.clone();
                sub_chain.push(ScopeKind::Workspace);
                resolve_named_workspace(
                    &sub_name,
                    declared,
                    sub_chain,
                    project_marker,
                    bound_workspace,
                    global_settings,
                    central_db,
                    state,
                )?;
            }
            CompositionRef::NamedWorkspace(name) => {
                let declared = central_db.directly_declared_harnesses(&name)?;
                let mut sub_chain = source_chain.clone();
                sub_chain.push(ScopeKind::Workspace);
                resolve_named_workspace(
                    name.as_str(),
                    declared,
                    sub_chain,
                    project_marker,
                    bound_workspace,
                    global_settings,
                    central_db,
                    state,
                )?;
            }
            CompositionRef::Global => {
                state.visit(ScopeKind::Global, "<global>")?;
                if let Some(global_list) = global_settings.harnesses.as_ref() {
                    let mut sub_chain = source_chain.clone();
                    sub_chain.push(ScopeKind::Global);
                    resolve_list(
                        global_list,
                        ScopeKind::Global,
                        sub_chain,
                        project_marker,
                        bound_workspace,
                        global_settings,
                        central_db,
                        state,
                    )?;
                }
                // `[global]` is terminal — no further recursion shape.
            }
        }
    }
    Ok(())
}

/// Recurse into a workspace's directly-declared list (FR-449). The
/// list comes from the central registry (in F8: [`StubScope`]; in US3:
/// the SQLite-backed `ScopeProvider`).
#[allow(clippy::too_many_arguments)]
fn resolve_named_workspace<P: ScopeProvider>(
    name: &str,
    declared: Option<Vec<String>>,
    source_chain: Vec<ScopeKind>,
    project_marker: Option<&ProjectMarkerConfig>,
    bound_workspace: Option<&WorkspaceSettings>,
    global_settings: &GlobalSettings,
    central_db: &P,
    state: &mut ResolveState,
) -> Result<(), CompositionErrorKind> {
    state.visit(ScopeKind::Workspace, name)?;
    let Some(list) = declared else {
        // FR-449: workspace exists but didn't declare `harnesses`.
        // Resolve to the empty list — do NOT fall through to global.
        return Ok(());
    };
    resolve_list(
        &list,
        ScopeKind::Workspace,
        source_chain,
        project_marker,
        bound_workspace,
        global_settings,
        central_db,
        state,
    )
}

/// Map [`ScopeKind`] (Project/Workspace/Global) → the closed-set
/// `workspace::ScopeKind` (Workspace/Global) that the
/// `WorkspaceRefOutsideProject` error variant carries. `Project` is
/// impossible at this call site (the gate above filters it).
fn workspace_scope_for(scope: ScopeKind) -> crate::workspace::ScopeKind {
    match scope {
        // Per the type system: this branch is unreachable in practice
        // because callers only invoke `workspace_scope_for` after
        // gating out `Project`. Defensive fallback maps to `Workspace`
        // (the closer-fit variant) rather than panicking.
        ScopeKind::Project | ScopeKind::Workspace => crate::workspace::ScopeKind::Workspace,
        ScopeKind::Global => crate::workspace::ScopeKind::Global,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_global() -> GlobalSettings {
        GlobalSettings::default()
    }

    #[test]
    fn no_declarations_returns_empty_effective_list() {
        let stub = StubScope::new();
        let result = resolve_effective_list(None, None, &empty_global(), &stub).unwrap();
        assert!(result.harnesses.is_empty());
        assert!(result.excluded.is_empty());
    }

    #[test]
    fn global_only_declaration_includes_listed_harnesses() {
        let stub = StubScope::new();
        let global = GlobalSettings {
            harnesses: Some(vec!["claude-code".to_owned(), "codex".to_owned()]),
        };
        let result = resolve_effective_list(None, None, &global, &stub).unwrap();
        assert_eq!(result.harnesses.len(), 2);
        assert_eq!(result.harnesses[0].name, "claude-code");
        assert_eq!(result.harnesses[1].name, "codex");
        assert!(result.excluded.is_empty());
    }

    #[test]
    fn workspace_ref_outside_project_errors() {
        let stub = StubScope::new();
        let ws = WorkspaceSettings {
            name: WorkspaceName::global(),
            summaries: None,
            catalogs: Vec::new(),
            harnesses: Some(vec!["[workspace]".to_owned()]),
        };
        let err = resolve_effective_list(None, Some(&ws), &empty_global(), &stub).expect_err("err");
        assert!(matches!(
            err,
            CompositionErrorKind::WorkspaceRefOutsideProject { .. }
        ));
    }
}

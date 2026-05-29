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
///
/// `source_chain` is a mixed-notation list per
/// `contracts/settings-composition.md` example output. Each step is one
/// of:
///
/// * `"project"`, `"workspace"`, `"global"` — direct declaration in the
///   named scope.
/// * `"[workspaces.<name>]"`, `"[workspace]"`, `"[global]"` — pulled in
///   via that composition reference.
///
/// Example: a harness declared in `global` settings that the project
/// marker pulls in via `[global]` would emit `["project", "[global]"]`.
/// A harness directly declared in the project marker would emit
/// `["project"]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectiveHarness {
    pub name: String,
    pub source_chain: Vec<String>,
}

/// The resolved effective harness list.
///
/// `harnesses` is ordered by first-included-from chain. `excluded`
/// surfaces the names subtracted by `!`-prefixes, for
/// `tome harness list` to report transparently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    state.enter(start_scope, &start_key)?;
    // C-M1: source_chain is a mixed-notation Vec<String>. The first
    // step records the priority-walk scope (`"project"` / `"workspace"`
    // / `"global"`); subsequent steps are appended by composition-ref
    // recursion as bracketed reference strings.
    let walk_result = resolve_list(
        &start_list,
        start_scope,
        vec![scope_label(start_scope)],
        project_marker,
        bound_workspace,
        global_settings,
        central_db,
        &mut state,
    );
    state.leave();
    walk_result?;

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

    // C-M4: per-entry validation now happens inside `resolve_list` for
    // every `CompositionRef::Include` so a typo'd inclusion is reported
    // even if a later exclusion would have cancelled it out. No
    // end-of-resolution check is needed.

    Ok(EffectiveHarnessList {
        harnesses: effective,
        excluded: excluded_sorted,
    })
}

/// Render a [`ScopeKind`] as its first-step source-chain label per
/// `contracts/settings-composition.md` (`"project"` / `"workspace"` /
/// `"global"`). Used to populate [`EffectiveHarness::source_chain`]'s
/// first step — subsequent steps are bracketed reference strings.
fn scope_label(scope: ScopeKind) -> String {
    match scope {
        ScopeKind::Project => "project".to_string(),
        ScopeKind::Workspace => "workspace".to_string(),
        ScopeKind::Global => "global".to_string(),
    }
}

/// Is `name` a harness registered in the effective harness registry?
///
/// Consults the test-injected `HARNESS_MODULES_OVERRIDE` slot via
/// [`crate::harness::with_effective_modules`] so test fixtures that swap
/// in synthetic harnesses validate against their registry rather than the
/// production `SUPPORTED_HARNESSES` constant.
fn is_supported_harness(name: &str) -> bool {
    crate::harness::with_effective_modules(|modules| modules.iter().any(|m| m.name() == name))
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
///
/// Cycle detection uses two parallel structures:
/// * `visited` — O(1) re-visit check on `(ScopeKind, key)`.
/// * `stack` — DFS path in walk order so a detected cycle renders the
///   actual loop chain (FR-445 "naming every scope in the loop chain"),
///   not an alphabetically-sorted set.
#[derive(Debug, Default)]
struct ResolveState {
    visited: HashSet<(ScopeKind, String)>,
    stack: Vec<String>,
    inclusions: Vec<EffectiveHarness>,
    exclusions: HashSet<String>,
}

impl ResolveState {
    /// Enter a new scope in the DFS. Returns `Err(Cycle)` if the
    /// `(scope, key)` pair is already in the visited set; otherwise
    /// pushes the key onto the walk-order stack and returns. Callers
    /// pair this with [`Self::leave`] after the recursive resolution
    /// completes (success or propagated error — leave must run on both
    /// paths to keep the stack consistent for any subsequent siblings).
    ///
    /// Visited entries are intentionally NEVER removed — once a scope's
    /// list has been walked it can't contribute new harnesses, and a
    /// later sibling that names the same scope would silently re-walk
    /// it without that invariant. The stack is the *active* DFS chain
    /// for cycle path reporting; the visited set is the *exhausted* set
    /// of already-walked nodes.
    fn enter(&mut self, scope: ScopeKind, key: &str) -> Result<(), CompositionErrorKind> {
        let pair = (scope, key.to_owned());
        if !self.visited.insert(pair) {
            // Re-visit: cycle detected. Path is the current DFS stack
            // (the chain of scopes that led us back here) plus the
            // re-visited key as the loop-closing entry. This preserves
            // walk order — `["A", "B", "A"]` for an A→B→A cycle.
            let mut path = self.stack.clone();
            path.push(key.to_owned());
            return Err(CompositionErrorKind::Cycle { path });
        }
        self.stack.push(key.to_owned());
        Ok(())
    }

    /// Pop the most recently entered key off the DFS stack.
    fn leave(&mut self) {
        self.stack.pop();
    }
}

/// Walk one scope's declared list, dispatching each entry to
/// inclusions / exclusions / recursive scope resolution.
///
/// `source_chain` is the rendered mixed-notation chain accumulated so
/// far during DFS (see [`EffectiveHarness::source_chain`]). Each
/// recursive scope dispatch appends one element naming the reference
/// that triggered the recursion (`"[workspace]"`, `"[workspaces.<name>]"`,
/// or `"[global]"`).
#[allow(clippy::too_many_arguments)]
fn resolve_list<P: ScopeProvider>(
    list: &[String],
    current_scope: ScopeKind,
    source_chain: Vec<String>,
    project_marker: Option<&ProjectMarkerConfig>,
    bound_workspace: Option<&WorkspaceSettings>,
    global_settings: &GlobalSettings,
    central_db: &P,
    state: &mut ResolveState,
) -> Result<(), CompositionErrorKind> {
    for raw in list {
        match CompositionRef::parse(raw)? {
            CompositionRef::Include(name) => {
                // FR-460 per-entry validation (C-M4 from US3 review):
                // every inclusion must name a harness in the production
                // registry (or a synthetic one installed via
                // `HARNESS_MODULES_OVERRIDE` for tests). Validate at
                // parse time rather than end-of-resolution so
                // `["fake", "!fake"]` fails fast on the inclusion
                // (per-entry cancellation invariant — exclusions cannot
                // mask a typo'd harness name from being reported).
                if !is_supported_harness(&name) {
                    return Err(CompositionErrorKind::HarnessNotSupported(name));
                }
                state.inclusions.push(EffectiveHarness {
                    name,
                    source_chain: source_chain.clone(),
                });
            }
            CompositionRef::Exclude(name) => {
                state.exclusions.insert(name);
            }
            CompositionRef::CurrentWorkspace => {
                // `[workspace]` is only valid in project scope (FR-446 /
                // FR-449). Encountering it inside a workspace's or
                // global's directly-declared list is an error.
                //
                // R-M3 (US3 review): `found_in` must report the scope
                // where the `[workspace]` token actually appeared
                // (`current_scope`), not the parent scope it was
                // discovered through. `workspace_scope_for` maps our
                // richer settings::ScopeKind (Project / Workspace /
                // Global) onto the closed-set workspace::ScopeKind
                // (Workspace / Global) that the error variant carries.
                if current_scope != ScopeKind::Project {
                    return Err(CompositionErrorKind::WorkspaceRefOutsideProject {
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
                // FR-446 (project-without-binding): if the caller did
                // not load a bound workspace, the `[workspace]` token
                // has nothing to resolve against. Refuse rather than
                // silently emit an empty inclusion set — surfacing the
                // shape mismatch is the point of this guard.
                if bound_workspace.is_none() {
                    return Err(CompositionErrorKind::WorkspaceRefOutsideProject {
                        // Closest-fit fallback — `workspace::ScopeKind`
                        // is Workspace/Global only; the error variant
                        // semantically wants "Project" here.
                        found_in: workspace_scope_for(current_scope),
                    });
                }
                let sub_name = pm.workspace.as_str().to_owned();
                let declared = central_db.directly_declared_harnesses(&pm.workspace)?;
                let mut sub_chain = source_chain.clone();
                sub_chain.push("[workspace]".to_string());
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
                sub_chain.push(format!("[workspaces.{}]", name.as_str()));
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
                state.enter(ScopeKind::Global, "<global>")?;
                let sub_result = if let Some(global_list) = global_settings.harnesses.as_ref() {
                    let mut sub_chain = source_chain.clone();
                    sub_chain.push("[global]".to_string());
                    resolve_list(
                        global_list,
                        ScopeKind::Global,
                        sub_chain,
                        project_marker,
                        bound_workspace,
                        global_settings,
                        central_db,
                        state,
                    )
                } else {
                    Ok(())
                };
                state.leave();
                sub_result?;
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
    source_chain: Vec<String>,
    project_marker: Option<&ProjectMarkerConfig>,
    bound_workspace: Option<&WorkspaceSettings>,
    global_settings: &GlobalSettings,
    central_db: &P,
    state: &mut ResolveState,
) -> Result<(), CompositionErrorKind> {
    state.enter(ScopeKind::Workspace, name)?;
    let result = if let Some(list) = declared {
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
    } else {
        // FR-449: workspace exists but didn't declare `harnesses`.
        // Resolve to the empty list — do NOT fall through to global.
        Ok(())
    };
    state.leave();
    result
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
            expose_agents_as_personas: None,
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
            expose_agents_as_personas: None,
        };
        let err = resolve_effective_list(None, Some(&ws), &empty_global(), &stub).expect_err("err");
        assert!(matches!(
            err,
            CompositionErrorKind::WorkspaceRefOutsideProject { .. }
        ));
    }
}

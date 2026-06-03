//! T259 / FR-441 — three-layer priority walk.
//!
//! Stop-at-first-declarer: the FIRST scope whose `harnesses` key is
//! `Some(...)` terminates the priority walk, regardless of whether the
//! list is empty (`Some(vec![])` opts out entirely; FR-442). Lower-
//! priority scopes are NOT consulted unless the first declarer's list
//! contains an explicit composition reference (`[global]`,
//! `[workspaces.<name>]`).

use crate::common::{HarnessModulesGuard, NamedStubHarness};
use tome::settings::resolver::{StubScope, resolve_effective_list};
use tome::settings::{GlobalSettings, ProjectMarkerConfig, WorkspaceSettings};
use tome::workspace::WorkspaceName;

fn install_synthetic() -> (HarnessModulesGuard, std::sync::MutexGuard<'static, ()>) {
    // Tuple drop order matters: fields drop in declared order, so
    // HarnessModulesGuard (override → None) MUST drop BEFORE the
    // MutexGuard (releases serialisation). Reversing the tuple opens a
    // race window where a concurrent test grabs the mutex + installs a
    // new override, then this test's HarnessModulesGuard::drop clears
    // it — observable on macOS stable as flaky `HarnessNotSupported`
    // failures in the override-using tests.
    let lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let names = ["a", "b", "c", "should-not-appear"];
    let guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set(names));
    (guard, lock)
}

fn ws(name: &str, harnesses: Option<Vec<String>>) -> WorkspaceSettings {
    WorkspaceSettings {
        name: WorkspaceName::parse(name).expect("test workspace name parses"),
        summaries: None,
        catalogs: Vec::new(),
        harnesses,
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
    }
}

fn project(workspace: &str, harnesses: Option<Vec<String>>) -> ProjectMarkerConfig {
    ProjectMarkerConfig {
        workspace: WorkspaceName::parse(workspace).expect("test workspace name parses"),
        harnesses,
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
    }
}

fn names(harnesses: &[tome::settings::EffectiveHarness]) -> Vec<&str> {
    harnesses.iter().map(|h| h.name.as_str()).collect()
}

#[test]
fn global_declares_list_no_project_no_workspace_effective_from_global() {
    let _g = install_synthetic();
    let stub = StubScope::new();
    let global = GlobalSettings {
        harnesses: Some(vec!["a".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
    };
    let result = resolve_effective_list(None, None, &global, &stub).expect("resolves");
    assert_eq!(names(&result.harnesses), vec!["a"]);
    assert_eq!(result.harnesses[0].source_chain, vec!["global".to_string()]);
    assert!(result.excluded.is_empty());
}

#[test]
fn workspace_declares_list_no_project_effective_from_workspace_global_not_consulted() {
    let _g = install_synthetic();
    // Workspace declares `["b"]`; global declares `["should-not-appear"]`.
    // The priority walk terminates at the workspace declarer; global is
    // NOT consulted because the workspace's list contains no `[global]`
    // reference.
    let stub = StubScope::new();
    let ws_settings = ws("a", Some(vec!["b".to_owned()]));
    let global = GlobalSettings {
        harnesses: Some(vec!["should-not-appear".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
    };
    let result =
        resolve_effective_list(None, Some(&ws_settings), &global, &stub).expect("resolves");
    assert_eq!(names(&result.harnesses), vec!["b"]);
    assert_eq!(
        result.harnesses[0].source_chain,
        vec!["workspace".to_string()],
    );
}

#[test]
fn workspace_with_explicit_global_ref_unions() {
    let _g = install_synthetic();
    // Workspace's list explicitly references `[global]`; the union of
    // the workspace's directly-declared `["b"]` and global's `["a"]`
    // is the effective set.
    let stub = StubScope::new();
    let ws_settings = ws("a", Some(vec!["b".to_owned(), "[global]".to_owned()]));
    let global = GlobalSettings {
        harnesses: Some(vec!["a".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
    };
    let result =
        resolve_effective_list(None, Some(&ws_settings), &global, &stub).expect("resolves");
    let set: std::collections::HashSet<&str> = names(&result.harnesses).into_iter().collect();
    assert!(set.contains("a"));
    assert!(set.contains("b"));
}

#[test]
fn project_declares_list_workspace_global_ignored_unless_referenced() {
    let _g = install_synthetic();
    // Project declares `["a"]`. Workspace declares `["b"]`; global
    // declares `["c"]`. Neither workspace nor global is referenced, so
    // both are silently ignored.
    let stub = StubScope::new().with_workspace("ws", Some(vec!["b".to_owned()]));
    let proj = project("ws", Some(vec!["a".to_owned()]));
    let ws_settings = ws("ws", Some(vec!["b".to_owned()]));
    let global = GlobalSettings {
        harnesses: Some(vec!["c".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
    };
    let result =
        resolve_effective_list(Some(&proj), Some(&ws_settings), &global, &stub).expect("resolves");
    assert_eq!(names(&result.harnesses), vec!["a"]);
    assert_eq!(
        result.harnesses[0].source_chain,
        vec!["project".to_string()],
    );
}

#[test]
fn empty_list_at_any_scope_terminates_walk_with_empty_effective() {
    // FR-442: `Some(vec![])` at the workspace scope opts out entirely.
    // The walk does NOT fall through to global even though it has
    // content.
    let stub = StubScope::new();
    let ws_settings = ws("ws", Some(Vec::new()));
    let global = GlobalSettings {
        harnesses: Some(vec!["should-not-appear".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
    };
    let result =
        resolve_effective_list(None, Some(&ws_settings), &global, &stub).expect("resolves");
    assert!(result.harnesses.is_empty());
    assert!(result.excluded.is_empty());
}

#[test]
fn no_declaration_anywhere_returns_empty() {
    // All three scopes have `harnesses: None`. Result is empty, not an
    // error.
    let stub = StubScope::new();
    let result =
        resolve_effective_list(None, None, &GlobalSettings::default(), &stub).expect("resolves");
    assert!(result.harnesses.is_empty());
    assert!(result.excluded.is_empty());
}

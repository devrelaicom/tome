//! T270 / FR-446 — `[workspace]` is valid only inside a project scope.
//!
//! Three failure modes:
//!
//! 1. `[workspace]` appears in a workspace's directly-declared list →
//!    [`CompositionErrorKind::WorkspaceRefOutsideProject`] with `found_in`
//!    naming the workspace scope.
//! 2. `[workspace]` appears in a global settings file (and a project marker
//!    references global) → same variant with `found_in = Global`.
//! 3. `[workspace]` appears in a project's marker but no bound workspace
//!    was loaded (the binding is broken / not yet resolved) → same
//!    variant; the defensive fallback maps `found_in` to the closest-fit
//!    `Workspace` variant since `workspace::ScopeKind` carries no
//!    `Project` arm.
//!
//! The fourth test is a happy-path counter-test: a project marker with
//! `[workspace]` and a bound workspace resolves cleanly.

use crate::common::{HarnessModulesGuard, NamedStubHarness};
use tome::error::CompositionErrorKind;
use tome::settings::resolver::{StubScope, resolve_effective_list};
use tome::settings::{GlobalSettings, ProjectMarkerConfig, WorkspaceSettings};
use tome::workspace::WorkspaceName;

// Tuple order is `(HarnessModulesGuard, MutexGuard)` so fields drop in
// declared order — HarnessModulesGuard (override → None) BEFORE the
// MutexGuard (releases serialisation). The opposite order opens a race
// window where a concurrent test grabs the mutex + installs a new
// override, then this test's HarnessModulesGuard::drop clears it
// (observable on macOS stable as flaky `HarnessNotSupported` failures).
fn install_registry() -> (HarnessModulesGuard, std::sync::MutexGuard<'static, ()>) {
    let lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set([
        "claude-code",
        "codex",
        "cursor",
        "gemini",
        "opencode",
        "x",
    ]));
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

#[test]
fn workspace_ref_in_workspace_settings_returns_error() {
    let _g = install_registry();
    // Workspace's directly-declared list contains `[workspace]`. No
    // project marker — the priority walk lands on the workspace scope,
    // where the `[workspace]` token is not valid.
    let stub = StubScope::new();
    let ws_settings = ws("foo", Some(vec!["[workspace]".to_owned()]));
    let err = resolve_effective_list(None, Some(&ws_settings), &GlobalSettings::default(), &stub)
        .expect_err("must reject");
    assert!(
        matches!(err, CompositionErrorKind::WorkspaceRefOutsideProject { .. }),
        "expected WorkspaceRefOutsideProject, got {err:?}"
    );
}

#[test]
fn workspace_ref_in_global_settings_returns_error() {
    let _g = install_registry();
    // Global declares `[workspace]`. The priority walk lands on global
    // (no project, no workspace settings) and refuses.
    let stub = StubScope::new();
    let global = GlobalSettings {
        harnesses: Some(vec!["[workspace]".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
    };
    let err = resolve_effective_list(None, None, &global, &stub).expect_err("must reject");
    match err {
        CompositionErrorKind::WorkspaceRefOutsideProject { found_in } => {
            assert_eq!(
                found_in,
                tome::workspace::ScopeKind::Global,
                "global scope must surface in `found_in`"
            );
        }
        other => panic!("expected WorkspaceRefOutsideProject, got {other:?}"),
    }
}

#[test]
fn workspace_ref_in_project_with_no_bound_workspace_returns_error() {
    let _g = install_registry();
    // Project marker carries `[workspace]` but the caller passed
    // `bound_workspace: None`. The resolver refuses with the same
    // variant — composition cannot resolve `[workspace]` when no
    // workspace is loaded.
    let stub = StubScope::new();
    let proj = project("foo", Some(vec!["[workspace]".to_owned()]));
    let err = resolve_effective_list(Some(&proj), None, &GlobalSettings::default(), &stub)
        .expect_err("must reject");
    assert!(
        matches!(err, CompositionErrorKind::WorkspaceRefOutsideProject { .. }),
        "expected WorkspaceRefOutsideProject, got {err:?}"
    );
}

#[test]
fn workspace_ref_in_project_with_bound_workspace_resolves_cleanly() {
    let _g = install_registry();
    // Counter-test: when both the project marker AND a bound workspace
    // are passed (and the central registry knows the workspace), the
    // `[workspace]` token resolves to the workspace's directly-declared
    // list — `["x"]` here.
    let stub = StubScope::new().with_workspace("foo", Some(vec!["x".to_owned()]));
    let proj = project("foo", Some(vec!["[workspace]".to_owned()]));
    let ws_settings = ws("foo", Some(vec!["x".to_owned()]));
    let result = resolve_effective_list(
        Some(&proj),
        Some(&ws_settings),
        &GlobalSettings::default(),
        &stub,
    )
    .expect("must resolve");
    let names: Vec<&str> = result.harnesses.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, vec!["x"]);
}

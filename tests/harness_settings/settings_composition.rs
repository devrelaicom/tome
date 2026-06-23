//! T260 / FR-443 + FR-444 + FR-449 — composition resolution forms +
//! union + subtraction + order-independence.
//!
//! These tests reference synthetic harness names (`a`, `b`, `c`,
//! `alpha`, `x`, `y`, `z`, …). Since US3.b's FR-460 check rejects
//! anything not in [`SUPPORTED_HARNESSES`], every test installs a
//! permissive [`HarnessModulesGuard`] up-front via `install_synthetic`.

use std::collections::HashSet;

use crate::common::{HarnessModulesGuard, NamedStubHarness};
use tome::settings::resolver::{StubScope, resolve_effective_list};
use tome::settings::{GlobalSettings, ProjectMarkerConfig, WorkspaceSettings};
use tome::workspace::WorkspaceName;

/// Install a permissive registry containing every synthetic harness
/// name referenced in this file. The guard returned must be held for
/// the entire test body. Tuple order is `(HarnessModulesGuard,
/// MutexGuard)` so fields drop in declared order — HarnessModulesGuard
/// (override → None) BEFORE the MutexGuard (releases serialisation).
/// The opposite order opens a race window where a concurrent test
/// grabs the mutex + installs a new override, then this test's
/// HarnessModulesGuard::drop clears it (observable on macOS stable as
/// flaky `HarnessNotSupported` failures).
fn install_synthetic() -> (HarnessModulesGuard, std::sync::MutexGuard<'static, ()>) {
    let lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let names = [
        "a",
        "b",
        "c",
        "x",
        "y",
        "z",
        "g1",
        "g2",
        "alpha",
        "beta",
        "claude-code",
        "codex",
        "cursor",
    ];
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

fn name_set(harnesses: &[tome::settings::EffectiveHarness]) -> HashSet<String> {
    harnesses.iter().map(|h| h.name.clone()).collect()
}

#[test]
fn plain_include() {
    let _g = install_synthetic();
    let stub = StubScope::new();
    let global = GlobalSettings {
        enabled: Some(vec!["claude-code".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
        default_scope: None,
    };
    let result = resolve_effective_list(None, None, &global, &stub).expect("resolves");
    assert_eq!(
        name_set(&result.harnesses),
        HashSet::from(["claude-code".to_owned()])
    );
}

#[test]
fn exclude_subtracts_from_union() {
    let _g = install_synthetic();
    // Global = ["a", "b", "c"]; project = ["[global]", "!b"] → effective
    // = {a, c}; excluded = ["b"].
    let stub = StubScope::new().with_workspace("ws", None);
    let proj = project("ws", Some(vec!["[global]".to_owned(), "!b".to_owned()]));
    let ws_settings = ws("ws", None);
    let global = GlobalSettings {
        enabled: Some(vec!["a".to_owned(), "b".to_owned(), "c".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
        default_scope: None,
    };
    let result =
        resolve_effective_list(Some(&proj), Some(&ws_settings), &global, &stub).expect("resolves");
    assert_eq!(
        name_set(&result.harnesses),
        HashSet::from(["a".to_owned(), "c".to_owned()])
    );
    assert_eq!(result.excluded, vec!["b".to_owned()]);
}

#[test]
fn workspace_ref_in_project() {
    let _g = install_synthetic();
    // Project's `["[workspace]"]` resolves to the bound workspace's
    // directly-declared list `["x"]`.
    let stub = StubScope::new().with_workspace("ws", Some(vec!["x".to_owned()]));
    let proj = project("ws", Some(vec!["[workspace]".to_owned()]));
    let ws_settings = ws("ws", Some(vec!["x".to_owned()]));
    let result = resolve_effective_list(
        Some(&proj),
        Some(&ws_settings),
        &GlobalSettings::default(),
        &stub,
    )
    .expect("resolves");
    assert_eq!(name_set(&result.harnesses), HashSet::from(["x".to_owned()]));
    // Source chain must include both Project (entry) and Workspace
    // (the referenced scope) — FR-441 reporting.
    let harness = result
        .harnesses
        .iter()
        .find(|h| h.name == "x")
        .expect("x present");
    // After C-M1 source_chain is mixed-notation Vec<String>: the
    // entry-point scope is rendered as `"project"` and the recursion
    // step that pulled x in is `"[workspace]"`.
    assert!(harness.source_chain.iter().any(|s| s == "project"));
    assert!(harness.source_chain.iter().any(|s| s == "[workspace]"));
}

#[test]
fn named_workspace_ref() {
    let _g = install_synthetic();
    // Project references `[workspaces.other]`. The stub registry maps
    // `other` to `["y"]`; the effective set is `{y}`.
    let stub = StubScope::new()
        .with_workspace("ws", None)
        .with_workspace("other", Some(vec!["y".to_owned()]));
    let proj = project("ws", Some(vec!["[workspaces.other]".to_owned()]));
    let ws_settings = ws("ws", None);
    let result = resolve_effective_list(
        Some(&proj),
        Some(&ws_settings),
        &GlobalSettings::default(),
        &stub,
    )
    .expect("resolves");
    assert_eq!(name_set(&result.harnesses), HashSet::from(["y".to_owned()]));
}

#[test]
fn global_ref() {
    let _g = install_synthetic();
    // Workspace references `[global]`. Global declares `["z"]`.
    let stub = StubScope::new();
    let ws_settings = ws("ws", Some(vec!["[global]".to_owned()]));
    let global = GlobalSettings {
        enabled: Some(vec!["z".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
        default_scope: None,
    };
    let result =
        resolve_effective_list(None, Some(&ws_settings), &global, &stub).expect("resolves");
    assert_eq!(name_set(&result.harnesses), HashSet::from(["z".to_owned()]));
}

#[test]
fn multi_level_composition() {
    let _g = install_synthetic();
    // Project bound to workspace A. A's list = ["[workspaces.B]"]; B's
    // list = ["x", "y"]. Effective set = {x, y}.
    let stub = StubScope::new()
        .with_workspace("A", Some(vec!["[workspaces.B]".to_owned()]))
        .with_workspace("B", Some(vec!["x".to_owned(), "y".to_owned()]));
    let proj = project("A", Some(vec!["[workspace]".to_owned()]));
    let ws_settings = ws("A", Some(vec!["[workspaces.B]".to_owned()]));
    let result = resolve_effective_list(
        Some(&proj),
        Some(&ws_settings),
        &GlobalSettings::default(),
        &stub,
    )
    .expect("resolves");
    assert_eq!(
        name_set(&result.harnesses),
        HashSet::from(["x".to_owned(), "y".to_owned()])
    );
}

#[test]
fn explicit_add_and_remove_combined() {
    let _g = install_synthetic();
    // Project = ["a", "[global]", "!a", "b"]; global = ["x"].
    // Inclusions = {a, x, b}; exclusions = {a}; effective = {x, b};
    // excluded = ["a"].
    let stub = StubScope::new().with_workspace("ws", None);
    let proj = project(
        "ws",
        Some(vec![
            "a".to_owned(),
            "[global]".to_owned(),
            "!a".to_owned(),
            "b".to_owned(),
        ]),
    );
    let ws_settings = ws("ws", None);
    let global = GlobalSettings {
        enabled: Some(vec!["x".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
        default_scope: None,
    };
    let result =
        resolve_effective_list(Some(&proj), Some(&ws_settings), &global, &stub).expect("resolves");
    assert_eq!(
        name_set(&result.harnesses),
        HashSet::from(["x".to_owned(), "b".to_owned()])
    );
    assert_eq!(result.excluded, vec!["a".to_owned()]);
}

#[test]
fn order_of_entries_does_not_affect_membership() {
    let _g = install_synthetic();
    // FR-444: array entry order doesn't affect the resulting *set* of
    // included harnesses. The internal display order may differ but
    // membership must match.
    let stub_a = StubScope::new().with_workspace("ws", None);
    let stub_b = StubScope::new().with_workspace("ws", None);
    let ws_settings_a = ws("ws", None);
    let ws_settings_b = ws("ws", None);
    let global = GlobalSettings {
        enabled: Some(vec!["g1".to_owned(), "g2".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
        default_scope: None,
    };

    let proj_a = project(
        "ws",
        Some(vec![
            "alpha".to_owned(),
            "[global]".to_owned(),
            "!g1".to_owned(),
            "beta".to_owned(),
        ]),
    );
    let proj_b = project(
        "ws",
        Some(vec![
            "!g1".to_owned(),
            "beta".to_owned(),
            "[global]".to_owned(),
            "alpha".to_owned(),
        ]),
    );

    let result_a =
        resolve_effective_list(Some(&proj_a), Some(&ws_settings_a), &global, &stub_a).expect("a");
    let result_b =
        resolve_effective_list(Some(&proj_b), Some(&ws_settings_b), &global, &stub_b).expect("b");

    assert_eq!(name_set(&result_a.harnesses), name_set(&result_b.harnesses));
    assert_eq!(
        name_set(&result_a.harnesses),
        HashSet::from(["alpha".to_owned(), "beta".to_owned(), "g2".to_owned()])
    );
    assert_eq!(result_a.excluded, result_b.excluded);
}

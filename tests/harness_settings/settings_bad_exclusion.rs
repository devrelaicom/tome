//! T271 / FR-448 — `!`-prefixed exclusions accept plain harness names
//! only. Bracketed forms (`![global]`, `![workspace]`,
//! `![workspaces.<name>]`) parse as
//! [`CompositionErrorKind::BadExclusion`].
//!
//! Exclusions describe individual harnesses, not whole scopes; the
//! resolver pins this at parse time (in
//! [`tome::settings::CompositionRef::parse`]) so it surfaces with the
//! offending token preserved for the human-readable error. Resolution
//! exercises the same path end-to-end here.

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
        "cursor",
        "claude-code",
        "codex",
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
fn bracketed_global_exclusion_returns_bad_exclusion() {
    let _g = install_registry();
    let stub = StubScope::new();
    let global = GlobalSettings {
        harnesses: Some(vec!["![global]".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
    };
    let err = resolve_effective_list(None, None, &global, &stub).expect_err("must reject");
    match err {
        CompositionErrorKind::BadExclusion(token) => {
            assert_eq!(
                token, "![global]",
                "offending token must be preserved verbatim"
            );
        }
        other => panic!("expected BadExclusion, got {other:?}"),
    }
}

#[test]
fn bracketed_workspace_exclusion_returns_bad_exclusion() {
    let _g = install_registry();
    let stub = StubScope::new();
    let global = GlobalSettings {
        harnesses: Some(vec!["![workspace]".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
    };
    let err = resolve_effective_list(None, None, &global, &stub).expect_err("must reject");
    match err {
        CompositionErrorKind::BadExclusion(token) => {
            assert_eq!(token, "![workspace]");
        }
        other => panic!("expected BadExclusion, got {other:?}"),
    }
}

#[test]
fn bracketed_named_workspace_exclusion_returns_bad_exclusion() {
    let _g = install_registry();
    let stub = StubScope::new();
    let global = GlobalSettings {
        harnesses: Some(vec!["![workspaces.foo]".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
    };
    let err = resolve_effective_list(None, None, &global, &stub).expect_err("must reject");
    match err {
        CompositionErrorKind::BadExclusion(token) => {
            assert_eq!(token, "![workspaces.foo]");
        }
        other => panic!("expected BadExclusion, got {other:?}"),
    }
}

#[test]
fn plain_name_exclusion_works() {
    let _g = install_registry();
    // Counter-test: `!cursor` parses as a plain-name exclusion and
    // subtracts cleanly. The effective list is built from the bound
    // workspace's `["claude-code", "cursor"]`; subtracting `cursor`
    // leaves just `claude-code`.
    let stub = StubScope::new().with_workspace(
        "foo",
        Some(vec!["claude-code".to_owned(), "cursor".to_owned()]),
    );
    let proj = project(
        "foo",
        Some(vec!["[workspace]".to_owned(), "!cursor".to_owned()]),
    );
    let ws_settings = ws(
        "foo",
        Some(vec!["claude-code".to_owned(), "cursor".to_owned()]),
    );
    let result = resolve_effective_list(
        Some(&proj),
        Some(&ws_settings),
        &GlobalSettings::default(),
        &stub,
    )
    .expect("must resolve");
    let names: Vec<&str> = result.harnesses.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, vec!["claude-code"]);
    assert_eq!(result.excluded, vec!["cursor".to_owned()]);
}

/// R-M8 (US3 review) — malformed bracketed refs are rejected at parse
/// time as `BadExclusion`. Before R-M8 these fell through to
/// `Include(...)`, silently producing an inclusion of a harness named
/// `[workspaces.a`.
#[test]
fn malformed_bracketed_refs_are_rejected() {
    let _g = install_registry();
    let stub = StubScope::new();
    let cases = [
        "[workspaces.a", // missing closing bracket
        "[unknown]",     // unrecognised inner keyword
        "[]",            // empty brackets
        "[workspaces.]", // empty workspace name
    ];
    for input in cases {
        let global = GlobalSettings {
            harnesses: Some(vec![input.to_owned()]),
            expose_agents_as_personas: None,
            strip_plugin_agent_privileges: None,
        };
        let err = resolve_effective_list(None, None, &global, &stub)
            .expect_err("malformed bracketed ref must error");
        assert!(
            matches!(err, CompositionErrorKind::BadExclusion(_)),
            "want BadExclusion for `{input}`, got {err:?}",
        );
    }
}

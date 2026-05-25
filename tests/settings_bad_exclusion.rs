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

mod common;

use std::sync::Mutex;

use common::{HarnessModulesGuard, NamedStubHarness};
use tome::error::CompositionErrorKind;
use tome::settings::resolver::{StubScope, resolve_effective_list};
use tome::settings::{GlobalSettings, ProjectMarkerConfig, WorkspaceSettings};
use tome::workspace::WorkspaceName;

static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

fn install_registry() -> (std::sync::MutexGuard<'static, ()>, HarnessModulesGuard) {
    let lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set([
        "cursor",
        "claude-code",
        "codex",
    ]));
    (lock, guard)
}

fn ws(name: &str, harnesses: Option<Vec<String>>) -> WorkspaceSettings {
    WorkspaceSettings {
        name: WorkspaceName::parse(name).expect("test workspace name parses"),
        summaries: None,
        catalogs: Vec::new(),
        harnesses,
    }
}

fn project(workspace: &str, harnesses: Option<Vec<String>>) -> ProjectMarkerConfig {
    ProjectMarkerConfig {
        workspace: WorkspaceName::parse(workspace).expect("test workspace name parses"),
        harnesses,
    }
}

#[test]
fn bracketed_global_exclusion_returns_bad_exclusion() {
    let _g = install_registry();
    let stub = StubScope::new();
    let global = GlobalSettings {
        harnesses: Some(vec!["![global]".to_owned()]),
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

//! T272 / FR-460 — composition references that resolve to a harness
//! name not in the production registry raise
//! [`CompositionErrorKind::HarnessNotSupported`], which the
//! `From<CompositionErrorKind> for TomeError` boundary rewrites as
//! [`TomeError::HarnessNotSupported`] (exit 18).
//!
//! Three positive tests cover the three scope file types (project,
//! workspace, global). A counter-test verifies that a fully-supported
//! list resolves cleanly.
//!
//! This file deliberately does NOT install
//! [`HARNESS_MODULES_OVERRIDE`] — the whole point of the test is that
//! the production registry (the five real harness names) rejects
//! everything else. The counter-test uses `"claude-code"` (production).

use tome::error::{CompositionErrorKind, TomeError};
use tome::settings::resolver::{StubScope, resolve_effective_list};
use tome::settings::{GlobalSettings, ProjectMarkerConfig, WorkspaceSettings};
use tome::workspace::WorkspaceName;

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
fn unsupported_name_in_project_settings_returns_error() {
    let stub = StubScope::new().with_workspace("foo", None);
    let proj = project("foo", Some(vec!["my-custom-harness".to_owned()]));
    let ws_settings = ws("foo", None);
    let err = resolve_effective_list(
        Some(&proj),
        Some(&ws_settings),
        &GlobalSettings::default(),
        &stub,
    )
    .expect_err("must reject unsupported");
    match err {
        CompositionErrorKind::HarnessNotSupported(name) => {
            assert_eq!(name, "my-custom-harness");
        }
        other => panic!("expected HarnessNotSupported, got {other:?}"),
    }
}

#[test]
fn unsupported_name_in_workspace_settings_returns_error() {
    let stub = StubScope::new();
    let ws_settings = ws("foo", Some(vec!["another-bad-name".to_owned()]));
    let err = resolve_effective_list(None, Some(&ws_settings), &GlobalSettings::default(), &stub)
        .expect_err("must reject unsupported");
    match err {
        CompositionErrorKind::HarnessNotSupported(name) => {
            assert_eq!(name, "another-bad-name");
        }
        other => panic!("expected HarnessNotSupported, got {other:?}"),
    }
}

#[test]
fn unsupported_name_in_global_settings_returns_error() {
    let stub = StubScope::new();
    let global = GlobalSettings {
        enabled: Some(vec!["yet-another-bad-name".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
        default_scope: None,
    };
    let err =
        resolve_effective_list(None, None, &global, &stub).expect_err("must reject unsupported");
    match err {
        CompositionErrorKind::HarnessNotSupported(name) => {
            assert_eq!(name, "yet-another-bad-name");
        }
        other => panic!("expected HarnessNotSupported, got {other:?}"),
    }
}

/// T-M8 / C-M4 (US3 review) — per-entry validation invariant.
///
/// Before C-M4 the unsupported-harness check ran at end-of-resolution
/// against the FINAL effective list. That meant `["fake", "!fake"]`
/// silently passed: the exclusion cancelled the inclusion before the
/// check ran. Contract `settings-composition.md` §Error mapping pins
/// per-entry validation: a typo'd inclusion is reported even if a later
/// exclusion would have cancelled it. After C-M4 the check runs inside
/// `resolve_list` for each `CompositionRef::Include`.
#[test]
fn fake_then_exclamation_fake_still_errors_per_entry() {
    let stub = StubScope::new();
    let global = GlobalSettings {
        enabled: Some(vec!["fake".to_owned(), "!fake".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
        default_scope: None,
    };
    let err =
        resolve_effective_list(None, None, &global, &stub).expect_err("must reject inclusion");
    match err {
        CompositionErrorKind::HarnessNotSupported(name) => {
            assert_eq!(name, "fake");
        }
        other => panic!("expected HarnessNotSupported, got {other:?}"),
    }
}

#[test]
fn supported_name_resolves_cleanly() {
    // Reads the production `SUPPORTED_HARNESSES` registry (expects `claude-code`
    // to resolve). Hold the process-global override mutex so a concurrent
    // override-installing test in this binary can't leak its
    // `HARNESS_MODULES_OVERRIDE` (e.g. a stub `["alpha","beta"]` set) into this
    // read — the guard's Drop restores the slot to `None`, so once we hold the
    // lock the registry is the real one. (Pre-existing parallel-execution race
    // surfaced by Phase 8's added test load.)
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let stub = StubScope::new();
    let global = GlobalSettings {
        enabled: Some(vec!["claude-code".to_owned()]),
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
        default_scope: None,
    };
    let result = resolve_effective_list(None, None, &global, &stub).expect("must resolve");
    let names: Vec<&str> = result.harnesses.iter().map(|h| h.name.as_str()).collect();
    assert_eq!(names, vec!["claude-code"]);
}

#[test]
fn boundary_maps_unsupported_to_exit_18() {
    // The resolver's `CompositionErrorKind::HarnessNotSupported` is
    // rewritten into `TomeError::HarnessNotSupported` at the call-site
    // boundary via `From<CompositionErrorKind> for TomeError`. Verify
    // the wire-visible exit code is 18 (not 17).
    let mapped: TomeError = CompositionErrorKind::HarnessNotSupported("nope".to_owned()).into();
    assert_eq!(mapped.exit_code(), 18);
}

#[test]
fn boundary_maps_unknown_workspace_to_exit_13() {
    // Same boundary rewrites `UnknownWorkspace` to
    // `WorkspaceNotFound` (exit 13) per FR-602. Other variants stay on
    // exit 17. Pin both rails so the rewrite table doesn't silently
    // drift.
    let unknown: TomeError = CompositionErrorKind::UnknownWorkspace("absent".to_owned()).into();
    assert_eq!(unknown.exit_code(), 13);

    let bad_excl: TomeError = CompositionErrorKind::BadExclusion("![global]".to_owned()).into();
    assert_eq!(bad_excl.exit_code(), 17);
}

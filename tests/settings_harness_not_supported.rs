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
    }
}

fn project(workspace: &str, harnesses: Option<Vec<String>>) -> ProjectMarkerConfig {
    ProjectMarkerConfig {
        workspace: WorkspaceName::parse(workspace).expect("test workspace name parses"),
        harnesses,
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
        harnesses: Some(vec!["yet-another-bad-name".to_owned()]),
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

#[test]
fn supported_name_resolves_cleanly() {
    let stub = StubScope::new();
    let global = GlobalSettings {
        harnesses: Some(vec!["claude-code".to_owned()]),
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

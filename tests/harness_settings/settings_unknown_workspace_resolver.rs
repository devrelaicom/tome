//! T-M1 (US3 review) — FR-447 boundary coverage via the resolver path.
//!
//! `[workspaces.<name>]` resolves through `ScopeProvider`; when the
//! provider reports `UnknownWorkspace`, the
//! `From<CompositionErrorKind> for TomeError` boundary rewrites that
//! into `TomeError::WorkspaceNotFound` (exit 13). Prior coverage only
//! tested the boundary mapping in isolation; this file covers the full
//! path from resolver through to the exit-code-bearing variant.

use crate::common::{HarnessModulesGuard, NamedStubHarness};
use tome::error::TomeError;
use tome::settings::resolver::{StubScope, resolve_effective_list};
use tome::settings::{GlobalSettings, ProjectMarkerConfig, WorkspaceSettings};
use tome::workspace::WorkspaceName;

fn install() -> (HarnessModulesGuard, std::sync::MutexGuard<'static, ()>) {
    let lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let guard = HarnessModulesGuard::install(NamedStubHarness::boxed_set(["x"]));
    (guard, lock)
}

fn ws(name: &str, harnesses: Option<Vec<String>>) -> WorkspaceSettings {
    WorkspaceSettings {
        name: WorkspaceName::parse(name).expect("parse"),
        summaries: None,
        catalogs: Vec::new(),
        harnesses,
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
        raw_event_passthrough: None,
    }
}

fn project(workspace: &str, harnesses: Option<Vec<String>>) -> ProjectMarkerConfig {
    ProjectMarkerConfig {
        workspace: WorkspaceName::parse(workspace).expect("parse"),
        harnesses,
        expose_agents_as_personas: None,
        strip_plugin_agent_privileges: None,
    }
}

/// Resolver surfaces `UnknownWorkspace` when `[workspaces.unknown]` is
/// followed and the provider has no row for `unknown`. The
/// `TomeError::from(CompositionErrorKind)` boundary maps that to
/// `WorkspaceNotFound` (exit 13).
#[test]
fn named_workspace_ref_to_unknown_resolves_to_exit_13() {
    let _g = install();
    // Project references `[workspaces.unknown]`; stub registry has
    // only `foo`.
    let stub = StubScope::new().with_workspace("foo", Some(vec!["x".to_owned()]));
    let proj = project("foo", Some(vec!["[workspaces.unknown]".to_owned()]));
    let ws_settings = ws("foo", Some(vec!["x".to_owned()]));

    let kind = resolve_effective_list(
        Some(&proj),
        Some(&ws_settings),
        &GlobalSettings::default(),
        &stub,
    )
    .expect_err("must reject");

    // Cross the From boundary; verify exit code.
    let mapped: TomeError = kind.into();
    assert_eq!(mapped.exit_code(), 13);
    assert!(
        matches!(mapped, TomeError::WorkspaceNotFound { ref name } if name == "unknown"),
        "expected WorkspaceNotFound(unknown), got {mapped:?}",
    );
}

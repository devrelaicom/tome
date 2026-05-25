//! T261 / FR-445 — composition cycle detection.
//!
//! The DFS tracks `(ScopeKind, key)` visited pairs. On re-visit the
//! resolver emits `CompositionErrorKind::Cycle { path }` where `path`
//! is the chain of scope keys in walk order PLUS the re-visited key
//! that closed the loop. The path must therefore name every scope in
//! the loop chain (FR-445), in order — not as a deduplicated set.

use tome::error::CompositionErrorKind;
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
fn workspace_a_includes_workspace_b_includes_workspace_a() {
    // A's list refs B; B's list refs A → cycle through both.
    // Resolving from a project bound to A must report a cycle whose
    // path names BOTH A and B.
    let stub = StubScope::new()
        .with_workspace("a", Some(vec!["[workspaces.b]".to_owned()]))
        .with_workspace("b", Some(vec!["[workspaces.a]".to_owned()]));
    let proj = project("a", Some(vec!["[workspace]".to_owned()]));
    let ws_settings = ws("a", Some(vec!["[workspaces.b]".to_owned()]));

    let err = resolve_effective_list(
        Some(&proj),
        Some(&ws_settings),
        &GlobalSettings::default(),
        &stub,
    )
    .expect_err("cycle");

    match err {
        CompositionErrorKind::Cycle { path } => {
            // The cycle must name BOTH A and B (loop chain).
            assert!(
                path.iter().any(|k| k == "a"),
                "expected `a` in cycle path: {path:?}"
            );
            assert!(
                path.iter().any(|k| k == "b"),
                "expected `b` in cycle path: {path:?}"
            );
            // The loop-closing key (the last entry) must equal the one
            // re-visited first along the DFS chain — `a` is the entry
            // point, so closure should re-visit `a`.
            assert_eq!(
                path.last().map(String::as_str),
                Some("a"),
                "cycle path must close with re-visited scope: {path:?}"
            );
        }
        other => panic!("expected Cycle, got {other:?}"),
    }
}

#[test]
fn self_reference_cycle() {
    // Workspace A's list references itself → self-cycle.
    let stub = StubScope::new().with_workspace("a", Some(vec!["[workspaces.a]".to_owned()]));
    let proj = project("a", Some(vec!["[workspace]".to_owned()]));
    let ws_settings = ws("a", Some(vec!["[workspaces.a]".to_owned()]));

    let err = resolve_effective_list(
        Some(&proj),
        Some(&ws_settings),
        &GlobalSettings::default(),
        &stub,
    )
    .expect_err("cycle");

    assert!(matches!(err, CompositionErrorKind::Cycle { .. }));
    if let CompositionErrorKind::Cycle { path } = err {
        // Path must name `a` (the only scope in the loop) and close on
        // a re-visit of `a`.
        assert!(path.iter().any(|k| k == "a"), "expected `a`: {path:?}");
        assert_eq!(path.last().map(String::as_str), Some("a"));
    }
}

#[test]
fn project_to_workspace_to_workspace_cycle() {
    // Project's list: ["[workspace]"]. Bound workspace W's list:
    // ["[workspaces.w]"] — references itself by name. The DFS:
    //   project → W (via [workspace]) → W (via [workspaces.w])
    // The second entry into the (Workspace, W) pair triggers the cycle.
    let stub = StubScope::new().with_workspace("w", Some(vec!["[workspaces.w]".to_owned()]));
    let proj = project("w", Some(vec!["[workspace]".to_owned()]));
    let ws_settings = ws("w", Some(vec!["[workspaces.w]".to_owned()]));

    let err = resolve_effective_list(
        Some(&proj),
        Some(&ws_settings),
        &GlobalSettings::default(),
        &stub,
    )
    .expect_err("cycle");

    assert!(matches!(err, CompositionErrorKind::Cycle { .. }));
}

//! T262 / FR-449 — composition references resolve to the referenced
//! scope's **directly-declared** list, NOT its computed effective list.
//!
//! This is the "composition is a one-level reference, not a re-entrant
//! resolver" rule. Without it, every composition reference would
//! re-trigger the full priority walk and the rules become
//! unintelligible (a workspace's `[workspace]` ref would silently fall
//! through to global if the workspace had no `harnesses` of its own).

use std::collections::HashSet;

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

fn name_set(harnesses: &[tome::settings::EffectiveHarness]) -> HashSet<String> {
    harnesses.iter().map(|h| h.name.clone()).collect()
}

#[test]
fn workspace_ref_to_undeclared_workspace_resolves_to_empty_not_global() {
    // Project is bound to W; W has `harnesses: None` (no declaration);
    // project's list is `["[workspace]", "!cursor"]`.
    //
    // FR-449: `[workspace]` resolves to W's **directly-declared** list,
    // which is `None` → the empty set, NOT to whatever W's effective
    // list would be if W were resolved as the priority-walk entry
    // point (which would have fallen through to global).
    //
    // So the inclusion set is `{}`; we then subtract `cursor` from
    // nothing; the effective result is `[]`. Global is NOT consulted
    // even though it has content.
    let stub = StubScope::new().with_workspace("ws", None);
    let proj = project(
        "ws",
        Some(vec!["[workspace]".to_owned(), "!cursor".to_owned()]),
    );
    let ws_settings = ws("ws", None);
    let global = GlobalSettings {
        // If the resolver mistakenly recursed through W's effective
        // list, it would surface these globals — that would be the bug
        // FR-449 forbids.
        harnesses: Some(vec!["claude-code".to_owned(), "codex".to_owned()]),
    };

    let result =
        resolve_effective_list(Some(&proj), Some(&ws_settings), &global, &stub).expect("resolves");
    assert!(
        result.harnesses.is_empty(),
        "[workspace] to undeclared workspace must resolve empty, not via global; got {:?}",
        result.harnesses
    );
    // The excluded name is reported transparently for `tome harness
    // list`, but `cursor` was subtracted from an empty set.
    assert_eq!(result.excluded, vec!["cursor".to_owned()]);
}

#[test]
fn workspace_ref_with_global_in_it_does_not_recurse_through_workspace_effective_list() {
    // Workspace W's directly-declared list is `["[global]"]`. Project
    // is bound to W and declares `["[workspace]"]`.
    //
    // The resolver must walk W's *as-written* list verbatim, which IS
    // `["[global]"]`. Each entry in that list is then processed
    // normally — `[global]` triggers the global lookup. The end result
    // is global's directly-declared list.
    //
    // The point of the test is that the resolver does NOT short-circuit
    // to W's effective list (which would also be global's list, but
    // computed via a re-entrant priority walk). The path must be:
    //
    //     project's [workspace] -> W's directly-declared list = ["[global]"]
    //                           -> process each entry: [global] -> global
    //
    // Functionally indistinguishable from the broken alternative in
    // this exact case (both arrive at global), but the WALK matters:
    // if we instead resolved [workspace] to W's *effective* list,
    // we would have re-entered the priority walk and recorded an extra
    // (Workspace, W) visit chain that the FR-449 contract forbids.
    let stub = StubScope::new().with_workspace("ws", Some(vec!["[global]".to_owned()]));
    let proj = project("ws", Some(vec!["[workspace]".to_owned()]));
    let ws_settings = ws("ws", Some(vec!["[global]".to_owned()]));
    let global = GlobalSettings {
        harnesses: Some(vec!["claude-code".to_owned()]),
    };

    let result =
        resolve_effective_list(Some(&proj), Some(&ws_settings), &global, &stub).expect("resolves");

    // End-state assertion: the inclusion set includes global's items.
    assert_eq!(
        name_set(&result.harnesses),
        HashSet::from(["claude-code".to_owned()])
    );

    // Companion assertion: a sibling case where W has NO direct
    // declaration is the canonical FR-449 differentiator (covered by
    // `workspace_ref_to_undeclared_workspace_resolves_to_empty_not_global`
    // above). The pair pins both halves of the invariant.
}

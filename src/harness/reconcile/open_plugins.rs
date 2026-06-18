//! Open Plugins `tome-op` bundle reconciliation (Phase 11 / US4) — the
//! OPEN_PLUGINS sink.
//!
//! Some harnesses (`generic-op`, `goose`) integrate by hosting a self-contained
//! Open Plugins `tome-op` plugin rather than per-sink rules/MCP files. A harness
//! declares it via [`HarnessModule::open_plugins_root`](crate::harness::HarnessModule::open_plugins_root)
//! returning `Some(root)`; this reconciler emits the whole bundle atomically
//! ([`open_plugins::emit_tome_op`]) for a live harness and removes it
//! (structural-match) for a non-live one.
//!
//! ## Dispatch INSTEAD of the per-sink loop (no double-write)
//!
//! Harnesses with `open_plugins_root == Some` are PARTITIONED OUT of the
//! orchestrator's rules/MCP/hooks/guardrails/agents/plugins snapshots before
//! those passes run (see [`crate::harness::sync::sync_project`]). The bundle is
//! all-or-nothing — its `AGENTS.md` + `.mcp.json` ARE the rules + MCP surface —
//! so routing it through the per-sink loop would double-write those files
//! (and the per-sink writers don't stage atomically as a unit). This reconciler
//! is the single owner of those harnesses' on-disk state.
//!
//! ## Mass-delete safeguard
//!
//! Removal goes through [`open_plugins::remove_tome_op`], which deletes the
//! directory ONLY when it is recognisably the `tome-op` bundle (its
//! `.plugin/plugin.json` names `tome-op`). A developer's same-named or sibling
//! directory is left untouched.
//!
//! ## Forward progress / fast-exit
//!
//! Mirrors the other reconcilers: a per-harness aggregate [`Action`] map keyed on
//! `name()` + a forward-progress `first_error`. With no Open Plugins harness in
//! the snapshot set the pass is a NO-OP, so the orchestrator output is
//! byte-identical for every project without one. Sync-only —
//! `tests/sync_boundary.rs` guards this tree.

use std::collections::HashSet;
use std::path::Path;

use crate::error::TomeError;
use crate::harness::open_plugins::{self, RemoveOutcome};
use crate::harness::reconcile::record_action;
use crate::harness::sync::{Action, HarnessSnapshot, SyncDeps, SyncOutcome, SyncSubsystem};

/// Reconcile the Open Plugins `tome-op` bundle for every harness whose
/// `open_plugins_root` is `Some` (Phase 11 / US4).
///
/// Live harness → emit the whole bundle atomically (`Created`/`Updated` — the
/// atomic landing always replaces, so a pre-existing bundle is `Updated`, a
/// fresh one `Created`). Non-live → remove ONLY the Tome-owned `tome-op` bundle
/// (structural match; `Removed` when our bundle was present, else `LeftAlone`).
///
/// A failure for one harness is recorded on `first_error` and does NOT abort the
/// pass; sibling harnesses still reconcile. Returns the per-harness aggregate
/// action map (keyed on `name()`) plus that first error.
pub(crate) fn reconcile_open_plugins(
    project_root: &Path,
    deps: &SyncDeps<'_>,
    effective_names: &HashSet<String>,
    snapshots: &[HarnessSnapshot],
    outcome: &mut SyncOutcome,
) -> (std::collections::HashMap<String, Action>, Option<TomeError>) {
    let mut actions = std::collections::HashMap::new();
    let mut first_error: Option<TomeError> = None;

    for snap in snapshots {
        let Some(root) = snap.open_plugins_root.as_ref() else {
            continue;
        };
        let is_live = effective_names.contains(&snap.name);

        let action = if is_live {
            emit_bundle(snap, root, deps, project_root, outcome, &mut first_error)
        } else {
            remove_bundle(snap, root, outcome, &mut first_error)
        };
        actions.insert(snap.name.clone(), action);
    }

    (actions, first_error)
}

/// Emit the `tome-op` bundle for a live harness. The atomic landing always
/// replaces, so classify `Updated` when a bundle already existed and `Created`
/// when not. A symlink refusal / IO failure is recorded on `first_error`.
fn emit_bundle(
    snap: &HarnessSnapshot,
    root: &Path,
    deps: &SyncDeps<'_>,
    project_root: &Path,
    outcome: &mut SyncOutcome,
    first_error: &mut Option<TomeError>,
) -> Action {
    // `exists()` follows the final symlink; an exact-bundle check isn't needed
    // for the Created-vs-Updated label, and `emit_tome_op` re-runs the symlink
    // guard before any write.
    let pre_existed = root.exists();
    match open_plugins::emit_tome_op(root, project_root, deps.workspace_name.as_str(), &snap.name) {
        Ok(()) => {
            let action = if pre_existed {
                Action::Updated
            } else {
                Action::Created
            };
            record_action(
                outcome,
                &snap.name,
                SyncSubsystem::OpenPlugins,
                root,
                action,
            );
            action
        }
        Err(e) => {
            if first_error.is_none() {
                *first_error = Some(e);
            }
            Action::LeftAlone
        }
    }
}

/// Remove ONLY the Tome-owned `tome-op` bundle for a non-live harness. A
/// non-bundle directory (or absent) is left untouched (`LeftAlone`).
fn remove_bundle(
    snap: &HarnessSnapshot,
    root: &Path,
    outcome: &mut SyncOutcome,
    first_error: &mut Option<TomeError>,
) -> Action {
    match open_plugins::remove_tome_op(root) {
        Ok(RemoveOutcome::Removed) => {
            record_action(
                outcome,
                &snap.name,
                SyncSubsystem::OpenPlugins,
                root,
                Action::Removed,
            );
            Action::Removed
        }
        Ok(RemoveOutcome::NotPresent | RemoveOutcome::NotTomeOp) => Action::LeftAlone,
        Err(e) => {
            if first_error.is_none() {
                *first_error = Some(e);
            }
            Action::LeftAlone
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::lookup;
    use crate::paths::Paths;
    use crate::workspace::WorkspaceName;
    use tempfile::TempDir;

    /// Build a real `generic-op` / `goose` snapshot via the orchestrator's path.
    fn op_snapshot(name: &str, project: &Path) -> HarnessSnapshot {
        let home = project.join("..").join(".home");
        let module = lookup(name).expect("module");
        crate::harness::sync::snapshot_for_test(module, project, &home)
    }

    fn deps_for<'a>(paths: &'a Paths, home: &'a Path, ws: &'a WorkspaceName) -> SyncDeps<'a> {
        SyncDeps {
            paths,
            home_root: home,
            workspace_name: ws,
            force: false,
            only_harness: None,
        }
    }

    #[test]
    fn live_emits_bundle_then_non_live_removes() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        // Seed the inline rules source the bundle's AGENTS.md mirrors.
        let rules = Paths::project_marker_rules(&project);
        std::fs::create_dir_all(rules.parent().unwrap()).unwrap();
        std::fs::write(&rules, "# r\n").unwrap();

        let paths = Paths::from_root(tmp.path().join(".tome"));
        let home = tmp.path().join(".home");
        let ws = WorkspaceName::global();
        let deps = deps_for(&paths, &home, &ws);

        let snapshots = vec![op_snapshot("goose", &project)];
        let root = project.join(".config/goose/plugins/tome-op");

        // Live → emit (Created).
        let live: HashSet<String> = std::iter::once("goose".to_string()).collect();
        let mut outcome = SyncOutcome::default();
        let (actions, err) =
            reconcile_open_plugins(&project, &deps, &live, &snapshots, &mut outcome);
        assert!(err.is_none(), "{err:?}");
        assert_eq!(actions.get("goose"), Some(&Action::Created));
        assert!(root.join(".plugin/plugin.json").is_file());
        assert!(root.join(".mcp.json").is_file());
        assert!(root.join("AGENTS.md").is_file());
        assert_eq!(outcome.added.len(), 1);
        assert_eq!(outcome.added[0].subsystem, SyncSubsystem::OpenPlugins);

        // Re-emit while still live → Updated (atomic landing replaces).
        let mut outcome2 = SyncOutcome::default();
        let (actions2, err2) =
            reconcile_open_plugins(&project, &deps, &live, &snapshots, &mut outcome2);
        assert!(err2.is_none());
        assert_eq!(actions2.get("goose"), Some(&Action::Updated));

        // Non-live → remove ONLY the bundle.
        let none: HashSet<String> = HashSet::new();
        let mut outcome3 = SyncOutcome::default();
        let (actions3, err3) =
            reconcile_open_plugins(&project, &deps, &none, &snapshots, &mut outcome3);
        assert!(err3.is_none());
        assert_eq!(actions3.get("goose"), Some(&Action::Removed));
        assert!(!root.exists(), "bundle removed");
        assert_eq!(outcome3.removed.len(), 1);
    }

    #[test]
    fn fast_exits_when_no_open_plugins_harness() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let paths = Paths::from_root(tmp.path().join(".tome"));
        let home = tmp.path().join(".home");
        let ws = WorkspaceName::global();
        let deps = deps_for(&paths, &home, &ws);

        // A plain harness (no open_plugins_root) → no actions.
        let snapshots = vec![crate::harness::sync::snapshot_for_test(
            &crate::harness::StubHarness::default(),
            &project,
            &home,
        )];
        let live: HashSet<String> = std::iter::once("stub".to_string()).collect();
        let mut outcome = SyncOutcome::default();
        let (actions, err) =
            reconcile_open_plugins(&project, &deps, &live, &snapshots, &mut outcome);
        assert!(actions.is_empty());
        assert!(err.is_none());
        assert!(outcome.added.is_empty() && outcome.removed.is_empty());
    }
}

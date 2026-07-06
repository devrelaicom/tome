//! Embedded TypeScript-shim reconciliation (Phase 11 / G2, T018) — the
//! PLUGINS sink.
//!
//! Some harnesses (Cline, Pi, OpenCode) cannot run a native Tome session-start
//! hook, so Tome ships a small TypeScript plugin shim per harness (embedded in
//! the binary by `build.rs`; see [`crate::harness::plugin_assets`]). A harness
//! declares it via [`SessionSteering::TsPlugin { dir, kind }`]; this reconciler
//! installs / removes that shim under the harness's Tome-managed plugin dir.
//!
//! It mirrors the [`agents`](crate::harness::reconcile::agents) sink shape:
//! a per-harness aggregate [`Action`] map keyed on `name()`, a forward-progress
//! `first_error`, the symlink-refusing single-file write through
//! [`rules_file::write_standalone`] (with the explicit
//! [`crate::util::refuse_symlinked_component`] pre-check), and the mass-delete
//! safeguard — a non-live harness has ONLY its Tome-owned `tome.ts` removed,
//! never the whole dir.
//!
//! ## Fast-exit / byte-identity
//!
//! With every Phase ≤10 module returning [`SessionSteering::None`] the fast
//! exit below makes the whole pass a NO-OP: no shim is written, no
//! `Plugins`-subsystem change is recorded, and `plugins_action` stays
//! `LeftAlone` (omitted from the JSON wire form via `skip_serializing_if`). So
//! the orchestrator output is byte-identical until a `TsPlugin` harness exists.
//!
//! ## Mass-delete safeguard
//!
//! This sink needs NO central-DB read (the shim bytes are the same regardless
//! of which plugins are enabled), so there is no enabled set that an unopenable
//! DB could collapse — the safeguard the other reconcilers carry does not apply
//! here. Removal is structural-match only: ONLY the Tome-owned `tome.ts`
//! filename is unlinked, and a developer's sibling file in the same dir is left
//! untouched.
//!
//! ## Exit-code classification
//!
//! A symlinked component on the shim write/remove path (intermediate OR final
//! node) refuses fail-closed → `Io` (exit 7), matching how the rules-file
//! `write_standalone`/`remove_standalone` classify a refused symlink. Generic
//! read/write IO maps to `Io` (7) the same way. Sync-only — `tests/sync_boundary.rs`
//! guards this tree.

use std::collections::HashSet;
use std::path::Path;

use crate::error::TomeError;
use crate::harness::reconcile::record_action;
use crate::harness::sync::{Action, HarnessSnapshot, SyncOutcome, SyncSubsystem};
use crate::harness::{SessionSteering, ShimKind, plugin_assets, rules_file};

/// The filename of the one shim file Tome owns in a `TsPlugin` harness's plugin
/// dir. Ownership is by filename + location: a `tome.ts` under the harness's
/// Tome-managed plugin dir is Tome's, and ONLY that file.
const SHIM_FILENAME: &str = "tome.ts";

/// Reconcile Tome's embedded TypeScript session-steering shim for every harness
/// whose [`SessionSteering`] is [`SessionSteering::TsPlugin`] (Phase 11 / G2).
///
/// Live harness → install the embedded shim (each file written atomically +
/// symlink-refusing; idempotent no-op when the bytes already match → classified
/// `Created`/`Updated`/`LeftAlone`). Non-live (or any harness not declaring
/// `TsPlugin`) → remove ONLY the Tome-owned `tome.ts` if present (structural
/// match; the rest of the dir is never touched; absent = no-op).
///
/// A read/write/refusal failure for one harness is recorded on the
/// forward-progress `first_error` and does NOT abort the pass; sibling
/// harnesses still reconcile. Returns the per-harness aggregate action map
/// (keyed on `name()`) plus that first error. Wired into the orchestrator LAST,
/// after the agents pass; the error is surfaced LAST in the fixed precedence
/// chain.
pub(crate) fn reconcile_plugins(
    project_root: &Path,
    effective_names: &HashSet<String>,
    snapshots: &[HarnessSnapshot],
    dry_run: bool,
    outcome: &mut SyncOutcome,
) -> (std::collections::HashMap<String, Action>, Option<TomeError>) {
    let mut actions = std::collections::HashMap::new();
    let mut first_error: Option<TomeError> = None;

    // Fast exit: no harness uses `TsPlugin` → no work, and (critically, with
    // every current module `None`) the orchestrator output is byte-identical.
    if !snapshots
        .iter()
        .any(|s| matches!(s.session_steering, SessionSteering::TsPlugin { .. }))
    {
        return (actions, first_error);
    }

    for snap in snapshots {
        let SessionSteering::TsPlugin { dir, kind } = &snap.session_steering else {
            continue;
        };

        // `dir` is the harness's Tome-managed plugin dir. Resolve it under
        // `project_root` (a `join` with an absolute `dir` keeps the absolute
        // path; a relative `dir` is anchored to the project).
        let resolved_dir = project_root.join(dir);
        let is_live = effective_names.contains(&snap.name);

        let action = if is_live {
            install_shim(
                &snap.name,
                &resolved_dir,
                *kind,
                dry_run,
                outcome,
                &mut first_error,
            )
        } else {
            remove_shim(
                &snap.name,
                &resolved_dir,
                dry_run,
                outcome,
                &mut first_error,
            )
        };
        actions.insert(snap.name.clone(), action);
    }

    (actions, first_error)
}

/// The harness id under which the embedded shim for a [`ShimKind`] is
/// registered in [`crate::harness::plugin_assets`].
fn shim_harness_id(kind: ShimKind) -> &'static str {
    match kind {
        ShimKind::Cline => "cline",
        ShimKind::Pi => "pi",
        ShimKind::OpenCode => "opencode",
    }
}

/// Install (write) the embedded shim for `kind` into `dir` for a live harness.
///
/// Each embedded file (today just `tome.ts`) is written atomically through the
/// symlink-refusing single-file writer. The aggregate [`Action`] is
/// `Created`/`Updated` when any file was (re)written and `LeftAlone` when every
/// file already matched on disk (idempotent re-sync). A missing embedded shim
/// for `kind`, or any write failure, is recorded on `first_error`.
fn install_shim(
    name: &str,
    dir: &Path,
    kind: ShimKind,
    dry_run: bool,
    outcome: &mut SyncOutcome,
    first_error: &mut Option<TomeError>,
) -> Action {
    let harness_id = shim_harness_id(kind);
    let Some(plugin) = plugin_assets::find(harness_id) else {
        // A `TsPlugin` harness whose embedded shim is absent is a build-time
        // invariant violation, but degrade to a recorded IO error on the
        // forward-progress path rather than panic in production sync.
        if first_error.is_none() {
            *first_error = Some(TomeError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no embedded harness shim for `{harness_id}`"),
            )));
        }
        return Action::LeftAlone;
    };

    let mut wrote = false;
    let mut updated = false;
    for file in plugin.files {
        let target = dir.join(file.rel_path);
        // Defence-in-depth: the embedded `rel_path` is validated `Normal`-only
        // at build time, but assert here too that the joined target stays
        // directly inside `dir` — never write outside the plugin dir.
        if target.parent() != Some(dir) {
            if first_error.is_none() {
                *first_error = Some(TomeError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("shim file `{}` escapes its plugin dir", file.rel_path),
                )));
            }
            continue;
        }
        match write_shim_file(&target, file.bytes, dry_run) {
            Ok(ShimWrite::Created) => {
                wrote = true;
                record_action(
                    outcome,
                    name,
                    SyncSubsystem::Plugins,
                    &target,
                    Action::Created,
                );
            }
            Ok(ShimWrite::Updated) => {
                updated = true;
                record_action(
                    outcome,
                    name,
                    SyncSubsystem::Plugins,
                    &target,
                    Action::Updated,
                );
            }
            Ok(ShimWrite::Unchanged) => {
                // Idempotent re-sync: identical bytes already on disk.
                outcome.leave_alones += 1;
            }
            Err(e) => {
                if first_error.is_none() {
                    *first_error = Some(e);
                }
            }
        }
    }

    if wrote {
        Action::Created
    } else if updated {
        Action::Updated
    } else {
        Action::LeftAlone
    }
}

/// Remove ONLY Tome's `tome.ts` shim from `dir` for a non-live / non-TsPlugin
/// harness (structural match; the rest of the dir is untouched; absent =
/// no-op). The removal is symlink-refusing (fail-closed): a symlinked shim is
/// refused and recorded on `first_error` rather than followed.
fn remove_shim(
    name: &str,
    dir: &Path,
    dry_run: bool,
    outcome: &mut SyncOutcome,
    first_error: &mut Option<TomeError>,
) -> Action {
    let target = dir.join(SHIM_FILENAME);
    // Structural-match removal: act only when our own `tome.ts` is present.
    // `symlink_metadata` does not follow the link, so a symlinked `tome.ts`
    // is detected here and then refused by `remove_standalone` below.
    if target.symlink_metadata().is_err() {
        return Action::LeftAlone;
    }
    // Dry run: preview the removal. The symlink refusal a real run would hit
    // still applies (read-only probe), so the preview surfaces the same error.
    if dry_run {
        if let Err(e) = crate::util::refuse_symlinked_component(&target).map_err(TomeError::Io) {
            if first_error.is_none() {
                *first_error = Some(e);
            }
            return Action::LeftAlone;
        }
        record_action(
            outcome,
            name,
            SyncSubsystem::Plugins,
            &target,
            Action::Removed,
        );
        return Action::Removed;
    }
    match rules_file::remove_standalone(&target) {
        Ok(()) => {
            record_action(
                outcome,
                name,
                SyncSubsystem::Plugins,
                &target,
                Action::Removed,
            );
            Action::Removed
        }
        Err(e) => {
            if first_error.is_none() {
                *first_error = Some(e);
            }
            Action::LeftAlone
        }
    }
}

/// Outcome of an atomic shim-file write.
#[derive(Debug, PartialEq, Eq)]
enum ShimWrite {
    Created,
    Updated,
    Unchanged,
}

/// Write one embedded shim file atomically, reusing the rules-file standalone
/// writer's discipline (symlink refusal on intermediate + final component,
/// umask-governed parent `create_dir_all`, idempotent no-op when bytes already
/// match). Classifies the result so the per-file `added`/`updated`/`leave_alones`
/// bookkeeping is accurate.
///
/// A symlinked component on the write path → `Io` (exit 7), matching how
/// `write_standalone` classifies a refused symlink (the shim sink has no
/// dedicated exit code; generic IO is the contract for it).
fn write_shim_file(target: &Path, bytes: &[u8], dry_run: bool) -> Result<ShimWrite, TomeError> {
    // Map the symlink refusal explicitly to `Io` (7) up front so a refused
    // intermediate/final component fails closed BEFORE the write. The shim
    // bytes are UTF-8 TypeScript source; decode for the byte-stable idempotence
    // compare + the verbatim `write_standalone` write.
    crate::util::refuse_symlinked_component(target).map_err(TomeError::Io)?;
    let contents = std::str::from_utf8(bytes).map_err(|_| {
        TomeError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "embedded shim bytes are not valid UTF-8",
        ))
    })?;

    // Idempotence pre-read: read the prior `tome.ts` (if any) to compare bytes.
    // An oversize prior file (> `HARNESS_RULES_MAX`) intentionally propagates
    // its bounded-read error rather than being silently overwritten — we never
    // blind-clobber a `tome.ts` we could not fully read back to compare.
    let prior = match crate::util::bounded_read_to_string(target, crate::util::HARNESS_RULES_MAX) {
        Ok(s) => Some(s),
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let classification = match prior.as_deref() {
        None => ShimWrite::Created,
        Some(existing) if existing == contents => return Ok(ShimWrite::Unchanged),
        Some(_) => ShimWrite::Updated,
    };
    // `write_standalone` is idempotent + atomic + symlink-refusing + creates
    // the parent dir via umask-governed `create_dir_all` — exactly the shim
    // write discipline.
    if !dry_run {
        rules_file::write_standalone(target, contents)?;
    }
    Ok(classification)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::StubHarness;
    use crate::paths::Paths;
    use crate::workspace::WorkspaceName;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Build a `HarnessSnapshot` for a stub declaring `TsPlugin { dir, kind }`,
    /// via the same path the orchestrator uses.
    fn ts_plugin_snapshot(dir: PathBuf, kind: ShimKind, project: &Path) -> HarnessSnapshot {
        let home = project.join("..").join(".home");
        let stub =
            StubHarness::default().with_session_steering(SessionSteering::TsPlugin { dir, kind });
        crate::harness::sync::snapshot_for_test(&stub, project, &home)
    }

    /// The exact embedded bytes Tome ships for a `ShimKind`, decoded as UTF-8.
    fn embedded_shim_bytes(kind: ShimKind) -> &'static [u8] {
        plugin_assets::find(shim_harness_id(kind))
            .expect("embedded shim exists")
            .files
            .iter()
            .find(|f| f.rel_path == "tome.ts")
            .expect("tome.ts in shim")
            .bytes
    }

    /// Live install writes `tome.ts` with the EXACT embedded bytes; a re-run is
    /// idempotent (`Unchanged` / `LeftAlone`); a subsequent non-live pass removes
    /// only `tome.ts`.
    #[test]
    fn live_installs_then_idempotent_then_non_live_removes() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let plugin_dir = PathBuf::from(".opencode/plugin");
        let snapshots = vec![ts_plugin_snapshot(
            plugin_dir.clone(),
            ShimKind::OpenCode,
            &project,
        )];

        // --- Live install ---
        let live: HashSet<String> = std::iter::once("stub".to_string()).collect();
        let mut outcome = SyncOutcome::default();
        let (actions, err) = reconcile_plugins(&project, &live, &snapshots, false, &mut outcome);
        assert!(err.is_none(), "{err:?}");
        assert_eq!(actions.get("stub"), Some(&Action::Created));
        let shim = project.join(".opencode/plugin/tome.ts");
        assert!(shim.is_file(), "tome.ts written");
        assert_eq!(
            std::fs::read(&shim).unwrap(),
            embedded_shim_bytes(ShimKind::OpenCode),
            "shim bytes match the embedded asset exactly"
        );
        assert_eq!(outcome.added.len(), 1);
        assert_eq!(outcome.added[0].subsystem, SyncSubsystem::Plugins);

        // --- Idempotent re-run (still live) ---
        let mut outcome2 = SyncOutcome::default();
        let (actions2, err2) = reconcile_plugins(&project, &live, &snapshots, false, &mut outcome2);
        assert!(err2.is_none());
        assert_eq!(actions2.get("stub"), Some(&Action::LeftAlone));
        assert!(outcome2.added.is_empty() && outcome2.updated.is_empty());
        assert_eq!(outcome2.leave_alones, 1);

        // --- Non-live: remove ONLY tome.ts ---
        let none: HashSet<String> = HashSet::new();
        let mut outcome3 = SyncOutcome::default();
        let (actions3, err3) = reconcile_plugins(&project, &none, &snapshots, false, &mut outcome3);
        assert!(err3.is_none());
        assert_eq!(actions3.get("stub"), Some(&Action::Removed));
        assert!(!shim.exists(), "tome.ts removed on non-live");
        assert_eq!(outcome3.removed.len(), 1);
    }

    /// Mass-delete safeguard: a developer's sibling file in the SAME plugin dir
    /// is NOT removed when the harness goes non-live.
    #[test]
    fn non_live_preserves_developer_sibling_file() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let plugin_dir = PathBuf::from(".pi/extensions");
        let snapshots = vec![ts_plugin_snapshot(plugin_dir, ShimKind::Pi, &project)];

        // Install live, then seed a developer file alongside tome.ts.
        let live: HashSet<String> = std::iter::once("stub".to_string()).collect();
        let mut outcome = SyncOutcome::default();
        reconcile_plugins(&project, &live, &snapshots, false, &mut outcome);
        let dev = project.join(".pi/extensions/dev.ts");
        std::fs::write(&dev, b"// developer's own extension\n").unwrap();

        // Non-live: only tome.ts goes.
        let none: HashSet<String> = HashSet::new();
        let mut outcome2 = SyncOutcome::default();
        let (_, err) = reconcile_plugins(&project, &none, &snapshots, false, &mut outcome2);
        assert!(err.is_none());
        assert!(
            !project.join(".pi/extensions/tome.ts").exists(),
            "tome.ts removed"
        );
        assert!(dev.is_file(), "developer sibling NOT removed");
    }

    /// In-place upgrade path: a STALE `tome.ts` (different bytes) on disk is
    /// rewritten to the embedded asset and classified `Updated`, with the
    /// `outcome.updated` bookkeeping recording one `Plugins` entry.
    #[test]
    fn stale_shim_is_updated_in_place() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let plugin_dir = PathBuf::from(".opencode/plugin");
        let snapshots = vec![ts_plugin_snapshot(
            plugin_dir.clone(),
            ShimKind::OpenCode,
            &project,
        )];

        // Seed a STALE `tome.ts` (different bytes than the embedded asset).
        let shim = project.join(".opencode/plugin/tome.ts");
        std::fs::create_dir_all(shim.parent().unwrap()).unwrap();
        std::fs::write(&shim, b"// stale tome.ts - must be overwritten\n").unwrap();
        assert_ne!(
            std::fs::read(&shim).unwrap(),
            embedded_shim_bytes(ShimKind::OpenCode),
            "precondition: the seeded shim differs from the embedded asset",
        );

        let live: HashSet<String> = std::iter::once("stub".to_string()).collect();
        let mut outcome = SyncOutcome::default();
        let (actions, err) = reconcile_plugins(&project, &live, &snapshots, false, &mut outcome);
        assert!(err.is_none(), "{err:?}");
        assert_eq!(
            actions.get("stub"),
            Some(&Action::Updated),
            "a stale shim must be classified Updated, not Created/LeftAlone",
        );
        assert_eq!(
            std::fs::read(&shim).unwrap(),
            embedded_shim_bytes(ShimKind::OpenCode),
            "the shim is now byte-identical to the embedded asset",
        );
        assert!(outcome.added.is_empty(), "an in-place upgrade adds nothing");
        assert_eq!(
            outcome.updated.len(),
            1,
            "exactly one Plugins update recorded",
        );
        assert_eq!(outcome.updated[0].subsystem, SyncSubsystem::Plugins);
    }

    /// `reconcile_plugins` fast-exits (no work, no error) when every snapshot is
    /// `SessionSteering::None` — the byte-identity guarantee.
    #[test]
    fn fast_exits_when_all_none() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let stub = StubHarness::default();
        let snapshots = vec![crate::harness::sync::snapshot_for_test(
            &stub,
            &project,
            tmp.path(),
        )];
        let live: HashSet<String> = std::iter::once("stub".to_string()).collect();
        let mut outcome = SyncOutcome::default();
        let (actions, err) = reconcile_plugins(&project, &live, &snapshots, false, &mut outcome);
        assert!(actions.is_empty(), "no TsPlugin harness → no actions");
        assert!(err.is_none());
        assert!(outcome.added.is_empty() && outcome.removed.is_empty());
        assert_eq!(outcome.leave_alones, 0);
    }

    // -----------------------------------------------------------------------
    // Symlink refusal on the shim write path (intermediate + final node) →
    // refused, fail-closed (file not written), classified `Io` (exit 7).
    // Mirrors the agents-sink symlink tests but for the plugins sink's
    // generic-IO classification.
    // -----------------------------------------------------------------------
    #[cfg(unix)]
    #[test]
    fn write_refuses_symlinked_intermediate_with_exit_7() {
        use std::os::unix::fs::symlink;
        let root = TempDir::new().unwrap();
        let base = root.path().canonicalize().unwrap();
        let real_dir = base.join("real_plugins");
        std::fs::create_dir(&real_dir).unwrap();
        symlink(&real_dir, base.join("link_plugins")).unwrap();

        let target = base.join("link_plugins").join("tome.ts");
        let err = write_shim_file(&target, b"// shim\n", false)
            .expect_err("symlinked intermediate component must be refused");
        assert_eq!(
            err.exit_code(),
            7,
            "plugins-sink refusal maps to Io (7); got {err:?}"
        );
        assert!(matches!(err, TomeError::Io(_)), "expected Io, got {err:?}");
        // Fail-closed: nothing written through the link.
        assert!(
            !real_dir.join("tome.ts").exists(),
            "no file written through symlink"
        );
    }

    #[cfg(unix)]
    #[test]
    fn write_refuses_symlinked_final_node_with_exit_7() {
        use std::os::unix::fs::symlink;
        let root = TempDir::new().unwrap();
        let base = root.path().canonicalize().unwrap();
        let dir = base.join("plugins");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(base.join("decoy.ts"), b"x").unwrap();
        let target = dir.join("tome.ts");
        symlink(base.join("decoy.ts"), &target).unwrap();

        let err = write_shim_file(&target, b"// shim\n", false)
            .expect_err("symlinked final node must be refused");
        assert_eq!(err.exit_code(), 7, "got {err:?}");
        // The decoy target is untouched (write refused, not followed).
        assert_eq!(std::fs::read(base.join("decoy.ts")).unwrap(), b"x");
    }

    /// Symlink-refusing REMOVAL: a symlinked `tome.ts` is refused fail-closed
    /// (the symlink and its target survive) and recorded on `first_error`.
    #[cfg(unix)]
    #[test]
    fn remove_refuses_symlinked_shim_fail_closed() {
        use std::os::unix::fs::symlink;
        let root = TempDir::new().unwrap();
        let base = root.path().canonicalize().unwrap();
        let dir = base.join("plugin");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(base.join("decoy.ts"), b"x").unwrap();
        let shim = dir.join("tome.ts");
        symlink(base.join("decoy.ts"), &shim).unwrap();

        let mut outcome = SyncOutcome::default();
        let mut first_error = None;
        let action = remove_shim("stub", &dir, false, &mut outcome, &mut first_error);
        assert_eq!(action, Action::LeftAlone);
        let err = first_error.expect("symlinked shim must be refused + recorded");
        assert_eq!(err.exit_code(), 7, "got {err:?}");
        assert!(
            shim.symlink_metadata().is_ok(),
            "the symlinked shim must NOT be unlinked (fail-closed)"
        );
        assert!(base.join("decoy.ts").is_file(), "decoy target untouched");
    }

    /// End-to-end through `sync_project` is covered by the integration suite;
    /// this unit proves the orchestrator-facing return shape for a clean live
    /// install via a real `Paths`/`WorkspaceName` to exercise the same field
    /// plumbing the orchestrator reads.
    #[test]
    fn clean_live_install_returns_created_action() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let _paths = Paths::from_root(tmp.path().join(".tome"));
        let _ws = WorkspaceName::global();
        let snapshots = vec![ts_plugin_snapshot(
            PathBuf::from(".cline/plugins"),
            ShimKind::Cline,
            &project,
        )];
        let live: HashSet<String> = std::iter::once("stub".to_string()).collect();
        let mut outcome = SyncOutcome::default();
        let (actions, err) = reconcile_plugins(&project, &live, &snapshots, false, &mut outcome);
        assert!(err.is_none());
        assert_eq!(actions.get("stub"), Some(&Action::Created));
        assert!(project.join(".cline/plugins/tome.ts").is_file());
    }
}

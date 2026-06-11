//! `tome harness sync` — reconcile the project's filesystem state
//! against the effective harness list.
//!
//! Requires a resolved project; otherwise exit 2 (Usage). Wraps
//! [`crate::harness::sync::sync_project`]; emits per `mode`.
//!
//! Per FR-525, byte-for-byte idempotent: on a second invocation where
//! the effective list and the filesystem already agree, no file is
//! rewritten and `outcome` carries empty `added` / `updated` /
//! `removed` lists.

use std::io::Write;

use crate::error::TomeError;
use crate::harness::sync as sync_lib;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::workspace::ResolvedScope;

use super::home_root;

pub fn run(scope: &ResolvedScope, paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let Some(project_root) = scope.project_root.as_deref() else {
        return Err(TomeError::Usage(
            "`tome harness sync` requires a project marker — run inside a project bound via `tome workspace use`"
                .into(),
        ));
    };
    let home = home_root()?;
    // `tome harness sync` does not expose --force in v1; users wanting
    // to override a clash explicitly run `tome workspace use <name>
    // --force` to re-bind.
    let deps = sync_lib::build_deps(paths, &home, scope.scope.name(), false);
    let outcome = sync_lib::sync_project(project_root, &deps)?;

    // `tome harness sync` reconciles ALL effective harnesses, not one — so emit
    // one `tome.harness_action{Sync}` per DISTINCT harness that actually had a
    // change in this sync. Unmapped names are skipped by `emit_harness_action`.
    // best-effort: an empty (fully idempotent) sync emits nothing here.
    let mut seen: Vec<&str> = Vec::new();
    for change in outcome
        .added
        .iter()
        .chain(outcome.updated.iter())
        .chain(outcome.removed.iter())
    {
        if !seen.contains(&change.harness.as_str()) {
            seen.push(change.harness.as_str());
            super::emit_harness_action(
                &change.harness,
                crate::telemetry::event::HarnessAction::Sync,
            );
        }
    }

    match mode {
        Mode::Human => emit_human(&outcome),
        Mode::Json => write_json(&outcome),
    }
}

fn emit_human(outcome: &sync_lib::SyncOutcome) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(
        out,
        "Sync: {} added, {} updated, {} removed, {} unchanged",
        outcome.added.len(),
        outcome.updated.len(),
        outcome.removed.len(),
        outcome.leave_alones,
    )?;
    for change in &outcome.added {
        writeln!(
            out,
            "  + {} {:?} {}",
            change.harness,
            change.subsystem,
            change.path.display(),
        )?;
    }
    for change in &outcome.updated {
        writeln!(
            out,
            "  ~ {} {:?} {}",
            change.harness,
            change.subsystem,
            change.path.display(),
        )?;
    }
    for change in &outcome.removed {
        writeln!(
            out,
            "  - {} {:?} {}",
            change.harness,
            change.subsystem,
            change.path.display(),
        )?;
    }
    Ok(())
}

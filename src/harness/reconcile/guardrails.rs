//! Guardrails-prose reconciliation (Phase 6 / US3) — the GUARDRAILS sink.
//!
//! Extracted verbatim from the `sync.rs` orchestrator in Phase 7 (FR-011, the
//! `reconcile/` decomposition). The logic is unchanged: this module owns the
//! one-pass guardrails reconciler plus its private helpers (the prepared-body
//! type, the target-path projection, and the `GuardrailsAction` → [`Action`]
//! mapping). It reuses the shared [`record_action`] bookkeeping the
//! orchestrator and the other sink reconcilers also call.
//!
//! See [`crate::harness::reconcile`] for the fixed sink order and the
//! first-error precedence the orchestrator enforces across the three sinks.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;

use crate::error::TomeError;
use crate::harness::reconcile::agents::record_action;
use crate::harness::sync::{Action, HarnessSnapshot, SyncDeps, SyncOutcome, SyncSubsystem};

// =====================================================================
// Guardrails reconciliation (Phase 6 / US3)
// =====================================================================

/// Result of the guardrails reconciliation pass. Mirrors the hooks/agents
/// reconciliation shape: a per-harness aggregate action map keyed on
/// `name()`, plus the FIRST failure encountered (forward progress).
pub(crate) struct GuardrailsReconciliation {
    pub(crate) actions: std::collections::HashMap<String, Action>,
    pub(crate) first_error: Option<TomeError>,
}

/// One enabled plugin's guardrails source (its `GUARDRAILS.md` body) plus the
/// `<catalog>:<plugin>` provenance key. Prepared once per sync, reused for
/// every harness target.
struct PreparedGuardrails {
    key: String,
    body: String,
}

/// Reconcile guardrails regions for every harness target (FR-011–FR-016,
/// FR-084).
///
/// Runs as one pass AFTER hooks and BEFORE agents (the fixed sink order). For
/// each harness's guardrails target — deduplicated by path so the shared
/// `AGENTS.md` is written once — the desired region set is the union of every
/// live harness contributing to that path, minus any plugin suppressed for
/// that harness (Claude Code suppression, FR-013). A non-live harness whose
/// target no path-sharing live harness wants has its regions removed.
///
/// The enabled-plugin enumeration + each plugin's `GUARDRAILS.md` body are
/// computed ONCE and shared across harnesses. A read/render/write failure for
/// one plugin/target is recorded on `first_error` but does not abort the pass
/// (FR-084 forward progress).
pub(crate) fn reconcile_guardrails(
    deps: &SyncDeps<'_>,
    effective_names: &HashSet<String>,
    snapshots: &[HarnessSnapshot],
    plugins_with_hooks_json: &HashSet<String>,
    outcome: &mut SyncOutcome,
) -> Result<GuardrailsReconciliation, TomeError> {
    use crate::harness::GuardrailsPlacement;

    let mut recon = GuardrailsReconciliation {
        actions: std::collections::HashMap::new(),
        first_error: None,
    };

    // Prepare every enabled plugin's GUARDRAILS.md body once (shared across
    // harnesses). An EXISTING-yet-unopenable DB propagates; a genuinely absent
    // DB means no enabled plugins (and thus removal-only reconciliation, which
    // still runs so orphaned regions are cleaned up).
    let conn = if deps.paths.index_db.exists() {
        Some(crate::index::open_read_only(&deps.paths.index_db)?)
    } else {
        None
    };
    let workspace = deps.workspace_name.as_str();
    let enabled = match &conn {
        Some(c) => crate::index::skills::enabled_plugins_for_workspace(c, workspace)?,
        None => Vec::new(),
    };

    let mut prepared: Vec<PreparedGuardrails> = Vec::new();
    if let Some(c) = &conn {
        for (catalog, plugin) in &enabled {
            let plugin_root = match crate::index::skills::plugin_root_dir(
                c, deps.paths, workspace, catalog, plugin,
            ) {
                Ok(p) => p,
                // Catalog cache evicted: no readable GUARDRAILS.md — skip
                // (its orphaned regions, if any, are removed by the absence
                // from `desired`).
                Err(_) => continue,
            };
            match crate::harness::guardrails::read_guardrails_source(&plugin_root) {
                Ok(Some(body)) => prepared.push(PreparedGuardrails {
                    key: crate::harness::guardrails::region_key(catalog, plugin),
                    body,
                }),
                Ok(None) => {}
                Err(e) => {
                    if recon.first_error.is_none() {
                        recon.first_error = Some(e);
                    }
                }
            }
        }
    }

    // Group snapshots by guardrails target path so a shared `AGENTS.md` is
    // reconciled once. The first snapshot for a path "owns" the recorded
    // action; the rest are LeftAlone.
    let mut processed: HashSet<PathBuf> = HashSet::new();

    for snap in snapshots {
        let target_path = guardrails_target_path(&snap.guardrails_target.placement);
        if !processed.insert(target_path.clone()) {
            // Another harness already reconciled this shared path.
            recon.actions.insert(snap.name.clone(), Action::LeftAlone);
            continue;
        }

        // Build the desired region map as the union across every harness that
        // shares this exact target path AND is in the effective list. Each
        // contributing harness applies its own suppression flag.
        let sharers: Vec<&HarnessSnapshot> = snapshots
            .iter()
            .filter(|s| guardrails_target_path(&s.guardrails_target.placement) == target_path)
            .collect();
        let any_live = sharers.iter().any(|s| effective_names.contains(&s.name));

        let mut desired: BTreeMap<String, String> = BTreeMap::new();
        if any_live {
            for sharer in &sharers {
                if !effective_names.contains(&sharer.name) {
                    continue;
                }
                let suppress = sharer.guardrails_target.suppress_if_hooks_present;
                for pg in &prepared {
                    if suppress && plugins_with_hooks_json.contains(&pg.key) {
                        continue;
                    }
                    desired.insert(pg.key.clone(), pg.body.clone());
                }
            }
        }
        // When no sharer is live, `desired` stays empty → removal of any
        // existing regions / deletion of a Cursor sibling.

        let result = match &snap.guardrails_target.placement {
            GuardrailsPlacement::InFileRegion { file } => {
                crate::harness::guardrails::reconcile_in_file_region(file, &desired)
            }
            GuardrailsPlacement::StandaloneSibling { file } => {
                crate::harness::guardrails::reconcile_standalone_sibling(file, &desired)
            }
        };

        let action = match result {
            Ok(ga) => {
                let action = guardrails_action_to_action(ga);
                if action != Action::LeftAlone {
                    record_action(
                        outcome,
                        &snap.name,
                        SyncSubsystem::Guardrails,
                        &target_path,
                        action,
                    );
                }
                action
            }
            Err(e) => {
                if recon.first_error.is_none() {
                    recon.first_error = Some(e);
                }
                Action::LeftAlone
            }
        };
        recon.actions.insert(snap.name.clone(), action);
    }

    Ok(recon)
}

/// Extract the on-disk path a guardrails placement targets.
fn guardrails_target_path(placement: &crate::harness::GuardrailsPlacement) -> PathBuf {
    match placement {
        crate::harness::GuardrailsPlacement::InFileRegion { file }
        | crate::harness::GuardrailsPlacement::StandaloneSibling { file } => file.clone(),
    }
}

/// Map a [`crate::harness::guardrails::GuardrailsAction`] to the sync
/// orchestrator's [`Action`].
fn guardrails_action_to_action(ga: crate::harness::guardrails::GuardrailsAction) -> Action {
    use crate::harness::guardrails::GuardrailsAction as G;
    match ga {
        G::Created => Action::Created,
        G::Updated => Action::Updated,
        G::Removed => Action::Removed,
        G::LeftAlone => Action::LeftAlone,
    }
}

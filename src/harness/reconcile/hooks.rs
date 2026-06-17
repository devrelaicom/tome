//! Real Claude Code hooks reconciliation (Phase 6 / US2) — the HOOKS sink.
//!
//! Extracted verbatim from the `sync.rs` orchestrator in Phase 7 (FR-011, the
//! `reconcile/` decomposition). The logic is unchanged: this module owns the
//! one-pass hooks reconciler plus its private helpers (the hooks-presence
//! enumerator and the per-harness merge/remove writers). It reuses the shared
//! [`record_action`](crate::harness::reconcile::record_action) bookkeeping the
//! orchestrator and the other sink reconcilers also call.
//!
//! The reconcile cluster holds the three per-sink reconcilers — hooks,
//! guardrails, agents — that the thin orchestrator runs in the **fixed sink
//! order hooks → guardrails → agents**. Hooks runs FIRST so the Claude Code
//! guardrails-suppression predicate reads the fresh hooks-presence set (FR-016)
//! rather than stale state. With forward progress more than one sink can fail
//! in a pass; the orchestrator surfaces failures in that same order
//! (**first-error precedence**: hooks 43/44 wins over guardrails 46 over agents
//! 45). See [`crate::harness::reconcile`] for the cluster-wide contract.
//!
//! ## Mass-delete safeguard
//!
//! The enabled-plugin enumeration opens the central DB read-only and
//! **propagates** the open error for an *existing* DB — it never `.ok()`-
//! swallows it. Swallowing would collapse the enabled set to empty and make the
//! removal path strip every owned hook entry from a live harness's
//! `settings.local.json`. Only a genuinely *absent* DB is treated as "no
//! enabled plugins". This is the single biggest behaviour-preservation risk of
//! the decomposition and is carried into this module verbatim.

use std::collections::HashSet;
use std::path::Path;

use crate::error::TomeError;
use crate::harness::reconcile::record_action;
use crate::harness::sync::{Action, HarnessSnapshot, SyncDeps, SyncOutcome, SyncSubsystem};

// =====================================================================
// Real-hooks reconciliation (Phase 6 / US2)
// =====================================================================

/// Result of the real-hooks reconciliation pass. Mirrors
/// [`AgentReconciliation`](crate::harness::reconcile::agents::AgentReconciliation):
/// a per-harness aggregate action map keyed on `name()`, plus the FIRST
/// failure encountered (forward progress).
pub(crate) struct HooksReconciliation {
    pub(crate) actions: std::collections::HashMap<String, Action>,
    pub(crate) first_error: Option<TomeError>,
    /// Phase 6 / US3 (FR-013, FR-016): the `<catalog>:<plugin>` keys of every
    /// enabled plugin that ships a `hooks/hooks.json`. Computed in the hooks
    /// pass (which runs FIRST) so the Claude Code guardrails suppression
    /// predicate never reads stale state. A plugin in this set has its
    /// `CLAUDE.md` guardrails region suppressed (real hooks supersede prose).
    pub(crate) plugins_with_hooks_json: HashSet<String>,
}

/// Reconcile real Claude Code hooks for every harness (FR-001–FR-006,
/// FR-084).
///
/// One pass after the rules/MCP loop, FIRST among the Phase 6 sinks:
///
/// * A live `RealJson` harness with a settings path gets every enabled
///   plugin's `hooks/hooks.json` read + path-rewritten + merged into its
///   `settings.local.json` (structural-match append, idempotent).
/// * A non-live `RealJson` harness has every enabled plugin's rewritten
///   entries removed from `settings.local.json` (the project no longer wants
///   Claude Code, so Tome cleans up the hooks it can prove it owns).
/// * A `GuardrailsOnly` harness (`hook_settings_path == None` after the
///   strategy gate) is a no-op — the guardrails fallback is US3.
///
/// The enabled-plugin enumeration + each plugin's rewritten entries are
/// computed ONCE per sync and shared across every participating harness. A
/// malformed `hooks.json` (exit 43) or a settings write failure (exit 44)
/// for one plugin/harness is recorded but does not abort the pass (FR-084
/// forward progress): sibling plugins/harnesses still reconcile.
pub(crate) fn reconcile_hooks(
    deps: &SyncDeps<'_>,
    effective_names: &HashSet<String>,
    snapshots: &[HarnessSnapshot],
    outcome: &mut SyncOutcome,
) -> Result<HooksReconciliation, TomeError> {
    let mut recon = HooksReconciliation {
        actions: std::collections::HashMap::new(),
        first_error: None,
        plugins_with_hooks_json: HashSet::new(),
    };

    // The hooks-presence set drives the Claude Code guardrails suppression
    // predicate (FR-013/FR-016), so it must be computed even when NO harness
    // participates in real hooks (e.g. claude-code is GuardrailsOnly in a
    // synthetic registry) — the guardrails pass still needs it. It is
    // independent of the merge/remove work below.
    //
    // SEC-2: this `unwrap_or_default` is an INTENTIONAL exception to the
    // propagate-on-existing-DB rule the reconcilers follow. Collapsing an
    // unopenable DB to empty here only un-suppresses guardrails — Claude Code
    // renders one extra (redundant) prose region, corrected on the next
    // successful sync. That is fail-SAFE, unlike `reconcile_agents` where an
    // empty enabled set would mass-delete state (fail-dangerous), so there the
    // same error MUST propagate.
    recon.plugins_with_hooks_json = compute_plugins_with_hooks_json(deps).unwrap_or_default();

    // Fast exit: no harness participates in real hooks → no merge/remove work.
    // (The hooks-presence set above is still populated for guardrails.)
    if !snapshots.iter().any(|s| s.hook_settings_path.is_some()) {
        return Ok(recon);
    }

    // Open the central DB read-only to enumerate enabled plugins. A genuinely
    // absent DB means no enabled plugins (no hooks to merge, nothing owned to
    // remove). An EXISTING-yet-unopenable DB must PROPAGATE its error here,
    // before any settings write — never collapse to an empty list, which
    // would make the removal path strip every owned hook for a live harness.
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

    // Read + rewrite each enabled plugin's hooks ONCE. A parse failure is
    // recorded on the forward-progress `first_error`; the plugin is skipped
    // (its sibling plugins still reconcile, loud-but-isolated). Plugins with
    // no `hooks/hooks.json` contribute nothing.
    let mut prepared: Vec<crate::harness::hooks::RewrittenHooks> = Vec::new();
    if let Some(c) = &conn {
        for (catalog, plugin) in &enabled {
            let plugin_root = match crate::index::skills::plugin_root_dir(
                c, deps.paths, workspace, catalog, plugin,
            ) {
                Ok(p) => p,
                // STALE-REMOVAL GAP (R2-1), arm (a): catalog cache evicted.
                // A plugin whose on-disk root cannot be resolved has no
                // readable hooks — skip it rather than fail the whole sync.
                //
                // The same gap exists in arm (b) below: when the plugin still
                // exists but its `hooks/hooks.json` is gone, `read_rewritten_
                // entries` returns `Ok(None)` and the plugin is likewise dropped
                // from `prepared`. In BOTH arms, ownership is structural
                // re-derivation with no sidecar (NFR-003): removal of a
                // previously-written `settings.local.json` entry needs the
                // source to re-derive the deep-equal entry. So if claude-code
                // later goes non-live or the plugin is removed, those plugins'
                // earlier-written entries cannot be re-derived for removal and
                // persist in `settings.local.json`. There is no clean fix under
                // the no-sidecar model; the US5 doctor `HooksReport` is the
                // surfacing path for these orphaned entries.
                Err(_) => continue,
            };
            let plugin_data = deps.paths.plugin_data_dir_for(catalog, plugin);
            match crate::harness::hooks::read_rewritten_entries(&plugin_root, &plugin_data) {
                Ok(Some(hooks)) if !hooks.is_empty() => prepared.push(hooks),
                // Arm (b) of the stale-removal gap (see above): an enabled
                // plugin whose `hooks/hooks.json` is now absent (or empty) is
                // dropped here, and its previously-written entries cannot be
                // re-derived for removal — they persist until the US5 doctor
                // surfaces and reconciles them.
                Ok(_) => {}
                Err(e) => {
                    if recon.first_error.is_none() {
                        recon.first_error = Some(e);
                    }
                }
            }
        }
    }

    // The first Tome-OWNED hook: a SessionStart entry delivering the routing
    // directive on Claude Code. Pushed into `prepared` so the SAME merge (live)
    // / remove (non-live) machinery reconciles it. Reached only after the
    // fast-exit above, so it is added unconditionally only when a RealJson
    // harness participates; a harness going non-live has its entry removed by
    // structural re-derivation in `remove_hooks_for_harness`. The binary
    // reference is the bare `"tome"` string the MCP-config sync also uses (see
    // `harness::sync::write_mcp_for_harness`), keeping the spawned command
    // consistent.
    prepared.push(crate::harness::routing::session_start_hook(
        "tome", workspace,
    ));

    for snap in snapshots {
        let Some(settings_path) = &snap.hook_settings_path else {
            // GuardrailsOnly (or no settings path) → no-op for hooks.
            recon.actions.insert(snap.name.clone(), Action::LeftAlone);
            continue;
        };
        let is_live = effective_names.contains(&snap.name);
        let action = if is_live {
            merge_hooks_for_harness(&snap.name, settings_path, &prepared, outcome, &mut recon)
        } else {
            remove_hooks_for_harness(&snap.name, settings_path, &prepared, outcome, &mut recon)
        };
        recon.actions.insert(snap.name.clone(), action);
    }

    Ok(recon)
}

/// Compute the set of `<catalog>:<plugin>` keys for every enabled plugin in
/// the bound workspace that ships a `hooks/hooks.json` (FR-013/FR-016).
///
/// Existence of the file alone suppresses Claude Code's guardrails region —
/// a malformed `hooks.json` still counts as "ships hooks", so this check is
/// purely filesystem existence and never parses. A plugin whose on-disk root
/// cannot be resolved (catalog cache evicted) contributes nothing.
///
/// Returns `Ok(empty)` when the DB is genuinely absent; an EXISTING-yet-
/// unopenable DB propagates its error (the caller treats it as empty via
/// `unwrap_or_default`, which is safe: an unresolvable DB means we cannot
/// suppress, so guardrails render conservatively — the next sync corrects).
fn compute_plugins_with_hooks_json(deps: &SyncDeps<'_>) -> Result<HashSet<String>, TomeError> {
    let mut set = HashSet::new();
    if !deps.paths.index_db.exists() {
        return Ok(set);
    }
    let conn = crate::index::open_read_only(&deps.paths.index_db)?;
    let workspace = deps.workspace_name.as_str();
    let enabled = crate::index::skills::enabled_plugins_for_workspace(&conn, workspace)?;
    for (catalog, plugin) in &enabled {
        let plugin_root = match crate::index::skills::plugin_root_dir(
            &conn, deps.paths, workspace, catalog, plugin,
        ) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let hooks_json = plugin_root.join("hooks").join("hooks.json");
        if hooks_json.exists() {
            set.insert(crate::harness::guardrails::region_key(catalog, plugin));
        }
    }
    Ok(set)
}

/// Merge every prepared plugin's rewritten hooks into one live harness's
/// `settings.local.json`. Returns the aggregate [`Action`]. A write failure
/// for one plugin is recorded on `recon.first_error`; the rest still merge.
fn merge_hooks_for_harness(
    name: &str,
    settings_path: &Path,
    prepared: &[crate::harness::hooks::RewrittenHooks],
    outcome: &mut SyncOutcome,
    recon: &mut HooksReconciliation,
) -> Action {
    let pre_existed = settings_path.exists();
    let mut changed = false;
    for hooks in prepared {
        match crate::harness::hooks::merge_into_settings(settings_path, hooks) {
            Ok(true) => changed = true,
            Ok(false) => {}
            Err(e) => {
                if recon.first_error.is_none() {
                    recon.first_error = Some(e);
                }
            }
        }
    }
    if changed {
        let action = if pre_existed {
            Action::Updated
        } else {
            Action::Created
        };
        record_action(outcome, name, SyncSubsystem::Hooks, settings_path, action);
        action
    } else {
        Action::LeftAlone
    }
}

/// Remove every prepared plugin's rewritten hooks from one non-live
/// harness's `settings.local.json` (the harness left the effective list).
fn remove_hooks_for_harness(
    name: &str,
    settings_path: &Path,
    prepared: &[crate::harness::hooks::RewrittenHooks],
    outcome: &mut SyncOutcome,
    recon: &mut HooksReconciliation,
) -> Action {
    let mut changed = false;
    for hooks in prepared {
        match crate::harness::hooks::remove_from_settings(settings_path, hooks) {
            Ok(true) => changed = true,
            Ok(false) => {}
            Err(e) => {
                if recon.first_error.is_none() {
                    recon.first_error = Some(e);
                }
            }
        }
    }
    if changed {
        record_action(
            outcome,
            name,
            SyncSubsystem::Hooks,
            settings_path,
            Action::Removed,
        );
        Action::Removed
    } else {
        Action::LeftAlone
    }
}

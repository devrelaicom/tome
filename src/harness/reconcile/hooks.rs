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

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::{Map as JsonMap, Value as JsonValue};
use tempfile::NamedTempFile;

use crate::error::TomeError;
use crate::harness::hooks_ir::{
    CanonicalHook, Handler, HookManifest, ManifestEntry, PortableEvent, parse_canonical_hooks,
    read_manifest, write_manifest,
};
use crate::harness::reconcile::record_action;
use crate::harness::sync::{Action, HarnessSnapshot, SyncDeps, SyncOutcome, SyncSubsystem};
use crate::harness::{HookEvent, HookFileSpec, HookSupport, SessionSteering};

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

// =====================================================================
// Tome's own SessionStart routing hook for non-RealJson harnesses (Codex)
// =====================================================================

/// Reconcile Tome's OWN `SessionStart` routing hook for harnesses that expose a
/// [`HarnessModule::tome_session_hook_path`](crate::harness::HarnessModule::tome_session_hook_path)
/// but are NOT routed through the Claude-Code `RealJson` plugin-hooks pass
/// (currently: Codex → `<project>/.codex/hooks.json`). Carries ONLY Tome's hook
/// — never plugin hooks — so this never maps plugin hooks onto another harness.
///
/// Live harness → merge the Tome entry (structural-match, idempotent). Non-live
/// → remove the deep-equal Tome entry (re-derived, no sidecar). A write failure
/// for one harness is recorded on `first_error` (exit 44) and does not abort the
/// pass (forward progress).
pub(crate) fn reconcile_tome_session_hooks(
    deps: &SyncDeps<'_>,
    effective_names: &HashSet<String>,
    snapshots: &[HarnessSnapshot],
    outcome: &mut SyncOutcome,
) -> (std::collections::HashMap<String, Action>, Option<TomeError>) {
    let mut actions = std::collections::HashMap::new();
    let mut first_error: Option<TomeError> = None;
    let workspace = deps.workspace_name.as_str();

    for snap in snapshots {
        let Some(path) = &snap.tome_session_hook_path else {
            continue;
        };
        let entry = crate::harness::routing::codex_session_start_hook("tome", workspace);
        let is_live = effective_names.contains(&snap.name);
        let action = if is_live {
            let pre_existed = path.exists();
            match crate::harness::hooks::merge_into_settings(path, &entry) {
                Ok(true) => {
                    let a = if pre_existed {
                        Action::Updated
                    } else {
                        Action::Created
                    };
                    record_action(outcome, &snap.name, SyncSubsystem::Hooks, path, a);
                    a
                }
                Ok(false) => Action::LeftAlone,
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                    Action::LeftAlone
                }
            }
        } else {
            match crate::harness::hooks::remove_from_settings(path, &entry) {
                Ok(true) => {
                    record_action(
                        outcome,
                        &snap.name,
                        SyncSubsystem::Hooks,
                        path,
                        Action::Removed,
                    );
                    Action::Removed
                }
                Ok(false) => Action::LeftAlone,
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                    Action::LeftAlone
                }
            }
        };
        actions.insert(snap.name.clone(), action);
    }

    (actions, first_error)
}

// =====================================================================
// Phase 11 (G2): the CommandHook session-start reconciler (T017).
//
// A NEW reconciler for harnesses whose `session_steering()` is
// `CommandHook` — NEW harnesses ONLY. It writes a Tome-OWNED session-start
// hook entry (running `tome harness session-start --workspace <ws>
// --harness <name>`) into the spec's JSON hook file, preserving developer
// hooks, and removes ONLY that entry when the harness goes non-live.
//
// It DELIBERATELY excludes claude-code/codex: both keep `SessionSteering::
// None` and their dedicated Phase ≤10 session-hook path
// (`reconcile_hooks` / `reconcile_tome_session_hooks`), so this reconciler
// never sees them — their byte output is untouched. With every CURRENT
// module returning `None`, the fast-exit below makes this a no-op and the
// orchestrator output stays byte-identical.
//
// Ownership is structural deep-equal (no sidecar — the same model the rest
// of this module uses). The mass-delete safeguard does not apply: this
// reconciler needs no central-DB read (the directive command is the same
// regardless of enabled plugins), so there is no enabled set to collapse.
// =====================================================================

/// Reconcile Tome's OWN `CommandHook` session-start entry for every harness
/// whose [`SessionSteering`] is `CommandHook` (FR-014–FR-021, G2 / T017).
///
/// Live harness → merge the Tome-owned hook entry into the spec's file
/// (structural-match append, idempotent, developer hooks preserved). Non-live
/// → remove ONLY the deep-equal Tome entry (re-derived, no sidecar; a mismatch
/// is left in place). A write failure for one harness is recorded on
/// `first_error` (exit 44; malformed existing file → exit 43; symlink refusal →
/// exit 44 — PW6 parity with the Claude hook sink) and does NOT abort the pass
/// (forward progress).
///
/// Returns the per-harness aggregate action map (keyed on `name()`) plus the
/// first error. Wired into the orchestrator AFTER the Phase ≤10 hook passes so
/// it shares the `hooks_action` decision field; reuses the hooks error classes.
pub(crate) fn reconcile_command_hooks(
    deps: &SyncDeps<'_>,
    effective_names: &HashSet<String>,
    snapshots: &[HarnessSnapshot],
    project_root: &Path,
    outcome: &mut SyncOutcome,
) -> (std::collections::HashMap<String, Action>, Option<TomeError>) {
    let mut actions = std::collections::HashMap::new();
    let mut first_error: Option<TomeError> = None;

    // Fast exit: no harness uses `CommandHook` → no work, and (critically with
    // every current module `None`) the orchestrator output is byte-identical.
    if !snapshots
        .iter()
        .any(|s| matches!(s.session_steering, SessionSteering::CommandHook { .. }))
    {
        return (actions, first_error);
    }

    let workspace = deps.workspace_name.as_str();

    for snap in snapshots {
        let SessionSteering::CommandHook {
            file_spec, event, ..
        } = snap.session_steering
        else {
            continue;
        };

        let Some(path) = hook_file_path(file_spec, project_root) else {
            // `ClaudeSettingsLocal` / `CodexHooks` are Phase ≤10 sinks and are
            // unreachable through a NEW-harness `CommandHook` — skip defensively
            // rather than write to the legacy path (which this reconciler must
            // never own).
            continue;
        };

        let command = session_start_command(&snap.name, workspace);
        let is_live = effective_names.contains(&snap.name);
        let action = if is_live {
            merge_command_hook(
                &snap.name,
                &path,
                file_spec,
                event,
                &command,
                outcome,
                &mut first_error,
            )
        } else {
            remove_command_hook(
                &snap.name,
                &path,
                file_spec,
                event,
                &command,
                outcome,
                &mut first_error,
            )
        };
        actions.insert(snap.name.clone(), action);
    }

    (actions, first_error)
}

/// The Tome-owned session-start command string for a `CommandHook` harness.
/// Mirrors the bare-`"tome"` convention the MCP-config + claude/codex hooks use
/// (see `harness::sync::write_mcp_for_harness`), with the trailing `--harness
/// <name>` so the printer selects this harness's stdout envelope.
fn session_start_command(harness: &str, workspace: &str) -> String {
    format!("tome harness session-start --workspace {workspace} --harness {harness}")
}

/// Resolve the on-disk hook file for a [`HookFileSpec`] under `project_root`.
///
/// `CodexHooks` resolves to `<project>/.codex/hooks.json` (wired in US3.1 so the
/// plugin-hook dispatch reconciler can register `run-hook` entries there — the
/// session-steering `reconcile_command_hooks` still never reaches this arm,
/// since codex keeps [`SessionSteering::None`]). Only `ClaudeSettingsLocal`
/// returns `None` — that sink is the Claude `settings.local.json` reconciled by
/// `reconcile_hooks`, never through a `HookFileSpec` path.
fn hook_file_path(spec: HookFileSpec, project_root: &Path) -> Option<PathBuf> {
    let rel: &[&str] = match spec {
        HookFileSpec::DevinHooksV1 => &[".devin", "hooks.v1.json"],
        // Copilot's hooks are cross-surface under `.github/hooks/`; Tome owns a
        // dedicated `tome.json` there so it never collides with a developer's
        // own hook file.
        HookFileSpec::CopilotHooks => &[".github", "hooks", "tome.json"],
        HookFileSpec::GeminiSettings => &[".gemini", "settings.json"],
        HookFileSpec::AntigravityHooks => &[".agents", "hooks.json"],
        HookFileSpec::CursorHooks => &[".cursor", "hooks.json"],
        HookFileSpec::CodexHooks => &[".codex", "hooks.json"],
        // The Claude `settings.local.json` sink is reconciled by `reconcile_hooks`
        // (the `RealJson` plugin-hooks pass), never through a `HookFileSpec` path.
        HookFileSpec::ClaudeSettingsLocal => return None,
    };
    let mut path = project_root.to_path_buf();
    for seg in rel {
        path.push(seg);
    }
    Some(path)
}

/// Build the Tome-owned hook ENTRY object for a spec — the leaf the merge/remove
/// append/match by deep structural equality. The entry's exact bytes ARE the
/// ownership marker, so keep them stable (contract session-steering.md).
fn tome_hook_entry(spec: HookFileSpec, command: &str) -> JsonValue {
    match spec {
        // Devin: `{ "matcher": "", "hooks": [ { "type": "command",
        // "command": "…" } ] }`.
        HookFileSpec::DevinHooksV1 => serde_json::json!({
            "matcher": "",
            "hooks": [ { "type": "command", "command": command } ]
        }),
        // Copilot: `{ "type": "command", "command": "…" }`.
        HookFileSpec::CopilotHooks => serde_json::json!({
            "type": "command", "command": command
        }),
        // Gemini: `{ "hooks": [ { "name": "tome", "type": "command",
        // "command": "…" } ] }`.
        HookFileSpec::GeminiSettings => serde_json::json!({
            "hooks": [ { "name": "tome", "type": "command", "command": command } ]
        }),
        // Antigravity: `{ "type": "command", "command": "…" }` (list item under
        // the named `tome` block's event array).
        HookFileSpec::AntigravityHooks => serde_json::json!({
            "type": "command", "command": command
        }),
        // Cursor: `{ "type": "command", "command": "…" }`.
        HookFileSpec::CursorHooks => serde_json::json!({
            "type": "command", "command": command
        }),
        // Unreachable (filtered upstream); return a benign placeholder.
        HookFileSpec::ClaudeSettingsLocal | HookFileSpec::CodexHooks => JsonValue::Null,
    }
}

/// The Tome-owned `run-hook` dispatcher command for `(harness, event)` under
/// `workspace` (US3). The harness fires this on its native event; the
/// `--event <cc>` argument carries the CANONICAL CC event name (the manifest
/// key the dispatcher reads), so for a harness with a renamed native event
/// (e.g. gemini's `BeforeTool`) the file-key is native but the command names
/// the CC event. Mirrors the bare-`"tome"` convention used by the MCP + session
/// hooks; the trailing `--harness <name>` selects this harness's wire dialect.
fn run_hook_command(harness: &str, event_cc: &str, workspace: &str) -> String {
    format!("tome harness run-hook --event {event_cc} --harness {harness} --workspace {workspace}")
}

/// Build the Tome-owned MATCH-ALL `run-hook` dispatcher ENTRY for a spec — the
/// single registration leaf per used event. Distinct from
/// [`tome_hook_entry`] (the session-steering entry): the per-plugin matchers
/// live in the resolved manifest, applied by the dispatcher at runtime, so the
/// registered entry is always match-all. The entry's exact bytes ARE the
/// ownership marker (re-derived deep-equal), so keep them stable.
///
/// Match-all per wire: Devin/Codex use `"matcher": ""`; Cursor/Copilot omit the
/// matcher key (= all); Gemini wraps a named handler under `hooks` (the name
/// `tome-hook-dispatch` keeps it distinct from session-steering's `tome`).
fn tome_run_hook_entry(spec: HookFileSpec, command: &str) -> JsonValue {
    match spec {
        HookFileSpec::DevinHooksV1 | HookFileSpec::CodexHooks => serde_json::json!({
            "matcher": "",
            "hooks": [ { "type": "command", "command": command } ]
        }),
        HookFileSpec::GeminiSettings => serde_json::json!({
            "hooks": [ { "name": "tome-hook-dispatch", "type": "command", "command": command } ]
        }),
        HookFileSpec::CursorHooks | HookFileSpec::CopilotHooks => serde_json::json!({
            "type": "command", "command": command
        }),
        // Antigravity exposes no plugin-hook translation surface (rules-only),
        // and the Claude sink is reconciled elsewhere — both are unreachable for
        // a `hook_support()` harness; return a benign placeholder.
        HookFileSpec::AntigravityHooks | HookFileSpec::ClaudeSettingsLocal => JsonValue::Null,
    }
}

/// The JSON event-key string for a [`HookEvent`].
fn event_key(event: HookEvent) -> &'static str {
    match event {
        HookEvent::SessionStart => "SessionStart",
        HookEvent::PreInvocation => "PreInvocation",
    }
}

/// Per-spec event-key string for the SESSION-STEERING reconciler. Identical to
/// [`event_key`] for every spec except `CursorHooks`: Cursor uses camelCase
/// native event names (`sessionStart`), while every other `CommandHook` spec
/// uses the PascalCase CC event name that [`event_key`] returns.
fn effective_event_key(spec: HookFileSpec, event: HookEvent) -> &'static str {
    match (spec, event) {
        (HookFileSpec::CursorHooks, HookEvent::SessionStart) => "sessionStart",
        _ => event_key(event),
    }
}

/// Navigate (creating containers as needed) to the entry ARRAY a spec's Tome
/// entry lives in, returning a mutable borrow. Fails closed (exit 43) when an
/// existing container/array slot holds a wrong-typed value — never coerces a
/// developer's value (the fail-closed discipline the claude `append_if_absent`
/// uses). The path navigated per spec:
///
/// - Devin: `<root>[event]` (no wrapper).
/// - Copilot: `<root>.hooks[event]`.
/// - Gemini: `<root>.hooks[event]`.
/// - Antigravity: `<root>.tome[event]`.
/// - Cursor: `<root>.hooks[event]` (camelCase key via [`effective_event_key`]).
fn entry_array<'a>(
    doc: &'a mut JsonValue,
    spec: HookFileSpec,
    event: HookEvent,
    path: &Path,
) -> Result<&'a mut Vec<JsonValue>, TomeError> {
    let key = effective_event_key(spec, event);
    let root = doc
        .as_object_mut()
        .ok_or_else(|| TomeError::HookSpecParseError {
            path: path.to_path_buf(),
        })?;
    // Copilot CLI's hook document is `{ "version": 1, "hooks": { … } }` — the
    // T087 live-probe confirms Copilot CLI silently ignores a hook file that
    // omits the top-level `version`. Stamp it on the CREATE path (or any file
    // that lacks it) but NEVER overwrite a developer-set value: `or_insert`
    // only fills it when absent. Cursor also uses `{ "version": 1, "hooks": { … } }`.
    // Other specs (devin: no wrapper, gemini/antigravity: their own container key)
    // are untouched by this.
    if matches!(spec, HookFileSpec::CopilotHooks | HookFileSpec::CursorHooks) {
        root.entry("version".to_string())
            .or_insert(JsonValue::from(1));
    }
    // The intermediate container object (if any) the event array nests under.
    let container_key: Option<&str> = match spec {
        HookFileSpec::DevinHooksV1 => None,
        HookFileSpec::CopilotHooks | HookFileSpec::GeminiSettings | HookFileSpec::CursorHooks => {
            Some("hooks")
        }
        HookFileSpec::AntigravityHooks => Some("tome"),
        HookFileSpec::ClaudeSettingsLocal | HookFileSpec::CodexHooks => {
            return Err(TomeError::HookSpecParseError {
                path: path.to_path_buf(),
            });
        }
    };
    let event_holder: &mut JsonMap<String, JsonValue> = match container_key {
        None => root,
        Some(ck) => {
            let container = root
                .entry(ck.to_string())
                .or_insert_with(|| JsonValue::Object(JsonMap::new()));
            container
                .as_object_mut()
                .ok_or_else(|| TomeError::HookSpecParseError {
                    path: path.to_path_buf(),
                })?
        }
    };
    let arr = event_holder
        .entry(key.to_string())
        .or_insert_with(|| JsonValue::Array(Vec::new()));
    arr.as_array_mut()
        .ok_or_else(|| TomeError::HookSpecParseError {
            path: path.to_path_buf(),
        })
}

/// Look up (read-only) the event array for the remove path; `None` when any
/// container/array on the path is absent or wrong-typed (nothing to remove).
fn entry_array_opt(
    doc: &mut JsonValue,
    spec: HookFileSpec,
    event: HookEvent,
) -> Option<&mut Vec<JsonValue>> {
    let key = effective_event_key(spec, event);
    let root = doc.as_object_mut()?;
    let container_key: Option<&str> = match spec {
        HookFileSpec::DevinHooksV1 => None,
        HookFileSpec::CopilotHooks | HookFileSpec::GeminiSettings | HookFileSpec::CursorHooks => {
            Some("hooks")
        }
        HookFileSpec::AntigravityHooks => Some("tome"),
        HookFileSpec::ClaudeSettingsLocal | HookFileSpec::CodexHooks => return None,
    };
    let holder = match container_key {
        None => root,
        Some(ck) => root.get_mut(ck)?.as_object_mut()?,
    };
    holder.get_mut(key)?.as_array_mut()
}

/// The intermediate container object (if any) a `run-hook` entry nests under,
/// for the five `hook_support()` specs (US3). Devin nests at the document root;
/// Codex/Cursor/Copilot/Gemini nest under `"hooks"`. The two non-hook-support
/// specs are unreachable for a registering harness and fail closed (exit 43).
fn run_hook_container_key(
    spec: HookFileSpec,
    path: &Path,
) -> Result<Option<&'static str>, TomeError> {
    match spec {
        HookFileSpec::DevinHooksV1 => Ok(None),
        HookFileSpec::CodexHooks
        | HookFileSpec::CursorHooks
        | HookFileSpec::CopilotHooks
        | HookFileSpec::GeminiSettings => Ok(Some("hooks")),
        HookFileSpec::AntigravityHooks | HookFileSpec::ClaudeSettingsLocal => {
            Err(TomeError::HookSpecParseError {
                path: path.to_path_buf(),
            })
        }
    }
}

/// US3 sibling of [`entry_array`] keyed by the harness-NATIVE event-name STRING
/// (from `hook_event_name`) rather than the [`HookEvent`] enum, so the `run-hook`
/// reconciler registers under e.g. gemini's `BeforeTool` / cursor's `preToolUse`.
/// Navigates (creating containers as needed) to the entry array the run-hook
/// entry lives in and returns a mutable borrow. Generalizes container nesting
/// for `CodexHooks` (nests under `"hooks"`). Stamps the required top-level
/// `version: 1` for Copilot/Cursor (never overwriting a developer value). Fails
/// closed (exit 43) on a wrong-typed slot — never coerces a developer's value.
fn entry_array_by_key<'a>(
    doc: &'a mut JsonValue,
    spec: HookFileSpec,
    event_key: &str,
    path: &Path,
) -> Result<&'a mut Vec<JsonValue>, TomeError> {
    let root = doc
        .as_object_mut()
        .ok_or_else(|| TomeError::HookSpecParseError {
            path: path.to_path_buf(),
        })?;
    // Copilot CLI + Cursor require a top-level `version: 1` (T087 live-probe for
    // Copilot; Cursor's `{version:1, hooks:{…}}` shape). Stamp on create / any
    // file lacking it, but NEVER overwrite a developer-set value.
    if matches!(spec, HookFileSpec::CopilotHooks | HookFileSpec::CursorHooks) {
        root.entry("version".to_string())
            .or_insert(JsonValue::from(1));
    }
    let event_holder: &mut JsonMap<String, JsonValue> = match run_hook_container_key(spec, path)? {
        None => root,
        Some(ck) => {
            let container = root
                .entry(ck.to_string())
                .or_insert_with(|| JsonValue::Object(JsonMap::new()));
            container
                .as_object_mut()
                .ok_or_else(|| TomeError::HookSpecParseError {
                    path: path.to_path_buf(),
                })?
        }
    };
    let arr = event_holder
        .entry(event_key.to_string())
        .or_insert_with(|| JsonValue::Array(Vec::new()));
    arr.as_array_mut()
        .ok_or_else(|| TomeError::HookSpecParseError {
            path: path.to_path_buf(),
        })
}

/// Read-only sibling of [`entry_array_opt`] keyed by the native event-name
/// STRING (US3 remove path); `None` when any container/array on the path is
/// absent or wrong-typed (nothing to remove).
fn entry_array_opt_by_key<'a>(
    doc: &'a mut JsonValue,
    spec: HookFileSpec,
    event_key: &str,
) -> Option<&'a mut Vec<JsonValue>> {
    let root = doc.as_object_mut()?;
    let container_key = run_hook_container_key(spec, Path::new("")).ok()?;
    let holder = match container_key {
        None => root,
        Some(ck) => root.get_mut(ck)?.as_object_mut()?,
    };
    holder.get_mut(event_key)?.as_array_mut()
}

/// Drop the now-empty native-keyed event array a removed `run-hook` entry left
/// behind (US3). Unlike [`prune_empty`], the shared `"hooks"` container is NOT
/// dropped when empty: for codex the session hook
/// ([`reconcile_tome_session_hooks`]) co-owns `"hooks"`, and for the other specs
/// a developer may own sibling keys — an empty `"hooks": {}` is harmless.
fn prune_empty_by_key(doc: &mut JsonValue, spec: HookFileSpec, event_key: &str) {
    let Some(root) = doc.as_object_mut() else {
        return;
    };
    let Ok(container_key) = run_hook_container_key(spec, Path::new("")) else {
        return;
    };
    let holder = match container_key {
        None => root,
        Some(ck) => {
            let Some(h) = root.get_mut(ck).and_then(JsonValue::as_object_mut) else {
                return;
            };
            h
        }
    };
    if holder
        .get(event_key)
        .and_then(JsonValue::as_array)
        .is_some_and(|a| a.is_empty())
    {
        holder.shift_remove(event_key);
    }
}

/// Load a spec hook file, returning `(value, existed)`. Absent → a fresh empty
/// object with `existed = false`. A malformed existing file → exit 43; a
/// non-absent read failure (oversize, permissions, non-UTF-8) → exit 43 (the
/// "malformed or unparsable" class for this third-party hook file).
fn load_hook_file(path: &Path) -> Result<(JsonValue, bool), TomeError> {
    let body = match crate::util::bounded_read_to_string(path, crate::util::HARNESS_MCP_MAX) {
        Ok(b) => b,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok((JsonValue::Object(JsonMap::new()), false));
        }
        Err(_) => {
            return Err(TomeError::HookSpecParseError {
                path: path.to_path_buf(),
            });
        }
    };
    if body.trim().is_empty() {
        return Ok((JsonValue::Object(JsonMap::new()), true));
    }
    let value =
        serde_json::from_str::<JsonValue>(&body).map_err(|_| TomeError::HookSpecParseError {
            path: path.to_path_buf(),
        })?;
    if !value.is_object() {
        return Err(TomeError::HookSpecParseError {
            path: path.to_path_buf(),
        });
    }
    Ok((value, true))
}

/// Map a refused-symlinked-component error at a hook-file sink to
/// [`TomeError::HookSettingsWriteFailed`] (exit 44) — PW6 exit-code parity with
/// the Claude hook sink (the P6 7→44 precedent). A symlinked component on a hook
/// path is a write-side refusal, so it shares the Claude sink's exit code rather
/// than the generic `Io` (7).
fn hook_symlink_refusal(path: &Path, e: std::io::Error) -> TomeError {
    TomeError::HookSettingsWriteFailed {
        path: path.to_path_buf(),
        source: e,
    }
}

/// Atomic, symlink-refusing, parent-creating write of a spec hook file. Symlink
/// refusal AND every other write failure map to `HookSettingsWriteFailed`
/// (exit 44) — PW6 exit-code parity with the Claude hook sink (the P6 7→44
/// precedent). Mirrors `harness::hooks::write_settings` / `mcp_config::atomic_write`.
pub(crate) fn write_hook_file(path: &Path, doc: &JsonValue) -> Result<(), TomeError> {
    // Symlink refusal on the write path → `HookSettingsWriteFailed` (exit 44),
    // the same code the Claude hook sink uses for a refused symlinked component.
    crate::util::refuse_symlinked_component(path).map_err(|e| hook_symlink_refusal(path, e))?;

    let mut bytes =
        serde_json::to_vec_pretty(doc).map_err(|e| TomeError::HookSettingsWriteFailed {
            path: path.to_path_buf(),
            source: std::io::Error::other(e),
        })?;
    bytes.push(b'\n');

    let parent = path
        .parent()
        .ok_or_else(|| TomeError::HookSettingsWriteFailed {
            path: path.to_path_buf(),
            source: std::io::Error::other("hook file path has no parent"),
        })?;
    let parent_existed = parent.exists();
    let wf = |e: std::io::Error| TomeError::HookSettingsWriteFailed {
        path: path.to_path_buf(),
        source: e,
    };
    std::fs::create_dir_all(parent).map_err(wf)?;
    #[cfg(unix)]
    if !parent_existed {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700)).map_err(wf)?;
    }
    #[cfg(not(unix))]
    let _ = parent_existed;

    #[cfg(unix)]
    let target_mode: Option<u32> = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::symlink_metadata(path)
            .ok()
            .map(|m| m.permissions().mode())
    };

    let mut tmp = NamedTempFile::with_prefix_in(".tome.tmp.", parent).map_err(wf)?;
    tmp.write_all(&bytes).map_err(wf)?;
    tmp.as_file().sync_all().map_err(wf)?;
    #[cfg(unix)]
    if let Some(mode) = target_mode {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(mode)).map_err(wf)?;
    }
    tmp.persist(path).map_err(|e| wf(e.error))?;
    Ok(())
}

/// Merge the Tome-owned hook entry into a live harness's spec file, appending
/// only when no deep-equal entry is already present (idempotent; developer
/// hooks preserved). Returns the aggregate [`Action`].
fn merge_command_hook(
    name: &str,
    path: &Path,
    spec: HookFileSpec,
    event: HookEvent,
    command: &str,
    outcome: &mut SyncOutcome,
    first_error: &mut Option<TomeError>,
) -> Action {
    // Refuse a symlinked component up front (read parity with the write path).
    // PW6: a symlink refusal at this hook sink shares the Claude sink's exit 44.
    if let Err(e) =
        crate::util::refuse_symlinked_component(path).map_err(|e| hook_symlink_refusal(path, e))
    {
        if first_error.is_none() {
            *first_error = Some(e);
        }
        return Action::LeftAlone;
    }

    let (mut doc, existed) = match load_hook_file(path) {
        Ok(v) => v,
        Err(e) => {
            if first_error.is_none() {
                *first_error = Some(e);
            }
            return Action::LeftAlone;
        }
    };

    let entry = tome_hook_entry(spec, command);
    let changed = {
        let arr = match entry_array(&mut doc, spec, event, path) {
            Ok(a) => a,
            Err(e) => {
                if first_error.is_none() {
                    *first_error = Some(e);
                }
                return Action::LeftAlone;
            }
        };
        if arr.contains(&entry) {
            false
        } else {
            arr.push(entry);
            true
        }
    };

    if !changed {
        return Action::LeftAlone;
    }
    if let Err(e) = write_hook_file(path, &doc) {
        if first_error.is_none() {
            *first_error = Some(e);
        }
        return Action::LeftAlone;
    }
    let action = if existed {
        Action::Updated
    } else {
        Action::Created
    };
    record_action(outcome, name, SyncSubsystem::Hooks, path, action);
    action
}

/// Remove the deep-equal Tome-owned hook entry from a non-live harness's spec
/// file (structural match only; a mismatch / absent file is a no-op). After the
/// removal the now-empty event array is pruned so an empty Tome block doesn't
/// linger.
fn remove_command_hook(
    name: &str,
    path: &Path,
    spec: HookFileSpec,
    event: HookEvent,
    command: &str,
    outcome: &mut SyncOutcome,
    first_error: &mut Option<TomeError>,
) -> Action {
    // PW6: a symlink refusal at this hook sink shares the Claude sink's exit 44.
    if let Err(e) =
        crate::util::refuse_symlinked_component(path).map_err(|e| hook_symlink_refusal(path, e))
    {
        if first_error.is_none() {
            *first_error = Some(e);
        }
        return Action::LeftAlone;
    }

    let (mut doc, existed) = match load_hook_file(path) {
        Ok(v) => v,
        Err(e) => {
            if first_error.is_none() {
                *first_error = Some(e);
            }
            return Action::LeftAlone;
        }
    };
    if !existed {
        return Action::LeftAlone;
    }

    let entry = tome_hook_entry(spec, command);
    let changed = match entry_array_opt(&mut doc, spec, event) {
        Some(arr) => {
            let before = arr.len();
            arr.retain(|existing| *existing != entry);
            before != arr.len()
        }
        None => false,
    };

    if !changed {
        return Action::LeftAlone;
    }

    // Prune the now-empty event array (and the named `tome` container for
    // antigravity) so removal leaves no empty Tome scaffolding behind. Best
    // effort: a failed navigation here just leaves an empty array, harmless.
    prune_empty(&mut doc, spec, event);

    if let Err(e) = write_hook_file(path, &doc) {
        if first_error.is_none() {
            *first_error = Some(e);
        }
        return Action::LeftAlone;
    }
    record_action(outcome, name, SyncSubsystem::Hooks, path, Action::Removed);
    Action::Removed
}

/// Drop the now-empty event array so a removed Tome hook leaves no scaffolding.
/// This prunes the container the spec nests under: the bare root for devin, the
/// `hooks` container for copilot-cli, gemini, and cursor, and the named `tome`
/// container for antigravity (additionally dropped once empty). Uses
/// [`effective_event_key`] so the Cursor `sessionStart` (camelCase) key is
/// pruned correctly.
fn prune_empty(doc: &mut JsonValue, spec: HookFileSpec, event: HookEvent) {
    let key = effective_event_key(spec, event);
    let Some(root) = doc.as_object_mut() else {
        return;
    };
    let container_key: Option<&str> = match spec {
        HookFileSpec::DevinHooksV1 => None,
        HookFileSpec::CopilotHooks | HookFileSpec::GeminiSettings | HookFileSpec::CursorHooks => {
            Some("hooks")
        }
        HookFileSpec::AntigravityHooks => Some("tome"),
        HookFileSpec::ClaudeSettingsLocal | HookFileSpec::CodexHooks => return,
    };
    match container_key {
        None => {
            if root
                .get(key)
                .and_then(JsonValue::as_array)
                .is_some_and(|a| a.is_empty())
            {
                root.shift_remove(key);
            }
        }
        Some(ck) => {
            if let Some(holder) = root.get_mut(ck).and_then(JsonValue::as_object_mut) {
                if holder
                    .get(key)
                    .and_then(JsonValue::as_array)
                    .is_some_and(|a| a.is_empty())
                {
                    holder.shift_remove(key);
                }
                // Antigravity's named `tome` block: drop it when it's now empty
                // (its only content is Tome's own event).
                let now_empty = holder.is_empty();
                if now_empty {
                    root.shift_remove(ck);
                }
            }
        }
    }
}

// =====================================================================
// US3 — plugin-hook dispatch registration + manifest write (sync-time).
//
// For every in-scope harness that declares a `hook_support()` capability,
// register a Tome-owned MATCH-ALL `run-hook` dispatcher entry into the harness's
// native hook file (one per USED event — an event ≥1 enabled plugin's hook
// targets) AND write the resolved per-(workspace, harness) `hooks-manifest.json`
// the runtime dispatcher reads. A non-live harness has its run-hook entries
// removed (structural-equal) + its manifest deleted.
//
// Ownership is structural deep-equal (no sidecar), mirroring the session-steering
// `reconcile_command_hooks` above: the `run-hook` entry is a SEPARATE leaf from
// the session-start entry (a `BeforeTool`/`preToolUse`/… event key vs
// `SessionStart`), so the two compose additively and neither clobbers the other.
//
// The enabled-plugin enumeration follows `reconcile_hooks`' mass-delete
// safeguard: a genuinely ABSENT central DB means "no enabled plugins", but an
// EXISTING-yet-unopenable DB PROPAGATES its error (collapsing it to empty would
// strip every live harness's manifest + registered entries).
// =====================================================================

/// Reconcile Tome's plugin-hook DISPATCH registration for every harness that
/// declares a [`HookSupport`] (US3.2). Returns the per-harness aggregate action
/// map (keyed on `name()`) plus the first error (forward progress).
///
/// Wired into the orchestrator AFTER `reconcile_command_hooks` so it shares the
/// `hooks_action` decision field and reuses the hook error classes (exit 43
/// parse / 44 write).
//
pub(crate) fn reconcile_plugin_hook_dispatch(
    deps: &SyncDeps<'_>,
    cfg: &crate::config::Config,
    effective_names: &HashSet<String>,
    snapshots: &[HarnessSnapshot],
    project_root: &Path,
    outcome: &mut SyncOutcome,
) -> (std::collections::HashMap<String, Action>, Option<TomeError>) {
    let mut actions = std::collections::HashMap::new();
    let mut first_error: Option<TomeError> = None;

    // Fast exit: no in-scope harness declares hook support → no work. With every
    // GuardrailsOnly harness this keeps the orchestrator output byte-identical.
    if !snapshots.iter().any(|s| s.hook_support.is_some()) {
        return (actions, first_error);
    }

    // US8.2: resolve the workspace-level `raw_event_passthrough` flag once,
    // shared across all harnesses in this pass. Fail-open: any settings read
    // error → flag is `false` (passthrough off, never a block).
    let raw_event_passthrough: bool =
        crate::settings::scopes::load_workspace_settings(deps.paths, deps.workspace_name)
            .ok()
            .flatten()
            .and_then(|w| w.raw_event_passthrough)
            .unwrap_or(false);

    // US6.1: opt-out gate. When `translate_plugin_hooks = false`, treat every
    // harness as non-live so any previously written run-hook entries + manifests
    // are removed (same teardown path as a non-live harness). Toggling off is
    // a clean, reversible operation — the next sync with the flag re-enabled
    // (or absent, which defaults to true) re-writes everything from scratch.
    if !cfg.hooks.translate_plugin_hooks.unwrap_or(true) {
        for snap in snapshots {
            let Some(support) = &snap.hook_support else {
                continue;
            };
            let action = reconcile_one_harness_dispatch(
                deps,
                project_root,
                snap,
                support,
                &[],   // empty canonical → no hooks to register
                false, // is_live=false → teardown path
                outcome,
                &mut first_error,
                cfg,
                raw_event_passthrough,
            );
            actions.insert(snap.name.clone(), action);
        }
        return (actions, first_error);
    }

    // Resolve the enabled plugins' canonical hooks ONCE, shared across harnesses.
    // A hard DB-open error PROPAGATES (mass-delete guard); a per-plugin parse
    // failure is recorded on `first_error` (forward progress) and that plugin is
    // skipped.
    let canonical = match resolve_enabled_canonical_hooks(deps, &mut first_error) {
        Ok(c) => c,
        Err(e) => return (actions, Some(e)),
    };

    for snap in snapshots {
        let Some(support) = &snap.hook_support else {
            continue;
        };
        let is_live = effective_names.contains(&snap.name);
        let action = reconcile_one_harness_dispatch(
            deps,
            project_root,
            snap,
            support,
            &canonical,
            is_live,
            outcome,
            &mut first_error,
            cfg,
            raw_event_passthrough,
        );
        actions.insert(snap.name.clone(), action);
    }

    (actions, first_error)
}

/// Resolve every enabled plugin's typed [`CanonicalHook`]s for the bound
/// workspace, reading the central DB READ-ONLY (US3.2).
///
/// Mass-delete safeguard (mirrors [`reconcile_hooks`]): a genuinely ABSENT DB is
/// `Ok(empty)`; an EXISTING-yet-unopenable DB PROPAGATES via `Err` (the caller
/// aborts the pass — collapsing to empty would mass-delete live manifests). A
/// per-plugin malformed `hooks.json` (exit 43) is recorded on `first_error`
/// (forward progress) and the plugin is skipped; a plugin whose root can't be
/// resolved (catalog cache evicted) is likewise skipped.
fn resolve_enabled_canonical_hooks(
    deps: &SyncDeps<'_>,
    first_error: &mut Option<TomeError>,
) -> Result<Vec<CanonicalHook>, TomeError> {
    if !deps.paths.index_db.exists() {
        return Ok(Vec::new());
    }
    let conn = crate::index::open_read_only(&deps.paths.index_db)?;
    let workspace = deps.workspace_name.as_str();
    let enabled = crate::index::skills::enabled_plugins_for_workspace(&conn, workspace)?;

    let mut out = Vec::new();
    // Collected for the US11 doctor surfacing; US3 needs them collected, not
    // rendered (a non-portable event / unsupported handler keeps its GUARDRAILS
    // floor).
    let mut drops = Vec::new();
    for (catalog, plugin) in &enabled {
        let plugin_root = match crate::index::skills::plugin_root_dir(
            &conn, deps.paths, workspace, catalog, plugin,
        ) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let plugin_data = deps.paths.plugin_data_dir_for(catalog, plugin);
        match crate::harness::hooks::read_rewritten_entries(&plugin_root, &plugin_data) {
            Ok(Some(rewritten)) => {
                // Fix 1 (US8 review): bake the DB-resolved plugin_root into every
                // hook at this site (the only place it is available). `to_string_lossy`
                // handles non-UTF-8 paths safely — this is a provenance field, not
                // an executed command, so U+FFFD replacement is acceptable.
                let root_str = plugin_root.to_string_lossy().into_owned();
                let mut hooks = parse_canonical_hooks(catalog, plugin, &rewritten, &mut drops);
                for h in &mut hooks {
                    h.plugin_root = root_str.clone();
                }
                out.extend(hooks);
            }
            Ok(None) => {}
            Err(e) => {
                // Forward progress: record once, skip this plugin, keep going.
                if first_error.is_none() {
                    *first_error = Some(e);
                }
            }
        }
    }
    let _ = drops;
    Ok(out)
}

/// Reconcile the run-hook registration + manifest for ONE harness (US3.2).
#[allow(clippy::too_many_arguments)]
fn reconcile_one_harness_dispatch(
    deps: &SyncDeps<'_>,
    project_root: &Path,
    snap: &HarnessSnapshot,
    support: &HookSupport,
    canonical: &[CanonicalHook],
    is_live: bool,
    outcome: &mut SyncOutcome,
    first_error: &mut Option<TomeError>,
    cfg: &crate::config::Config,
    raw_event_passthrough: bool,
) -> Action {
    let workspace = deps.workspace_name.as_str();
    let manifest_path = deps.paths.hooks_manifest(deps.workspace_name, &snap.name);
    let Some(hook_path) = hook_file_path(support.file_spec, project_root) else {
        // Only `ClaudeSettingsLocal` returns `None`, and no `hook_support()`
        // harness uses it — skip defensively.
        return Action::LeftAlone;
    };

    // US6.2 prompt gate: when neither prompt_provider nor prompt_model is
    // configured, exclude Handler::Prompt entries from the manifest. The
    // HookDropReason::PromptDisabled variant is available for the US11 doctor
    // to surface; for now the drops are silently elided (matches the existing
    // `let _ = drops;` pattern in resolve_enabled_canonical_hooks).
    let prompt_enabled = cfg.hooks.prompt_provider.is_some() || cfg.hooks.prompt_model.is_some();
    let filtered: Vec<CanonicalHook>;
    let effective_canonical: &[CanonicalHook] = if prompt_enabled {
        canonical
    } else {
        filtered = canonical
            .iter()
            .filter(|h| !matches!(h.handler, Handler::Prompt { .. }))
            .cloned()
            .collect();
        &filtered
    };

    // The USED events: events in this harness's support set that ≥1 enabled
    // canonical hook targets. For a non-live harness the desired set is empty
    // (every Tome run-hook entry is removed + the manifest deleted).
    let used: Vec<PortableEvent> = if is_live {
        support
            .events
            .iter()
            .copied()
            .filter(|ev| effective_canonical.iter().any(|h| h.event == *ev))
            .collect()
    } else {
        Vec::new()
    };

    let hook_action = reconcile_dispatch_hook_file(
        &snap.name,
        &hook_path,
        support,
        &snap.hook_event_names,
        workspace,
        &used,
        outcome,
        first_error,
    );
    let manifest_action = reconcile_dispatch_manifest(
        &snap.name,
        &manifest_path,
        effective_canonical,
        &used,
        is_live,
        outcome,
        first_error,
        raw_event_passthrough,
    );
    stronger(hook_action, manifest_action)
}

/// Merge / prune Tome's per-event `run-hook` registration entries in the
/// harness's native hook file. Ensure-PRESENT for each used event; ensure-ABSENT
/// for each supported-but-unused event (pruning a previously-registered event).
/// Structural deep-equal append/removal — developer hooks are preserved.
#[allow(clippy::too_many_arguments)]
fn reconcile_dispatch_hook_file(
    name: &str,
    path: &Path,
    support: &HookSupport,
    event_names: &[(PortableEvent, &'static str)],
    workspace: &str,
    used: &[PortableEvent],
    outcome: &mut SyncOutcome,
    first_error: &mut Option<TomeError>,
) -> Action {
    // Refuse a symlinked component up front (PW6 exit-44 parity with the Claude /
    // command-hook sinks).
    if let Err(e) =
        crate::util::refuse_symlinked_component(path).map_err(|e| hook_symlink_refusal(path, e))
    {
        if first_error.is_none() {
            *first_error = Some(e);
        }
        return Action::LeftAlone;
    }
    let (mut doc, existed) = match load_hook_file(path) {
        Ok(v) => v,
        Err(e) => {
            if first_error.is_none() {
                *first_error = Some(e);
            }
            return Action::LeftAlone;
        }
    };

    // Fix 2: track whether entry_array_by_key will stamp `version:1` on a
    // Copilot/Cursor file that currently lacks it. That stamp is a real
    // on-disk mutation even when no run-hook entry is added or removed.
    let version_absent_before = matches!(
        support.file_spec,
        HookFileSpec::CopilotHooks | HookFileSpec::CursorHooks
    ) && doc
        .as_object()
        .map(|o| !o.contains_key("version"))
        .unwrap_or(false);

    let mut added_any = false;
    let mut removed_any = false;
    for &event in support.events {
        let Some(native) = event_names
            .iter()
            .find(|(e, _)| *e == event)
            .map(|(_, n)| *n)
        else {
            continue;
        };
        let command = run_hook_command(name, event.cc_name(), workspace);
        let entry = tome_run_hook_entry(support.file_spec, &command);
        if used.contains(&event) {
            let arr = match entry_array_by_key(&mut doc, support.file_spec, native, path) {
                Ok(a) => a,
                Err(e) => {
                    if first_error.is_none() {
                        *first_error = Some(e);
                    }
                    return Action::LeftAlone;
                }
            };
            if !arr.contains(&entry) {
                arr.push(entry);
                added_any = true;
            }
        } else {
            // Ensure-absent: strip a stale Tome entry (structural-equal), then
            // prune the now-empty event array. Scoped so the `arr` borrow ends
            // before the prune re-borrows `doc`.
            let removed_this =
                if let Some(arr) = entry_array_opt_by_key(&mut doc, support.file_spec, native) {
                    let before = arr.len();
                    arr.retain(|existing| *existing != entry);
                    before != arr.len()
                } else {
                    false
                };
            if removed_this {
                removed_any = true;
                prune_empty_by_key(&mut doc, support.file_spec, native);
            }
        }
    }

    // version_stamped is true when entry_array_by_key wrote `version:1` into
    // a Copilot/Cursor doc that lacked it. This is a real mutation that must
    // be flushed even when no run-hook entry changed.
    let version_stamped = version_absent_before
        && doc
            .as_object()
            .map(|o| o.contains_key("version"))
            .unwrap_or(false);

    if !added_any && !removed_any && !version_stamped {
        return Action::LeftAlone;
    }
    if let Err(e) = write_hook_file(path, &doc) {
        if first_error.is_none() {
            *first_error = Some(e);
        }
        return Action::LeftAlone;
    }
    let action = if !existed {
        Action::Created
    } else if added_any || version_stamped {
        Action::Updated
    } else {
        Action::Removed
    };
    record_action(outcome, name, SyncSubsystem::Hooks, path, action);
    action
}

/// Build + write (or remove) the resolved per-(workspace, harness) dispatch
/// manifest. Keyed by the CC event name; per-plugin matcher / `if` carried
/// verbatim; `timeout_ms` = CC-seconds × 1000 (the dispatcher always reads ms,
/// regardless of the harness's registration `timeout_unit`). An empty desired
/// set (non-live, or no enabled hook targets a supported event) removes a stale
/// manifest. Idempotent: a byte-equal manifest is left untouched.
///
/// `raw_event_passthrough` is the workspace-level US8.2 flag; when `true` the
/// dispatcher will include the original harness payload verbatim as
/// `tome.raw_event` in the synthesized CC stdin (default `false`).
#[allow(clippy::too_many_arguments)]
fn reconcile_dispatch_manifest(
    name: &str,
    path: &Path,
    canonical: &[CanonicalHook],
    used: &[PortableEvent],
    is_live: bool,
    outcome: &mut SyncOutcome,
    first_error: &mut Option<TomeError>,
    raw_event_passthrough: bool,
) -> Action {
    let mut events: BTreeMap<String, Vec<ManifestEntry>> = BTreeMap::new();
    if is_live {
        for hook in canonical {
            if !used.contains(&hook.event) {
                continue;
            }
            events
                .entry(hook.event.cc_name().to_string())
                .or_default()
                .push(ManifestEntry {
                    plugin: format!("{}:{}", hook.catalog, hook.plugin),
                    // Fix 1 (US8 review): bake the resolved install root into the
                    // manifest so the hot-path dispatcher reads it directly and
                    // never re-derives it from the plugin-data path.
                    plugin_root: if hook.plugin_root.is_empty() {
                        None
                    } else {
                        Some(hook.plugin_root.clone())
                    },
                    matcher: hook.matcher.clone(),
                    if_pred: hook.if_pred.clone(),
                    handler: hook.handler.clone(),
                    // Manifest timeout is ALWAYS ms = CC-seconds × 1000; the
                    // harness's `timeout_unit` only governs per-plugin timeouts
                    // written INTO a harness hook file, which the match-all
                    // run-hook registration never carries.
                    timeout_ms: hook.timeout_secs.map(|s| s.saturating_mul(1000)),
                    cwd: None,
                    env: BTreeMap::new(),
                });
        }
    }

    let manifest_existed = path.exists();

    if events.is_empty() {
        // No dispatch needed → remove a stale manifest if present.
        if !manifest_existed {
            return Action::LeftAlone;
        }
        // Guard against symlink attacks before removing (PW6 exit-44 parity).
        if let Err(e) =
            crate::util::refuse_symlinked_component(path).map_err(|e| hook_symlink_refusal(path, e))
        {
            if first_error.is_none() {
                *first_error = Some(e);
            }
            return Action::LeftAlone;
        }
        match std::fs::remove_file(path) {
            Ok(()) => {
                record_action(outcome, name, SyncSubsystem::Hooks, path, Action::Removed);
                Action::Removed
            }
            Err(e) => {
                if first_error.is_none() {
                    *first_error = Some(TomeError::HookSettingsWriteFailed {
                        path: path.to_path_buf(),
                        source: e,
                    });
                }
                Action::LeftAlone
            }
        }
    } else {
        let manifest = HookManifest {
            harness: name.to_string(),
            raw_event_passthrough,
            events,
        };
        // Idempotent: only write when the on-disk manifest differs (a malformed /
        // unreadable Tome-owned manifest is healed by overwriting).
        let need_write = if manifest_existed {
            !read_manifest(path).is_ok_and(|existing| existing == manifest)
        } else {
            true
        };
        if !need_write {
            return Action::LeftAlone;
        }
        if let Err(e) = write_manifest(path, &manifest) {
            if first_error.is_none() {
                *first_error = Some(e);
            }
            return Action::LeftAlone;
        }
        let action = if manifest_existed {
            Action::Updated
        } else {
            Action::Created
        };
        record_action(outcome, name, SyncSubsystem::Hooks, path, action);
        action
    }
}

/// The "stronger" of two aggregate sink actions for the composed
/// `hooks_action`: `Created` > `Updated` > `Removed` > `LeftAlone`.
fn stronger(a: Action, b: Action) -> Action {
    fn rank(x: Action) -> u8 {
        match x {
            Action::Created => 3,
            Action::Updated => 2,
            Action::Removed => 1,
            Action::LeftAlone => 0,
        }
    }
    if rank(a) >= rank(b) { a } else { b }
}

#[cfg(test)]
mod command_hook_tests {
    use super::*;
    use crate::harness::{Envelope, HookEvent, HookFileSpec, SessionSteering};
    use tempfile::TempDir;

    const CMD: &str = "tome harness session-start --workspace ws --harness h";

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    /// All four new-harness specs round-trip the Tome entry into the exact
    /// container the contract pins, and the entry deep-equals on a second
    /// merge (idempotent). We assert via the parsed `serde_json::Value` to be
    /// resilient to pretty-printer whitespace while still pinning structure.
    fn merge_once(spec: HookFileSpec, event: HookEvent, path: &Path) {
        let mut outcome = SyncOutcome::default();
        let mut first_error = None;
        let action =
            merge_command_hook("h", path, spec, event, CMD, &mut outcome, &mut first_error);
        assert!(
            first_error.is_none(),
            "merge must not error: {first_error:?}"
        );
        assert_eq!(action, Action::Created);
    }

    #[test]
    fn devin_spec_writes_exact_shape() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".devin/hooks.v1.json");
        merge_once(HookFileSpec::DevinHooksV1, HookEvent::SessionStart, &path);
        let v: JsonValue = serde_json::from_str(&read(&path)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "SessionStart": [
                    { "matcher": "", "hooks": [ { "type": "command", "command": CMD } ] }
                ]
            })
        );
    }

    #[test]
    fn copilot_spec_writes_exact_shape() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".github/hooks/tome.json");
        merge_once(HookFileSpec::CopilotHooks, HookEvent::SessionStart, &path);
        let v: JsonValue = serde_json::from_str(&read(&path)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                // Copilot CLI requires the top-level `version` (T087 live-probe);
                // Tome stamps it on create.
                "version": 1,
                "hooks": {
                    "SessionStart": [ { "type": "command", "command": CMD } ]
                }
            })
        );
    }

    #[test]
    fn gemini_spec_writes_exact_shape() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gemini/settings.json");
        merge_once(HookFileSpec::GeminiSettings, HookEvent::SessionStart, &path);
        let v: JsonValue = serde_json::from_str(&read(&path)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "hooks": {
                    "SessionStart": [
                        { "hooks": [ { "name": "tome", "type": "command", "command": CMD } ] }
                    ]
                }
            })
        );
    }

    /// US7: the REAL cursor module writes `.cursor/hooks.json` with the
    /// `{ version:1, hooks:{ sessionStart:[…] } }` shape, using the camelCase
    /// native event key (NOT PascalCase `SessionStart`). Tome stamps the required
    /// `version:1` on create.
    #[test]
    fn cursor_spec_writes_exact_shape() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".cursor/hooks.json");
        merge_once(HookFileSpec::CursorHooks, HookEvent::SessionStart, &path);
        let v: JsonValue = serde_json::from_str(&read(&path)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                // Cursor requires the top-level `version` (same as Copilot-CLI).
                "version": 1,
                "hooks": {
                    "sessionStart": [ { "type": "command", "command": CMD } ]
                }
            })
        );
    }

    #[test]
    fn antigravity_spec_writes_exact_shape() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".agents/hooks.json");
        merge_once(
            HookFileSpec::AntigravityHooks,
            HookEvent::PreInvocation,
            &path,
        );
        let v: JsonValue = serde_json::from_str(&read(&path)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "tome": {
                    "PreInvocation": [ { "type": "command", "command": CMD } ]
                }
            })
        );
    }

    /// Drift-pin: `effective_event_key(CursorHooks, SessionStart)` and
    /// `Cursor::hook_event_name(SessionStart)` are two independent literals that
    /// MUST stay equal. A drift would place the session-steering entry and the
    /// run-hook dispatch entry under DIFFERENT keys in `.cursor/hooks.json`,
    /// breaking their documented coexistence (US3 + US7 compose additively).
    #[test]
    fn cursor_session_start_key_matches_hook_event_name() {
        use crate::harness::HarnessModule;
        use crate::harness::cursor::CURSOR;
        use crate::harness::hooks_ir::PortableEvent;
        assert_eq!(
            effective_event_key(HookFileSpec::CursorHooks, HookEvent::SessionStart),
            CURSOR.hook_event_name(PortableEvent::SessionStart),
            "sessionStart literals must stay in sync: drift would split the \
             session-steering and run-hook entries under different keys",
        );
    }

    #[test]
    fn merge_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".gemini/settings.json");
        merge_once(HookFileSpec::GeminiSettings, HookEvent::SessionStart, &path);
        // Second merge → no change, LeftAlone.
        let mut outcome = SyncOutcome::default();
        let mut first_error = None;
        let action = merge_command_hook(
            "h",
            &path,
            HookFileSpec::GeminiSettings,
            HookEvent::SessionStart,
            CMD,
            &mut outcome,
            &mut first_error,
        );
        assert_eq!(action, Action::LeftAlone);
        assert!(first_error.is_none());
    }

    /// A developer's pre-existing hook entry under the SAME event is preserved
    /// across a Tome merge — Tome owns ONLY its own entry.
    #[test]
    fn merge_preserves_developer_hooks() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".github/hooks/tome.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&serde_json::json!({
                "version": 1,
                "hooks": {
                    "SessionStart": [ { "type": "command", "command": "dev-tool run" } ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        // The file pre-exists, so the merge action is `Updated`, not `Created`.
        let mut outcome = SyncOutcome::default();
        let mut first_error = None;
        let action = merge_command_hook(
            "h",
            &path,
            HookFileSpec::CopilotHooks,
            HookEvent::SessionStart,
            CMD,
            &mut outcome,
            &mut first_error,
        );
        assert!(first_error.is_none(), "{first_error:?}");
        assert_eq!(action, Action::Updated);
        let v: JsonValue = serde_json::from_str(&read(&path)).unwrap();
        let arr = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "developer entry + Tome entry");
        assert_eq!(arr[0]["command"], "dev-tool run");
        assert_eq!(arr[1]["command"], CMD);
        // The developer's top-level `version` key survives.
        assert_eq!(v["version"], 1);
    }

    /// PW6 (phase-wide): a symlinked component on a command-hook path fails
    /// CLOSED with `HookSettingsWriteFailed` (exit 44) — parity with the Claude
    /// hook sink (the P6 7→44 precedent), NOT the generic `Io` (7). Exercised on
    /// both the merge (live) and remove (non-live) paths.
    #[cfg(unix)]
    #[test]
    fn command_hook_symlink_refusal_is_exit_44() {
        use std::os::unix::fs::symlink;
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().canonicalize().unwrap();
        // `.devin` is a symlink to a sibling real dir — a symlinked component on
        // the hook path `<root>/.devin/hooks.v1.json`.
        let real = base.join("real");
        std::fs::create_dir_all(&real).unwrap();
        symlink(&real, base.join(".devin")).unwrap();
        let path = base.join(".devin/hooks.v1.json");

        let mut outcome = SyncOutcome::default();
        let mut first_error = None;
        let action = merge_command_hook(
            "h",
            &path,
            HookFileSpec::DevinHooksV1,
            HookEvent::SessionStart,
            CMD,
            &mut outcome,
            &mut first_error,
        );
        assert_eq!(action, Action::LeftAlone);
        let err = first_error.take().expect("symlink must be refused");
        assert_eq!(err.exit_code(), 44, "got {err:?}");

        // The remove path refuses the same component with the same exit code.
        let action = remove_command_hook(
            "h",
            &path,
            HookFileSpec::DevinHooksV1,
            HookEvent::SessionStart,
            CMD,
            &mut outcome,
            &mut first_error,
        );
        assert_eq!(action, Action::LeftAlone);
        assert_eq!(first_error.expect("remove also refuses").exit_code(), 44,);
    }

    /// Non-live removal takes ONLY Tome's deep-equal entry; a developer entry
    /// under the same event stays.
    #[test]
    fn remove_takes_only_tome_entry() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".devin/hooks.v1.json");
        // Seed with a developer entry + Tome's entry.
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let tome_entry = tome_hook_entry(HookFileSpec::DevinHooksV1, CMD);
        std::fs::write(
            &path,
            serde_json::to_string_pretty(&serde_json::json!({
                "SessionStart": [
                    { "matcher": "x", "hooks": [ { "type": "command", "command": "dev" } ] },
                    tome_entry,
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let mut outcome = SyncOutcome::default();
        let mut first_error = None;
        let action = remove_command_hook(
            "h",
            &path,
            HookFileSpec::DevinHooksV1,
            HookEvent::SessionStart,
            CMD,
            &mut outcome,
            &mut first_error,
        );
        assert_eq!(action, Action::Removed);
        assert!(first_error.is_none());
        let v: JsonValue = serde_json::from_str(&read(&path)).unwrap();
        let arr = v["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "only the developer entry remains");
        assert_eq!(arr[0]["hooks"][0]["command"], "dev");
    }

    /// Removing the sole Tome entry prunes the now-empty event array (and the
    /// antigravity `tome` block).
    #[test]
    fn remove_prunes_empty_antigravity_block() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".agents/hooks.json");
        merge_once(
            HookFileSpec::AntigravityHooks,
            HookEvent::PreInvocation,
            &path,
        );
        let mut outcome = SyncOutcome::default();
        let mut first_error = None;
        let action = remove_command_hook(
            "h",
            &path,
            HookFileSpec::AntigravityHooks,
            HookEvent::PreInvocation,
            CMD,
            &mut outcome,
            &mut first_error,
        );
        assert_eq!(action, Action::Removed);
        let v: JsonValue = serde_json::from_str(&read(&path)).unwrap();
        assert_eq!(v, serde_json::json!({}), "empty tome block pruned");
    }

    /// A malformed existing hook file → exit 43 (recorded on first_error,
    /// forward progress).
    #[test]
    fn malformed_existing_file_is_exit_43() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".devin/hooks.v1.json");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        let mut outcome = SyncOutcome::default();
        let mut first_error = None;
        let action = merge_command_hook(
            "h",
            &path,
            HookFileSpec::DevinHooksV1,
            HookEvent::SessionStart,
            CMD,
            &mut outcome,
            &mut first_error,
        );
        assert_eq!(action, Action::LeftAlone);
        let err = first_error.expect("malformed file must record an error");
        assert_eq!(err.exit_code(), 43, "got {err:?}");
    }

    /// `reconcile_command_hooks` fast-exits (no work, no error) when every
    /// snapshot is `SessionSteering::None` — the byte-identity guarantee.
    #[test]
    fn reconcile_fast_exits_when_all_none() {
        use crate::harness::StubHarness;
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let paths = crate::paths::Paths::from_root(tmp.path().join(".tome"));
        let workspace = crate::workspace::WorkspaceName::global();
        let deps = SyncDeps {
            paths: &paths,
            home_root: tmp.path(),
            workspace_name: &workspace,
            force: false,
            only_harness: None,
        };
        // A default stub returns `SessionSteering::None`.
        let stub = StubHarness::default();
        let snapshots = vec![crate::harness::sync::snapshot_for_test(
            &stub,
            &project,
            tmp.path(),
        )];
        let effective: HashSet<String> = std::iter::once("stub".to_string()).collect();
        let mut outcome = SyncOutcome::default();
        let (actions, err) =
            reconcile_command_hooks(&deps, &effective, &snapshots, &project, &mut outcome);
        assert!(actions.is_empty(), "no CommandHook harness → no actions");
        assert!(err.is_none());
        assert!(outcome.added.is_empty() && outcome.removed.is_empty());
    }

    /// End-to-end through `reconcile_command_hooks` with a CommandHook stub:
    /// live → the spec's file is written with the envelope-selecting command.
    #[test]
    fn reconcile_writes_for_live_command_hook_stub() {
        use crate::harness::StubHarness;
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let paths = crate::paths::Paths::from_root(tmp.path().join(".tome"));
        let workspace = crate::workspace::WorkspaceName::global();
        let deps = SyncDeps {
            paths: &paths,
            home_root: tmp.path(),
            workspace_name: &workspace,
            force: false,
            only_harness: None,
        };
        let stub = StubHarness::default().with_session_steering(SessionSteering::CommandHook {
            file_spec: HookFileSpec::DevinHooksV1,
            event: HookEvent::SessionStart,
            envelope: Envelope::ClaudeNested,
        });
        let snapshots = vec![crate::harness::sync::snapshot_for_test(
            &stub,
            &project,
            tmp.path(),
        )];
        let effective: HashSet<String> = std::iter::once("stub".to_string()).collect();
        let mut outcome = SyncOutcome::default();
        let (actions, err) =
            reconcile_command_hooks(&deps, &effective, &snapshots, &project, &mut outcome);
        assert!(err.is_none(), "{err:?}");
        assert_eq!(actions.get("stub"), Some(&Action::Created));
        let hook_file = project.join(".devin/hooks.v1.json");
        assert!(hook_file.is_file(), "devin hook file written");
        let v: JsonValue = serde_json::from_str(&read(&hook_file)).unwrap();
        let cmd = v["SessionStart"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(
            cmd,
            "tome harness session-start --workspace global --harness stub"
        );
    }
}

// =====================================================================
// Phase 11 / US2 (T045/T046/T048/T049): the REAL new-harness modules
// (devin / copilot-cli / gemini) drive `reconcile_command_hooks`
// end-to-end. These assert the module → snapshot → reconciler flow with
// each harness's ACTUAL `session_steering()` override, the exact
// `--harness <name>` command, developer-hook preservation, non-live
// removal, the gemini MCP-vs-hook no-clobber relationship, and that
// antigravity (rules-only) writes NO hook file.
// =====================================================================
#[cfg(test)]
mod us2_real_harness_tests {
    use super::*;
    use crate::harness::sync::{Action, SyncDeps, SyncOutcome};
    use crate::harness::{HarnessModule, SessionSteering};
    use std::collections::HashSet;
    use tempfile::TempDir;

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    /// Drive `reconcile_command_hooks` over a single real module with the
    /// given live set, returning `(actions, first_error, project_root_dir)`.
    fn run_reconcile(
        module: &dyn HarnessModule,
        live: bool,
        tmp: &TempDir,
    ) -> (
        std::collections::HashMap<String, Action>,
        Option<TomeError>,
        PathBuf,
    ) {
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let paths = crate::paths::Paths::from_root(tmp.path().join(".tome"));
        let workspace = crate::workspace::WorkspaceName::global();
        let deps = SyncDeps {
            paths: &paths,
            home_root: tmp.path(),
            workspace_name: &workspace,
            force: false,
            only_harness: None,
        };
        let snapshots = vec![crate::harness::sync::snapshot_for_test(
            module,
            &project,
            tmp.path(),
        )];
        let effective: HashSet<String> = if live {
            std::iter::once(module.name().to_string()).collect()
        } else {
            HashSet::new()
        };
        let mut outcome = SyncOutcome::default();
        let (actions, err) =
            reconcile_command_hooks(&deps, &effective, &snapshots, &project, &mut outcome);
        (actions, err, project)
    }

    /// T045/T049: the REAL devin module writes `.devin/hooks.v1.json` (no
    /// wrapper) carrying the exact `--harness devin` command, no envelope
    /// wrapper key.
    #[test]
    fn devin_real_module_writes_devin_hooks_v1_pin() {
        let tmp = TempDir::new().unwrap();
        let (actions, err, project) = run_reconcile(&crate::harness::devin::DEVIN, true, &tmp);
        assert!(err.is_none(), "{err:?}");
        assert_eq!(actions.get("devin"), Some(&Action::Created));
        let hook_file = project.join(".devin/hooks.v1.json");
        let v: JsonValue = serde_json::from_str(&read(&hook_file)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "SessionStart": [
                    { "matcher": "", "hooks": [ { "type": "command",
                        "command": "tome harness session-start --workspace global --harness devin" } ] }
                ]
            }),
        );
    }

    /// T046/T049: the REAL copilot-cli module writes `.github/hooks/tome.json`
    /// with the `{ hooks: { SessionStart: [...] } }` wrapper carrying the exact
    /// `--harness copilot-cli` command.
    #[test]
    fn copilot_cli_real_module_writes_github_hooks_tome_json_pin() {
        let tmp = TempDir::new().unwrap();
        let (actions, err, project) =
            run_reconcile(&crate::harness::copilot_cli::COPILOT_CLI, true, &tmp);
        assert!(err.is_none(), "{err:?}");
        assert_eq!(actions.get("copilot-cli"), Some(&Action::Created));
        let hook_file = project.join(".github/hooks/tome.json");
        let v: JsonValue = serde_json::from_str(&read(&hook_file)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                // T087 live-probe: Copilot CLI requires the top-level `version`.
                "version": 1,
                "hooks": {
                    "SessionStart": [ { "type": "command",
                        "command": "tome harness session-start --workspace global --harness copilot-cli" } ]
                }
            }),
        );
    }

    /// T048/T049: the REAL gemini module writes the PROJECT
    /// `.gemini/settings.json` `hooks` section carrying the exact `--harness
    /// gemini` command.
    #[test]
    fn gemini_real_module_writes_project_settings_hooks_pin() {
        let tmp = TempDir::new().unwrap();
        let (actions, err, project) = run_reconcile(&crate::harness::gemini::GEMINI, true, &tmp);
        assert!(err.is_none(), "{err:?}");
        assert_eq!(actions.get("gemini"), Some(&Action::Created));
        let hook_file = project.join(".gemini/settings.json");
        let v: JsonValue = serde_json::from_str(&read(&hook_file)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "hooks": {
                    "SessionStart": [
                        { "hooks": [ { "name": "tome", "type": "command",
                            "command": "tome harness session-start --workspace global --harness gemini" } ] }
                    ]
                }
            }),
        );
    }

    /// T047/T049: antigravity is RULES-ONLY — its `session_steering()` is
    /// `None`, so `reconcile_command_hooks` produces no action for it and
    /// writes NO hook file anywhere under the project root.
    #[test]
    fn antigravity_real_module_writes_no_hook_file() {
        let tmp = TempDir::new().unwrap();
        // Sanity: the module itself declares rules-only.
        assert_eq!(
            crate::harness::antigravity::ANTIGRAVITY.session_steering(),
            SessionSteering::None,
        );
        let (actions, err, project) =
            run_reconcile(&crate::harness::antigravity::ANTIGRAVITY, true, &tmp);
        assert!(err.is_none(), "{err:?}");
        // No CommandHook → fast-exit → no action recorded for antigravity.
        assert!(
            !actions.contains_key("antigravity"),
            "antigravity (rules-only) must produce no command-hook action",
        );
        // Neither candidate antigravity hook path exists.
        assert!(!project.join(".agents/hooks.json").exists());
        assert!(!project.join(".agent/hooks.json").exists());
    }

    /// T049 developer-hook preservation: a pre-existing unrelated hook entry in
    /// the devin file survives the Tome write (real module).
    #[test]
    fn devin_real_module_preserves_developer_hook() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        let hook_file = project.join(".devin/hooks.v1.json");
        std::fs::create_dir_all(hook_file.parent().unwrap()).unwrap();
        std::fs::write(
            &hook_file,
            serde_json::to_string_pretty(&serde_json::json!({
                "SessionStart": [
                    { "matcher": "x", "hooks": [ { "type": "command", "command": "dev run" } ] }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let paths = crate::paths::Paths::from_root(tmp.path().join(".tome"));
        let workspace = crate::workspace::WorkspaceName::global();
        let deps = SyncDeps {
            paths: &paths,
            home_root: tmp.path(),
            workspace_name: &workspace,
            force: false,
            only_harness: None,
        };
        let snapshots = vec![crate::harness::sync::snapshot_for_test(
            &crate::harness::devin::DEVIN,
            &project,
            tmp.path(),
        )];
        let effective: HashSet<String> = std::iter::once("devin".to_string()).collect();
        let mut outcome = SyncOutcome::default();
        let (actions, err) =
            reconcile_command_hooks(&deps, &effective, &snapshots, &project, &mut outcome);
        assert!(err.is_none(), "{err:?}");
        // File pre-existed → Updated.
        assert_eq!(actions.get("devin"), Some(&Action::Updated));
        let v: JsonValue = serde_json::from_str(&read(&hook_file)).unwrap();
        let arr = v["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "developer entry + Tome entry");
        assert_eq!(arr[0]["hooks"][0]["command"], "dev run");
        assert_eq!(
            arr[1]["hooks"][0]["command"],
            "tome harness session-start --workspace global --harness devin"
        );
    }

    /// T049 non-live removal: a harness that left the effective set has ONLY
    /// its Tome hook entry removed; a developer entry under the same event
    /// stays (real copilot-cli module).
    #[test]
    fn copilot_cli_real_module_non_live_removes_only_tome_entry() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        let hook_file = project.join(".github/hooks/tome.json");
        std::fs::create_dir_all(hook_file.parent().unwrap()).unwrap();
        // Seed: developer entry + Tome's exact entry.
        let tome_cmd = "tome harness session-start --workspace global --harness copilot-cli";
        std::fs::write(
            &hook_file,
            serde_json::to_string_pretty(&serde_json::json!({
                "hooks": {
                    "SessionStart": [
                        { "type": "command", "command": "dev run" },
                        { "type": "command", "command": tome_cmd }
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let paths = crate::paths::Paths::from_root(tmp.path().join(".tome"));
        let workspace = crate::workspace::WorkspaceName::global();
        let deps = SyncDeps {
            paths: &paths,
            home_root: tmp.path(),
            workspace_name: &workspace,
            force: false,
            only_harness: None,
        };
        let snapshots = vec![crate::harness::sync::snapshot_for_test(
            &crate::harness::copilot_cli::COPILOT_CLI,
            &project,
            tmp.path(),
        )];
        // NON-live: empty effective set.
        let effective: HashSet<String> = HashSet::new();
        let mut outcome = SyncOutcome::default();
        let (actions, err) =
            reconcile_command_hooks(&deps, &effective, &snapshots, &project, &mut outcome);
        assert!(err.is_none(), "{err:?}");
        assert_eq!(actions.get("copilot-cli"), Some(&Action::Removed));
        let v: JsonValue = serde_json::from_str(&read(&hook_file)).unwrap();
        let arr = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "only the developer entry remains");
        assert_eq!(arr[0]["command"], "dev run");
    }

    /// T049 gemini developer-hook preservation (nested shape): a pre-existing
    /// developer `hooks.SessionStart[]` entry AND an unrelated top-level key
    /// survive the Tome merge; Tome's entry lands AFTER the developer's.
    #[test]
    fn gemini_real_module_preserves_developer_hook() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        let hook_file = project.join(".gemini/settings.json");
        std::fs::create_dir_all(hook_file.parent().unwrap()).unwrap();
        std::fs::write(
            &hook_file,
            serde_json::to_string_pretty(&serde_json::json!({
                "theme": "dark",
                "hooks": {
                    "SessionStart": [
                        { "hooks": [ { "name": "dev", "type": "command", "command": "dev run" } ] }
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let paths = crate::paths::Paths::from_root(tmp.path().join(".tome"));
        let workspace = crate::workspace::WorkspaceName::global();
        let deps = SyncDeps {
            paths: &paths,
            home_root: tmp.path(),
            workspace_name: &workspace,
            force: false,
            only_harness: None,
        };
        let snapshots = vec![crate::harness::sync::snapshot_for_test(
            &crate::harness::gemini::GEMINI,
            &project,
            tmp.path(),
        )];
        let effective: HashSet<String> = std::iter::once("gemini".to_string()).collect();
        let mut outcome = SyncOutcome::default();
        let (actions, err) =
            reconcile_command_hooks(&deps, &effective, &snapshots, &project, &mut outcome);
        assert!(err.is_none(), "{err:?}");
        // File pre-existed → Updated.
        assert_eq!(actions.get("gemini"), Some(&Action::Updated));
        let v: JsonValue = serde_json::from_str(&read(&hook_file)).unwrap();
        let arr = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "developer entry + Tome entry");
        assert_eq!(arr[0]["hooks"][0]["command"], "dev run");
        assert_eq!(
            arr[1]["hooks"][0]["command"],
            "tome harness session-start --workspace global --harness gemini"
        );
        // The unrelated top-level key survives untouched.
        assert_eq!(v["theme"], "dark");
    }

    /// T049 gemini NON-LIVE removal: a harness that left the effective set has
    /// ONLY its deep-equal Tome entry removed; a developer entry under the same
    /// event stays and the now-empty Tome scaffolding is pruned.
    #[test]
    fn gemini_real_module_non_live_removes_only_tome_entry() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        let hook_file = project.join(".gemini/settings.json");
        std::fs::create_dir_all(hook_file.parent().unwrap()).unwrap();
        // Seed: a developer entry + Tome's EXACT gemini entry under the same
        // event array.
        let tome_cmd = "tome harness session-start --workspace global --harness gemini";
        std::fs::write(
            &hook_file,
            serde_json::to_string_pretty(&serde_json::json!({
                "hooks": {
                    "SessionStart": [
                        { "hooks": [ { "name": "dev", "type": "command", "command": "dev run" } ] },
                        { "hooks": [ { "name": "tome", "type": "command", "command": tome_cmd } ] }
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        let (actions, err, _project) = run_reconcile(&crate::harness::gemini::GEMINI, false, &tmp);
        assert!(err.is_none(), "{err:?}");
        assert_eq!(actions.get("gemini"), Some(&Action::Removed));
        let v: JsonValue = serde_json::from_str(&read(&hook_file)).unwrap();
        let arr = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "only the developer entry remains");
        assert_eq!(arr[0]["hooks"][0]["command"], "dev run");
    }

    /// T048/T049 gemini no-clobber: the MCP server lives in the GLOBAL
    /// `~/.gemini/settings.json` and the hook lives in the PROJECT
    /// `<project>/.gemini/settings.json`. Even when BOTH files exist and a
    /// pre-existing `mcpServers` block sits in the project file alongside the
    /// hook write, the hook write preserves it (and vice versa) — disjoint
    /// top-level keys through the lenient preserve-order parse.
    #[test]
    fn gemini_hook_write_preserves_a_coexisting_mcp_block() {
        let tmp = TempDir::new().unwrap();
        let project = tmp.path().join("project");
        let project_settings = project.join(".gemini/settings.json");
        std::fs::create_dir_all(project_settings.parent().unwrap()).unwrap();
        // A developer (or a hypothetical project-local MCP) wrote an
        // `mcpServers` block into the SAME project settings.json the hook
        // targets. The hook write must NOT drop it.
        std::fs::write(
            &project_settings,
            serde_json::to_string_pretty(&serde_json::json!({
                "mcpServers": { "other": { "command": "x", "args": [] } },
                "theme": "dark"
            }))
            .unwrap(),
        )
        .unwrap();

        let (actions, err, project_root) =
            run_reconcile(&crate::harness::gemini::GEMINI, true, &tmp);
        assert!(err.is_none(), "{err:?}");
        assert_eq!(actions.get("gemini"), Some(&Action::Updated));
        let v: JsonValue = serde_json::from_str(&read(&project_settings)).unwrap();
        // The hook was added under `hooks`.
        assert_eq!(
            v["hooks"]["SessionStart"][0]["hooks"][0]["command"],
            "tome harness session-start --workspace global --harness gemini"
        );
        // The pre-existing `mcpServers` + `theme` keys survive untouched.
        assert_eq!(v["mcpServers"]["other"]["command"], "x");
        assert_eq!(v["theme"], "dark");
        // The GLOBAL MCP path is a different file and was NOT created by the
        // hook write.
        let global_mcp = tmp.path().join(".gemini/settings.json");
        assert_ne!(project_settings, global_mcp);
        assert!(
            !global_mcp.exists(),
            "the hook reconciler must not touch the GLOBAL gemini settings.json",
        );
        let _ = project_root;
    }
}

// =====================================================================
// US3 — plugin-hook dispatch registration tests.
// =====================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::harness::HookFileSpec;

    /// US3.1: the `run-hook` match-all dispatcher entry is shaped per spec, and
    /// the two formerly-`None` file specs (Cursor already wired in US2; Codex
    /// newly wired here) now resolve to their on-disk hook files.
    #[test]
    fn run_hook_entry_is_match_all_per_spec() {
        // Devin: {matcher:"", hooks:[{type:command, command}]} (match-all).
        let e = tome_run_hook_entry(
            HookFileSpec::DevinHooksV1,
            "tome harness run-hook --event PreToolUse --harness devin --workspace w",
        );
        assert_eq!(e["matcher"], serde_json::json!(""));
        assert_eq!(e["hooks"][0]["type"], "command");
        // Cursor file path resolves now.
        let p = hook_file_path(HookFileSpec::CursorHooks, std::path::Path::new("/proj")).unwrap();
        assert!(p.ends_with(".cursor/hooks.json"));
        // Codex file path resolves now (was None).
        let p = hook_file_path(HookFileSpec::CodexHooks, std::path::Path::new("/proj")).unwrap();
        assert!(p.ends_with(".codex/hooks.json"));
    }
}

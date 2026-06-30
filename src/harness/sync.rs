//! Sync orchestrator. Computes the effective harness list, dispatches
//! per-harness writes (rules-file + MCP config), and runs the cleanup
//! pass for harnesses no longer in the effective list.
//!
//! ## Algorithm reference
//!
//! Mirrors `specs/004-phase-4-refactor-harnesses/contracts/sync-algorithm.md`.
//! Phase A (DB read under the lock) is the **caller's** responsibility — by
//! the time this module's [`sync_project`] runs, the project marker has been
//! landed, the central DB row has been UPSERTed, and the lockfile is
//! released. Phase B (filesystem reads + writes against every harness's
//! per-project / per-home files) runs entirely unlocked. The slow-FS
//! risk is mitigated by atomic-rename per individual write — a concurrent
//! sync's worst observable outcome is a stale effective list applied;
//! the next sync corrects it (FR-525 byte-for-byte idempotence).
//!
//! ## Multi-harness sharing (FR-482 / FR-483)
//!
//! When two effective harnesses target the same rules-file path (e.g.
//! `claude-code` + `codex` both at `<project>/AGENTS.md`) the orchestrator
//! deduplicates on the target path and writes once. Same for MCP config
//! paths (Codex + Gemini both share `<home>/.codex/`, etc.). The cleanup
//! pass for non-live harnesses respects the dedup — a path stays as long
//! as any live harness still targets it.
//!
//! ## Forward-progress on clash (FR-403)
//!
//! When a user-owned `tome` entry blocks an MCP write without `--force`,
//! the orchestrator records a [`TomeError::HarnessClash`] but keeps
//! processing the rest of the harness list. The first clash wins for the
//! overall `Result::Err` so the CLI's exit code is 19; the
//! rules-file writes for unaffected harnesses still happen. Re-running
//! after the user resolves the clash converges the state.

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::error::TomeError;
use crate::harness::reconcile::agents::reconcile_agents;
use crate::harness::reconcile::guardrails::reconcile_guardrails;
use crate::harness::reconcile::hooks::{
    reconcile_command_hooks, reconcile_hooks, reconcile_plugin_hook_dispatch,
    reconcile_tome_session_hooks,
};
use crate::harness::reconcile::open_plugins::reconcile_open_plugins;
use crate::harness::reconcile::plugins::reconcile_plugins;
// Shared bookkeeping for the orchestrator's rules/MCP loop; the per-sink
// reconcilers under `reconcile/` call the same path (Phase 7 / FR-011).
use crate::harness::reconcile::record_action;
use crate::harness::{
    BlockBodyStyle, HarnessModule, RulesFileStrategy, mcp_config, rules_file,
    with_effective_modules,
};
use crate::paths::Paths;
use crate::settings::{GlobalSettings, WorkspaceSettings, resolve_effective_list};
use crate::workspace::WorkspaceName;

// =====================================================================
// Public types
// =====================================================================

/// Caller-supplied inputs for one sync invocation.
///
/// `home_root` is passed in (rather than resolved from `$HOME` here) so
/// tests can isolate harness-detection paths against a tempdir without
/// env mutation. `workspace_name` names the binding the caller just
/// established (or the workspace already-bound to this project); the
/// orchestrator emits it verbatim into the MCP entry's `--workspace`
/// argument.
#[derive(Debug)]
pub struct SyncDeps<'a> {
    pub paths: &'a Paths,
    pub home_root: &'a Path,
    pub workspace_name: &'a WorkspaceName,
    /// When `true`, rewrites user-owned `tome` entries in harness MCP
    /// configs instead of returning [`TomeError::HarnessClash`]. Maps
    /// directly to the CLI `--force` flag on `tome workspace use` (and
    /// the future `tome harness sync --force`).
    pub force: bool,
    /// When `Some(set)`, reconcile ONLY the harnesses whose canonical name is
    /// in `set` (each written if effective, removed if not) and leave every
    /// other harness's files untouched. When `None`, reconcile the full
    /// effective set. Set by `tome sync --harness` (repeatable). The set holds
    /// alias-resolved canonical names, so membership is `set.contains(m.name())`.
    pub only_harness: Option<HashSet<String>>,
}

/// Summary of one sync pass per FR-547. Serialised verbatim in the
/// CLI's `--json` envelope; the field shape is wire-stable.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncOutcome {
    pub added: Vec<SyncChange>,
    pub updated: Vec<SyncChange>,
    pub removed: Vec<SyncChange>,
    pub leave_alones: usize,
    pub decisions: Vec<HarnessDecision>,
}

/// One on-disk change recorded under `added` / `updated` / `removed`.
///
/// `harness` is the harness's `name()` (e.g. `"claude-code"`); for
/// rules-file writes shared across multiple harnesses, the entry is
/// emitted once with the first-touching harness recorded so the audit
/// trail names a concrete harness without lying about which one
/// "owned" the write.
#[derive(Debug, Clone, Serialize)]
pub struct SyncChange {
    pub harness: String,
    pub subsystem: SyncSubsystem,
    pub path: PathBuf,
}

/// Which subsystem a [`SyncChange`] applies to. JSON wire form is
/// snake_case (`"rules"` / `"mcp"` / `"agents"`).
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncSubsystem {
    Rules,
    Mcp,
    /// Native agent files (Phase 6 / US1). One change per agent file
    /// written or removed under the harness's `agent_dir`.
    Agents,
    /// Real Claude Code hooks (Phase 6 / US2). One change per
    /// `settings.local.json` whose `hooks` object Tome merged into or pruned.
    Hooks,
    /// Guardrails prose fallback (Phase 6 / US3). One change per rules-file
    /// target or Cursor sibling whose guardrails regions Tome reconciled.
    Guardrails,
    /// Embedded TypeScript plugin shims (Phase 11 / G2, `TsPlugin` steering).
    /// One change per `tome.ts` shim written into or removed from a harness's
    /// Tome-managed plugin dir. Added LAST so the snake_case wire form only
    /// gains a new value (`"plugins"`) when a `TsPlugin` harness participates;
    /// with every Phase ≤10 module returning `SessionSteering::None` this value
    /// never appears, so the existing wire shape is byte-identical.
    Plugins,
    /// Open Plugins `tome-op` bundle emit/remove (Phase 11 / US4). One change per
    /// bundle written or removed for a `generic-op` / `goose` harness. Added LAST
    /// so the snake_case wire form only gains the new value (`"open_plugins"`)
    /// when an Open Plugins harness participates; no Phase ≤10 / US1–US3 harness
    /// is an Open Plugins host, so the existing wire shape is byte-identical.
    OpenPlugins,
}

/// Per-harness decision record. Populated for every harness in
/// `with_effective_modules`, regardless of whether it's in the
/// effective list — the field set lets `tome harness sync --json`
/// callers reason about cleanup as well as additions.
///
/// Serialized Phase 6 field order is `agents_action`, `hooks_action`,
/// `guardrails_action` — merge chronology (each appended LAST as US1→US2→US3
/// landed), distinct from the hooks→guardrails→agents sink *processing* order.
#[derive(Debug, Clone, Serialize)]
pub struct HarnessDecision {
    pub harness: String,
    pub in_effective_list: bool,
    pub rules_action: Action,
    pub mcp_action: Action,
    /// Phase 6 / US1: the aggregate native-agent reconciliation action for
    /// this harness. `Created`/`Updated` when at least one agent file was
    /// written, `Removed` when at least one was deleted (and none written),
    /// `LeftAlone` when nothing changed or the harness has no native-agent
    /// support. Per-file granularity lives in `added`/`updated`/`removed`.
    pub agents_action: Action,
    /// Phase 6 / US2: the hooks reconciliation action for this harness.
    /// `Created` when `settings.local.json` was created, `Updated` when its
    /// `hooks` object was merged into or pruned, `LeftAlone` otherwise (a
    /// `GuardrailsOnly` harness, or no on-disk change). Appended LAST so the
    /// byte-stable JSON pin only gains a trailing field.
    pub hooks_action: Action,
    /// Phase 6 / US3: the guardrails reconciliation action for this harness.
    /// `Created`/`Updated`/`Removed` when the harness's guardrails target
    /// gained/changed/lost a region, `LeftAlone` otherwise. Appended LAST so
    /// the byte-stable JSON pin only gains a trailing field.
    pub guardrails_action: Action,
    /// Phase 11 / G2: the TypeScript-shim (`TsPlugin`) reconciliation action
    /// for this harness. `Created`/`Updated` when the embedded shim was
    /// written, `Removed` when it was cleaned up, `LeftAlone` otherwise.
    ///
    /// Appended LAST AND gated with `skip_serializing_if` so the field is
    /// OMITTED from the JSON wire form when it is `LeftAlone` — which it always
    /// is for the five Phase ≤10 modules (none declares `TsPlugin`). That keeps
    /// the byte-stable `SyncOutcome` / `HarnessDecision` pins UNCHANGED until a
    /// `TsPlugin` harness actually does plugin work; only then does the new
    /// trailing `plugins_action` key appear.
    #[serde(skip_serializing_if = "Action::is_left_alone")]
    pub plugins_action: Action,
}

/// What happened to one subsystem (rules-file or MCP config) for one
/// harness during this sync pass.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Created,
    Updated,
    Removed,
    LeftAlone,
}

impl Action {
    /// `true` for [`Action::LeftAlone`]. Used by `serde`'s
    /// `skip_serializing_if` on the Phase 11 `HarnessDecision::plugins_action`
    /// field so a no-op (the steady state for every Phase ≤10 module) is
    /// omitted from the JSON wire form, keeping the byte-stable pins unchanged.
    fn is_left_alone(&self) -> bool {
        matches!(self, Action::LeftAlone)
    }
}

// =====================================================================
// Public entry point
// =====================================================================

/// Run the harness sync algorithm for `project_root` against `deps`.
///
/// Reads the project marker (`<project_root>/.tome/config.toml`), the
/// bound workspace's `settings.toml` (if present), and the global
/// `settings.toml` (if present); computes the effective harness list;
/// dispatches rules-file + MCP config writes for live harnesses; runs
/// the cleanup pass for non-live ones.
///
/// Returns [`SyncOutcome`] on success, [`TomeError`] on the first hard
/// error (the project marker is unreadable / composition failure /
/// HarnessClash without `--force`). On HarnessClash the orchestrator
/// processes every harness before returning so the user sees the full
/// state in the next `tome doctor` run.
///
/// **Note on the `ScopeProvider`**: production uses
/// [`crate::commands::harness::CentralDbScopeProvider`], which consults
/// the central SQLite registry for workspace membership and reads each
/// referenced workspace's `settings.toml` for its directly-declared
/// harness list. A reference to a workspace that does not exist in the
/// central registry surfaces as exit 13 (`WorkspaceNotFound`); a
/// reference to a workspace whose settings file is malformed surfaces
/// as exit 70 (`WorkspaceMalformed`).
pub fn sync_project(project_root: &Path, deps: &SyncDeps<'_>) -> Result<SyncOutcome, TomeError> {
    // -----------------------------------------------------------------
    // 1. Read the three settings layers.
    // -----------------------------------------------------------------
    let marker_path = Paths::project_marker_config(project_root);
    // The project marker is REQUIRED here (sync only runs on a bound
    // project) — route through the canonical `read_project_marker` whose
    // classification splits IO (exit 7) from parse (exit 70). This is the
    // not-Option form; `settings::scopes::load_project_marker` is the
    // Option-wrapping form used by the layered-walk consumers.
    let marker = crate::settings::parser::read_project_marker(&marker_path)?;

    let workspace_settings = read_workspace_settings(deps)?;
    let global_settings = read_global_settings(deps)?;

    // Resolve the `strip_plugin_agent_privileges` scalar ONCE per sync, against
    // the same project → workspace → global scopes (first-declarer-wins, R-12),
    // reusing the US4 closure resolver verbatim (one new call site, no second
    // resolver). The resolved bool governs only the Claude Code agent EMISSION
    // clone below — it never touches the agent source, so the US5 doctor
    // privilege audit still sees the original privileged fields (FR-050/052).
    let strip_agent_privileges = crate::settings::resolve_scalar_with(
        Some(&marker),
        workspace_settings.as_ref(),
        &global_settings,
        |p| p.strip_plugin_agent_privileges,
        |w| w.strip_plugin_agent_privileges,
        |g| g.strip_plugin_agent_privileges,
    );

    // -----------------------------------------------------------------
    // 2. Compute the effective list.
    //
    // The production `ScopeProvider` consults the central registry for
    // workspace membership and reads each named workspace's
    // `settings.toml` for its directly-declared harness list. Lives in
    // `crate::commands::harness` because the `CentralDbScopeProvider`
    // type is the seam between this orchestrator and the CLI command
    // surface; keeping it there avoids a circular dep.
    // -----------------------------------------------------------------
    let scope_provider = crate::commands::harness::CentralDbScopeProvider::new(deps.paths);
    let effective = resolve_effective_list(
        Some(&marker),
        workspace_settings.as_ref(),
        &global_settings,
        &scope_provider,
    )
    .map_err(TomeError::from)?;

    let effective_names: HashSet<String> =
        effective.harnesses.iter().map(|h| h.name.clone()).collect();

    reconcile_against_effective(project_root, deps, &effective_names, strip_agent_privileges)
}

/// Tear down EVERY Tome-owned harness integration in `project_root`, regardless
/// of the project marker's declared harness list (PW4).
///
/// Drives the SAME reconciliation machinery as [`sync_project`] but with an
/// EMPTY effective set, so every harness's snapshot dispatches through its
/// reconciler's non-live REMOVAL branch — rules files, MCP entries, plugin
/// hooks, Tome's own session hooks, the new `CommandHook` session entries, TS
/// plugin shims, guardrails, native agents, AND the Open Plugins `tome-op`
/// bundle. Opt-in targets (`generic` `mcp.json`, `generic-op`/`goose` bundles)
/// are picked up via the same artifact-present probe `sync_project` uses, so a
/// previously-selected target is cleaned even though it never sits in any
/// marker. This is the single SSOT for "remove everything" — it inherits every
/// safety guard the writers have (structural-match-only removal, marker-bounded
/// edits, symlink refusal, forward-progress `first_error`).
///
/// Unlike `sync_project` it does NOT require a readable project marker: with an
/// empty effective set there is no scope to resolve, so a project whose marker
/// is already gone (or unreadable) still tears down cleanly. `strip_agent_privileges`
/// is irrelevant here (it governs only the Claude Code agent EMISSION clone, and
/// every harness is non-live → agents are removed regardless).
pub fn teardown_project(
    project_root: &Path,
    deps: &SyncDeps<'_>,
) -> Result<SyncOutcome, TomeError> {
    let empty: HashSet<String> = HashSet::new();
    reconcile_against_effective(project_root, deps, &empty, false)
}

/// The reconciliation body shared by [`sync_project`] (effective set from the
/// marker) and [`teardown_project`] (empty effective set). Given the resolved
/// `effective_names`, it snapshots the registry, runs every per-sink
/// write/leave-alone/remove pass, and returns the [`SyncOutcome`]. Extracted so
/// the two entry points cannot drift in which sinks they touch.
fn reconcile_against_effective(
    project_root: &Path,
    deps: &SyncDeps<'_>,
    effective_names: &HashSet<String>,
    strip_agent_privileges: bool,
) -> Result<SyncOutcome, TomeError> {
    // -----------------------------------------------------------------
    // 3. Walk every harness in the effective registry.
    //
    // The registry view is captured *outside* `with_effective_modules`
    // so the read guard does not span the long FS work — we capture the
    // per-harness metadata into owned values up front, then drop the
    // borrow before dispatch.
    // -----------------------------------------------------------------
    let all_filtered = collect_harness_snapshots(project_root, deps, effective_names);
    let mut outcome = SyncOutcome::default();

    // Phase 11 / US4: partition out the Open Plugins harnesses (`generic-op`,
    // `goose` — `open_plugins_root == Some`). They integrate via the atomic
    // `open_plugins` emitter, NOT the per-sink loop, so they must be EXCLUDED
    // from every rules/MCP/hooks/guardrails/agents/plugins pass (the bundle is
    // all-or-nothing; the per-sink writers would double-write its `AGENTS.md` /
    // `.mcp.json`). With no Open Plugins harness in the effective set this is a
    // no-op split: `snapshots == all_filtered` and `op_snapshots` is empty, so
    // every existing project's sink processing is byte-identical.
    let (op_snapshots, snapshots): (Vec<HarnessSnapshot>, Vec<HarnessSnapshot>) = all_filtered
        .into_iter()
        .partition(|s| s.open_plugins_root.is_some());

    // The bytes written to a SHARED rules file must stay correct for EVERY live
    // co-owner of that path, regardless of `--harness`. A `--harness X` run still
    // touches only X's own sinks (the main loop below iterates the FILTERED
    // `snapshots`), but the body-style LCD and the live-sharer set for a shared
    // rules path are computed against the FULL effective registry — otherwise
    // `tome sync --harness codex` on a codex+opencode project would see only
    // codex as a sharer, pick the `AtInclude` LCD, and rewrite the shared
    // `AGENTS.md` to `@include` form, breaking opencode's still-live inline view
    // until a full sync. So the rules-path grouping is built from an UNFILTERED
    // snapshot pass; when `only_harness` is `None` this is identical to
    // `snapshots`, so the full-sync path is byte-for-byte unchanged.
    let all_snapshots = match deps.only_harness {
        Some(_) => collect_all_harness_snapshots(project_root, deps, effective_names),
        // No filter active — the full pass already IS `snapshots`; avoid the
        // second registry walk + allocation.
        None => Vec::new(),
    };
    let rules_grouping_source: &[HarnessSnapshot] = match deps.only_harness {
        Some(_) => &all_snapshots,
        None => &snapshots,
    };

    // Build the dedup maps for shared rules-file / MCP paths. The rules map is
    // keyed off the FULL effective view (so a shared file's body stays correct
    // for every live co-owner under `--harness`); the MCP map stays scoped to the
    // FILTERED set — MCP entries are per-harness writes, never co-owned content,
    // so a `--harness X` run must not consult other harnesses' MCP sharers.
    let rules_targets_by_path = group_by_path(rules_grouping_source, |s| &s.rules_path);
    let mcp_targets_by_path = group_by_path(&snapshots, |s| &s.mcp_path);

    // Track which deduplicated targets have already been processed so
    // multi-harness sharing only triggers one write/cleanup. The key is
    // canonicalised by the harness's reported path (no extra canonical
    // join — we operate on the bytes the harness handed back).
    let mut rules_paths_processed: HashSet<PathBuf> = HashSet::new();
    let mut mcp_paths_processed: HashSet<PathBuf> = HashSet::new();

    let mut first_clash: Option<TomeError> = None;

    for snap in &snapshots {
        let is_live = effective_names.contains(&snap.name);

        // -------------------------------------------------------------
        // 3a. Rules file
        // -------------------------------------------------------------
        let rules_action = if !rules_paths_processed.insert(snap.rules_path.clone()) {
            // Already processed this path under another harness.
            //
            // M3 / FR-013a (shared-sink single region): `rules_paths_processed`
            // is the dedupe that guarantees N live harnesses resolving to the
            // SAME file (e.g. several `AGENTS.md` sharers) produce exactly ONE
            // `tome:begin/end` region — the first sharer writes; every later one
            // short-circuits to `LeftAlone` here. The block writer itself
            // (`rules_file::compose_block_write`) collapses any pre-existing
            // duplicate Tome blocks in the file to a single canonical region, so
            // even a hand-edited file converges. The inserted directive body is
            // wholly Tome-owned (built by `routing::build_directive`), so no
            // verbatim-third-party marker-collision scan is needed at this sink —
            // a developer file whose own content carries a corrupt `tome:*`
            // marker still fails CLOSED via `find_all_blocks`'s malformed-marker
            // `Err` (exit 7), never a silent clobber. (Guardrails, which DOES
            // copy verbatim plugin content, keeps its own `body_contains_marker_line`
            // fail-closed scan — that is the right place for it.)
            Action::LeftAlone
        } else {
            // The "live" decision for a shared path is OR-of-live across
            // every harness that targets it: as long as ANY harness in
            // the effective list still wants this path, the block stays.
            let live_sharers: Vec<&HarnessSnapshot> = rules_targets_by_path
                .get(&snap.rules_path)
                .map(|sharers| {
                    sharers
                        .iter()
                        .copied()
                        .filter(|s| effective_names.contains(&s.name))
                        .collect()
                })
                .unwrap_or_default();
            let any_live = !live_sharers.is_empty();
            if any_live {
                // Lowest-common-denominator body style across the group sharing
                // this rules path (F-RULES-OPENCODE, §R-8 — mirrors the
                // guardrails reconciler's union-across-sharers). If ANY live
                // sharer requires `Inline` (OpenCode, which has no `@`-include
                // support and would read `@.tome/RULES.md` as prose), the inline
                // body is written so EVERY sharer receives the real rules.
                // Include-capable harnesses resolve an inline body correctly, so
                // inline is the safe LCD; an include-only group stays AtInclude.
                let style = group_body_style(&live_sharers);
                let body = compute_rules_body(style, &snap.rules_path, project_root)?;
                let action = write_rules_for_path(snap, &body)?;
                record_action(
                    &mut outcome,
                    &snap.name,
                    SyncSubsystem::Rules,
                    &snap.rules_path,
                    action,
                );
                action
            } else {
                let action = clean_rules_for_path(snap)?;
                record_action(
                    &mut outcome,
                    &snap.name,
                    SyncSubsystem::Rules,
                    &snap.rules_path,
                    action,
                );
                action
            }
        };

        // -------------------------------------------------------------
        // 3b. MCP config
        // -------------------------------------------------------------
        let mcp_action = if snap.mcp_manual_only {
            // Phase 11: this harness has no writable MCP config file (UI-only,
            // jetbrains-ai). Skip the MCP sink entirely — no read, no write, no
            // remove — and leave the path unmarked-processed so a real sharer of
            // the same path (there is none in practice) is unaffected. The
            // harness still gets its rules-file integration above; the
            // "paste this snippet" notice is a separate US5 concern.
            Action::LeftAlone
        } else if !mcp_paths_processed.insert(snap.mcp_path.clone()) {
            Action::LeftAlone
        } else {
            let any_live = mcp_targets_by_path
                .get(&snap.mcp_path)
                .map(|sharers| sharers.iter().any(|s| effective_names.contains(&s.name)))
                .unwrap_or(false);
            if any_live {
                match write_mcp_for_harness(snap, deps) {
                    Ok(action) => {
                        record_action(
                            &mut outcome,
                            &snap.name,
                            SyncSubsystem::Mcp,
                            &snap.mcp_path,
                            action,
                        );
                        action
                    }
                    Err(err @ TomeError::HarnessClash { .. }) => {
                        if first_clash.is_none() {
                            first_clash = Some(err);
                        }
                        // Don't record an action — nothing happened on disk.
                        Action::LeftAlone
                    }
                    Err(other) => return Err(other),
                }
            } else {
                let action = clean_mcp_for_harness(snap)?;
                record_action(
                    &mut outcome,
                    &snap.name,
                    SyncSubsystem::Mcp,
                    &snap.mcp_path,
                    action,
                );
                action
            }
        };

        outcome.decisions.push(HarnessDecision {
            harness: snap.name.clone(),
            in_effective_list: is_live,
            rules_action,
            mcp_action,
            // Backfilled by the hooks + guardrails + agents + plugins
            // reconciliation passes below.
            agents_action: Action::LeftAlone,
            hooks_action: Action::LeftAlone,
            guardrails_action: Action::LeftAlone,
            plugins_action: Action::LeftAlone,
        });
    }

    // -----------------------------------------------------------------
    // 3c. Hooks (Phase 6 / US2) — FIRST among the Phase 6 sinks.
    //
    // The canonical per-harness order is hooks → guardrails → agents.
    // Real-hook reconciliation runs as one pass after the rules/MCP loop
    // (guardrails is US3). Only `RealJson` harnesses with a settings path
    // participate; the enabled-plugin enumeration is shared across every
    // such harness (computed once per sync).
    // -----------------------------------------------------------------
    let hooks_recon = reconcile_hooks(deps, effective_names, &snapshots, &mut outcome)?;
    for decision in &mut outcome.decisions {
        if let Some(action) = hooks_recon.actions.get(&decision.harness) {
            decision.hooks_action = *action;
        }
    }

    // Phase 11: Tome's own SessionStart routing hook for non-RealJson harnesses
    // (Codex). Separate from the plugin-hooks pass above so plugin hooks are never
    // mapped onto Codex. Reuses the `hooks_action` decision field + the hooks error
    // class.
    let (codex_hook_actions, codex_hook_error) =
        reconcile_tome_session_hooks(deps, effective_names, &snapshots, &mut outcome);
    for decision in &mut outcome.decisions {
        if let Some(action) = codex_hook_actions.get(&decision.harness) {
            decision.hooks_action = *action;
        }
    }

    // Phase 11 (G2 / T017): Tome's own session-start `CommandHook` entry for the
    // NEW harnesses (devin / copilot-cli / gemini / antigravity). Excludes
    // claude-code/codex (both `SessionSteering::None`). With every CURRENT module
    // returning `None`, this pass fast-exits as a NO-OP, so the orchestrator
    // output is byte-identical to before — the call site is wired now so US2 only
    // has to add the per-harness `session_steering()` overrides.
    let (command_hook_actions, command_hook_error) = reconcile_command_hooks(
        deps,
        effective_names,
        &snapshots,
        project_root,
        &mut outcome,
    );
    for decision in &mut outcome.decisions {
        if let Some(action) = command_hook_actions.get(&decision.harness) {
            decision.hooks_action = *action;
        }
    }

    // US3 + US6.1: plugin-hook DISPATCH registration — for every harness declaring
    // a `hook_support()` capability, register Tome's match-all `run-hook` entries
    // (used-event-only) into its native hook file AND write the resolved
    // per-(workspace, harness) manifest. SEPARATE leaf from the session-steering
    // entry above (additive — composes, never clobbers), so its action backfills
    // `hooks_action` ONLY when non-`LeftAlone`. Gated by
    // `[hooks] translate_plugin_hooks` (default true / on). The `first_error`
    // joins the precedence chain after `command_hook_error`.
    let hooks_cfg = crate::config::load_or_default(deps.paths);
    let (dispatch_actions, dispatch_error) = reconcile_plugin_hook_dispatch(
        deps,
        &hooks_cfg,
        effective_names,
        &snapshots,
        project_root,
        &mut outcome,
    );
    for decision in &mut outcome.decisions {
        if let Some(action) = dispatch_actions.get(&decision.harness)
            && *action != Action::LeftAlone
        {
            decision.hooks_action = *action;
        }
    }

    // -----------------------------------------------------------------
    // 3c2. Guardrails (Phase 6 / US3) — SECOND among the Phase 6 sinks.
    //
    // Runs AFTER hooks (so the Claude Code suppression predicate reads the
    // fresh hooks-presence set, FR-016) and BEFORE agents. Reconciles each
    // harness's guardrails target (in-file region or Cursor sibling),
    // deduplicating shared `AGENTS.md` targets across harnesses.
    // -----------------------------------------------------------------
    let guardrails_recon = reconcile_guardrails(
        deps,
        effective_names,
        &snapshots,
        &hooks_recon.plugins_with_hooks_json,
        &mut outcome,
    )?;
    for decision in &mut outcome.decisions {
        if let Some(action) = guardrails_recon.actions.get(&decision.harness) {
            decision.guardrails_action = *action;
        }
    }

    // -----------------------------------------------------------------
    // 3d. Agents (Phase 6 / US1).
    //
    // Native-agent reconciliation runs as one pass after hooks because
    // `translate_agent` dispatches through the registry guard, and the DB
    // enumeration + clash-set query are shared across every harness
    // (computed once per sync, FR-072).
    // -----------------------------------------------------------------
    let agents_recon = reconcile_agents(
        project_root,
        deps,
        effective_names,
        &snapshots,
        strip_agent_privileges,
        &mut outcome,
    )?;

    // Backfill each decision's `agents_action` from the per-harness result.
    for decision in &mut outcome.decisions {
        if let Some(action) = agents_recon.actions.get(&decision.harness) {
            decision.agents_action = *action;
        }
    }

    // -----------------------------------------------------------------
    // 3e. Plugins (Phase 11 / G2, T018) — the `TsPlugin` shim sink.
    //
    // Installs / removes Tome's embedded TypeScript session-steering shim for
    // every harness whose `session_steering()` is `TsPlugin`. Runs LAST, after
    // agents. With every CURRENT module returning `SessionSteering::None`, this
    // pass fast-exits as a NO-OP — no shim writes, no decision-field changes,
    // no `Plugins` subsystem entries — so the orchestrator output stays
    // byte-identical to before. The `first_error` is surfaced LAST in the fixed
    // precedence chain below.
    // -----------------------------------------------------------------
    let (plugin_actions, plugin_error) =
        reconcile_plugins(project_root, effective_names, &snapshots, &mut outcome);
    for decision in &mut outcome.decisions {
        if let Some(action) = plugin_actions.get(&decision.harness) {
            decision.plugins_action = *action;
        }
    }

    // -----------------------------------------------------------------
    // 3f. Open Plugins (Phase 11 / US4) — the `tome-op` bundle sink.
    //
    // Reconciles the harnesses partitioned OUT of the per-sink loop
    // (`generic-op` / `goose`): emit the whole bundle atomically for a live
    // harness, remove it (structural-match) for a non-live one. These harnesses
    // never appeared in the per-harness loop above, so they have no
    // `HarnessDecision` yet — push one per op harness here, recording the bundle
    // action under `plugins_action` (the closest existing decision field; the
    // per-bundle granularity lives in `added`/`updated`/`removed` under the
    // `OpenPlugins` subsystem). With no Open Plugins harness in the effective
    // set this loop is empty, so the decision list is byte-identical.
    // -----------------------------------------------------------------
    let (op_actions, op_error) = reconcile_open_plugins(
        project_root,
        deps,
        effective_names,
        &op_snapshots,
        &mut outcome,
    );
    for snap in &op_snapshots {
        outcome.decisions.push(HarnessDecision {
            harness: snap.name.clone(),
            in_effective_list: effective_names.contains(&snap.name),
            rules_action: Action::LeftAlone,
            mcp_action: Action::LeftAlone,
            agents_action: Action::LeftAlone,
            hooks_action: Action::LeftAlone,
            guardrails_action: Action::LeftAlone,
            plugins_action: op_actions
                .get(&snap.name)
                .copied()
                .unwrap_or(Action::LeftAlone),
        });
    }

    if let Some(clash) = first_clash {
        return Err(clash);
    }
    // Surface failures in the fixed sink order hooks → guardrails → agents
    // (the earlier sink's error wins; forward progress means later sinks still
    // reconciled where possible before we return here).
    if let Some(hooks_err) = hooks_recon.first_error {
        return Err(hooks_err);
    }
    if let Some(codex_hook_err) = codex_hook_error {
        return Err(codex_hook_err);
    }
    if let Some(command_hook_err) = command_hook_error {
        return Err(command_hook_err);
    }
    // US3: the plugin-hook dispatch sink shares the hooks error classes (43/44)
    // and is surfaced right after the session-steering command-hook sink, before
    // guardrails/agents — keeping the fixed hooks-before-guardrails precedence.
    if let Some(dispatch_err) = dispatch_error {
        return Err(dispatch_err);
    }
    if let Some(guardrails_err) = guardrails_recon.first_error {
        return Err(guardrails_err);
    }
    if let Some(agent_err) = agents_recon.first_error {
        return Err(agent_err);
    }
    // Phase 11 / G2: the TsPlugin shim sink is surfaced LAST (after agents) in
    // the fixed precedence chain — every earlier sink's error wins, and forward
    // progress means the shim pass still reconciled where it could before this
    // error is returned.
    if let Some(plugin_err) = plugin_error {
        return Err(plugin_err);
    }
    // Phase 11 / US4: the Open Plugins bundle sink is surfaced LAST — every
    // earlier sink's error wins, and forward progress means the bundle pass
    // still reconciled where it could before this error is returned.
    if let Some(op_err) = op_error {
        return Err(op_err);
    }

    Ok(outcome)
}

// =====================================================================
// Harness-snapshot helpers
// =====================================================================

/// Per-harness data captured from the registry into owned values so
/// the rest of the orchestrator runs without holding the registry's
/// read guard.
///
/// `pub(crate)`, along with the fields the per-sink reconcilers under
/// [`crate::harness::reconcile`] read, so they can name it in their signatures
/// after the Phase 7 decomposition (FR-011).
pub(crate) struct HarnessSnapshot {
    pub(crate) name: String,
    rules_path: PathBuf,
    rules_strategy: RulesFileStrategy,
    /// Phase 11 (G3, FR-026): the Tome-owned YAML front-matter header written
    /// ABOVE the verbatim directive on a `StandaloneFile` sink (kiro's
    /// `inclusion: always`, jetbrains-ai's apply-mode marker). `None` for every
    /// harness without a front-matter requirement — every Phase ≤10 module, so
    /// their standalone bytes are unchanged. Consulted ONLY in the
    /// `StandaloneFile` write branch; the clean branch removes the whole file
    /// regardless.
    rules_frontmatter: Option<crate::harness::RulesFrontmatter>,
    block_body_style: BlockBodyStyle,
    mcp_path: PathBuf,
    /// Phase 11 (G1): the harness's full MCP wire-shape, replacing the
    /// Phase ≤10 `mcp_format` + `mcp_parent_key` scalar pair. Drives the
    /// shared dialect-aware `mcp_config` read/write/remove.
    mcp_dialect: crate::harness::McpDialect,
    /// Phase 11: `true` when the harness has NO writable MCP config file
    /// (jetbrains-ai — UI-only). The MCP read/write/remove sink is skipped
    /// entirely for such a harness; `false` for every other module, so the
    /// MCP byte output is unchanged for all writable harnesses.
    mcp_manual_only: bool,
    /// Phase 6 / US1: whether this harness emits native agent files. Drives
    /// the agents-reconciliation fast-exit; the actual `agent_dir` is
    /// re-derived under the registry guard at dispatch time (the trait
    /// dispatch for `translate_agent` already holds the guard).
    pub(crate) supports_native_agents: bool,
    /// Phase 6 / US2: the harness's machine-local hook settings file, when it
    /// has a `RealJson` hooks strategy. `None` for every `GuardrailsOnly`
    /// harness (no real-hook participation; the guardrails fallback is US3).
    /// `pub(crate)` so the hooks reconciler reads it across the module boundary.
    pub(crate) hook_settings_path: Option<PathBuf>,
    /// Phase 11: the JSON sink for Tome's OWN session-start routing hook on a
    /// non-`RealJson` harness (Codex → `<project>/.codex/hooks.json`). `None`
    /// for harnesses with no Tome-owned session hook (or whose Tome hook rides
    /// the `RealJson` pass, e.g. Claude Code).
    pub(crate) tome_session_hook_path: Option<PathBuf>,
    /// Phase 11 (G2): how this harness receives Tome's session-start steering
    /// directive. Drives the `CommandHook` reconciler's fast-exit + per-harness
    /// write/remove. Every Phase ≤10 module returns
    /// [`crate::harness::SessionSteering::None`], so the reconciler is a no-op
    /// for them and the orchestrator output stays byte-identical.
    pub(crate) session_steering: crate::harness::SessionSteering,
    /// US3: this harness's plugin-hook translation capability (`None` for a
    /// guardrails-only harness). Drives the dispatch reconciler's fast-exit +
    /// per-harness `run-hook` registration + manifest write. `pub(crate)` so the
    /// reconciler reads it across the module boundary.
    pub(crate) hook_support: Option<crate::harness::HookSupport>,
    /// US3: the harness-NATIVE event-name for each supported event, RESOLVED at
    /// snapshot time. `hook_event_name` is `&self`, so capturing the names here
    /// lets the dispatch reconciler / manifest builder register under e.g.
    /// gemini's `BeforeTool` without re-entering the registry. Empty when
    /// `hook_support` is `None`.
    pub(crate) hook_event_names: Vec<(crate::harness::hooks_ir::PortableEvent, &'static str)>,
    /// Phase 6 / US3: the harness's guardrails sink (in-file region or Cursor
    /// standalone sibling) plus its hooks-driven suppression flag.
    pub(crate) guardrails_target: crate::harness::GuardrailsTarget,
    /// Phase 11 / US4: the Open Plugins `tome-op` bundle root, when this harness
    /// integrates via the atomic `open_plugins` emitter (`generic-op`, `goose`)
    /// instead of the per-sink loop. `Some(root)` means the harness is dispatched
    /// to [`crate::harness::reconcile::open_plugins::reconcile_open_plugins`] and
    /// is EXCLUDED from the rules/MCP/hooks/guardrails/agents/plugins passes
    /// (the bundle is all-or-nothing; the per-sink loop must not double-write its
    /// `AGENTS.md` / `.mcp.json`). `None` for every other harness, so their sink
    /// processing is byte-identical.
    pub(crate) open_plugins_root: Option<PathBuf>,
}

fn collect_harness_snapshots(
    project_root: &Path,
    deps: &SyncDeps<'_>,
    effective_names: &HashSet<String>,
) -> Vec<HarnessSnapshot> {
    let mut snapshots = with_effective_modules(|mods| {
        mods.iter()
            // `only_harness` restricts the reconcile to the named harness SET
            // (for `tome sync --harness a --harness b`): only modules in the set
            // are snapshotted, so every downstream dedup map + the is-live
            // write-vs-remove decision operate over that subset and every OTHER
            // harness's files are left completely untouched. This is the SINGLE
            // filter point — every sink derives its scope from these snapshots
            // (the rules/MCP/hooks/guardrails loops iterate them directly; the
            // agents sink gates its registry walk on the snapshotted name set),
            // so the "other harnesses untouched" guarantee holds across ALL
            // sinks. `None` snapshots the full registry (the default full
            // reconcile).
            .filter(|m| match deps.only_harness.as_ref() {
                Some(set) => set.contains(m.name()),
                None => true,
            })
            .map(|m| snapshot_for(*m, project_root, deps.home_root))
            .collect::<Vec<_>>()
    });
    snapshots.extend(collect_opt_in_snapshots(
        project_root,
        deps,
        effective_names,
    ));
    snapshots
}

/// Snapshot every OPT-IN target (`generic` / `generic-op`) that is EXPLICITLY
/// in `effective_names`, OR whose Tome-managed artifact currently exists on disk
/// (the removal path), honouring `only_harness` (Phase 11 / US4, B1).
///
/// Opt-in targets (`OPT_IN_TARGETS`) live OUTSIDE `SUPPORTED_HARNESSES` and so
/// are never returned by [`with_effective_modules`] — without this union an
/// explicitly-selected `generic` / `generic-op` produces no snapshot and the
/// sinks (the open-plugins partition included) see nothing, so the bundle /
/// AGENTS.md is never written. [`HarnessModule::is_opt_in_target`] is the
/// load-bearing gate.
///
/// CRITICAL — the WRITE side is explicit-selection-only: an opt-in target is
/// snapshotted-as-LIVE ONLY when its `name()` is in `effective_names`. It is
/// NEVER pulled in from detection or `--all` — neither path puts it in the
/// effective list (its `detect` is inert).
///
/// The REMOVE side: a target that was previously selected leaves a managed
/// artifact on disk (the `tome-op` bundle for an op target; `generic`'s own
/// `mcp.json`). To clean it after the user drops the harness — when it is NO
/// LONGER in `effective_names` — we ALSO snapshot a target whose artifact is
/// present. Such a non-live snapshot dispatches through the REMOVE branch
/// (structural-match for the bundle; tome-owned-only for the MCP entry), so a
/// directory/entry Tome doesn't own is left untouched. The artifact probe is
/// tight (the target's OWN file, never a co-owned `AGENTS.md`), so when nothing
/// was ever written the returned `Vec` is empty and the snapshot set — and every
/// downstream sink — is byte-identical to before.
fn collect_opt_in_snapshots(
    project_root: &Path,
    deps: &SyncDeps<'_>,
    effective_names: &HashSet<String>,
) -> Vec<HarnessSnapshot> {
    crate::harness::OPT_IN_TARGETS
        .iter()
        .copied()
        // Load-bearing: only an opt-in target participates here.
        .filter(|m| m.is_opt_in_target())
        // Snapshot when EXPLICITLY selected (the write path) OR when its managed
        // artifact is present (the remove-after-drop path). Detection / `--all`
        // never reach here — they don't put an opt-in target in `effective_names`,
        // and the artifact probe is the target's own Tome-written file.
        .filter(|m| {
            effective_names.contains(m.name())
                || opt_in_artifact_present(*m, project_root, deps.home_root)
        })
        // Honour `--harness <name>...`: a filtered reconcile still touches
        // only the named harnesses, opt-in targets included.
        .filter(|m| match deps.only_harness.as_ref() {
            Some(set) => set.contains(m.name()),
            None => true,
        })
        .map(|m| snapshot_for(m, project_root, deps.home_root))
        .collect()
}

/// Does this opt-in target have a Tome-managed artifact on disk (so it needs the
/// remove-after-drop pass even when no longer in the effective list)?
///
/// Probes the target's OWN file only — never a co-owned `AGENTS.md` (which other
/// harnesses may legitimately own), so a project that never selected this target
/// returns `false` and the snapshot set stays byte-identical:
///
/// - op target (`open_plugins_root == Some`) → the bundle root exists.
/// - `generic` → its dedicated `<project>/mcp.json` exists.
fn opt_in_artifact_present(m: &dyn HarnessModule, project_root: &Path, home_root: &Path) -> bool {
    if let Some(root) = m.open_plugins_root(project_root) {
        return root.exists();
    }
    // The standard-sink opt-in target (`generic`): probe its own MCP file, the
    // one artifact it exclusively owns (its rules sink is the shared AGENTS.md).
    m.mcp_config_path(project_root, home_root).exists()
}

/// Snapshot the FULL effective registry, ignoring `only_harness`. Used solely
/// to compute the body-style LCD + live-sharer set for SHARED rules paths under
/// `tome sync --harness <name>`: a shared rules file's content must stay correct
/// for every live co-owner, even ones not being reconciled this pass. The main
/// per-harness write/leave-alone loop still iterates the FILTERED snapshots, so
/// non-shared sinks for other harnesses stay untouched.
fn collect_all_harness_snapshots(
    project_root: &Path,
    deps: &SyncDeps<'_>,
    effective_names: &HashSet<String>,
) -> Vec<HarnessSnapshot> {
    let mut snapshots = with_effective_modules(|mods| {
        mods.iter()
            .map(|m| snapshot_for(*m, project_root, deps.home_root))
            .collect::<Vec<_>>()
    });
    // Union the explicitly-selected opt-in targets, IGNORING `only_harness` (this
    // pass exists precisely to compute the shared-rules-path body for every live
    // co-owner, including ones not being reconciled this `--harness` pass). The
    // explicit-selection filter inside `collect_opt_in_snapshots` is honoured by
    // passing a deps clone with `only_harness = None`.
    let unfiltered_deps = SyncDeps {
        paths: deps.paths,
        home_root: deps.home_root,
        workspace_name: deps.workspace_name,
        force: deps.force,
        only_harness: None,
    };
    snapshots.extend(collect_opt_in_snapshots(
        project_root,
        &unfiltered_deps,
        effective_names,
    ));
    // Open Plugins harnesses never share a per-sink rules path (their `AGENTS.md`
    // lives inside the bundle), so they have no business in the shared-rules-path
    // grouping. Excluding them keeps the grouping source consistent with the
    // partitioned per-sink `snapshots`. This also drops `generic-op` (an
    // open-plugins opt-in target) while keeping `generic` (a standard-sink
    // opt-in target that DOES share `<project>/AGENTS.md`).
    snapshots.retain(|s| s.open_plugins_root.is_none());
    snapshots
}

/// The single source of truth for a harness's rules sink path (PW5).
///
/// Prefers the harness's dedicated, namespaced standalone file when it declares
/// one ([`HarnessModule::rules_namespaced_file`]) — so Tome never inserts a
/// block into (or owns) a developer-authored shared rules file — and otherwise
/// falls back to [`HarnessModule::rules_file_target`]. For the namespaced
/// overriders (cline/zed/kiro/jetbrains-ai) the namespaced accessor returns the
/// SAME path as the target today, so behaviour is unchanged; routing every
/// consumer through this ONE function makes the accessor load-bearing and means
/// the sync snapshot and any teardown / doctor path can never disagree about
/// which file a harness's rules content lives in.
pub(crate) fn rules_sink_path(m: &dyn HarnessModule, project_root: &Path) -> PathBuf {
    m.rules_namespaced_file(project_root)
        .unwrap_or_else(|| m.rules_file_target(project_root))
}

fn snapshot_for(m: &dyn HarnessModule, project_root: &Path, home_root: &Path) -> HarnessSnapshot {
    // US3: capture the harness's hook capability + resolve each supported event's
    // native name ONCE here (`hook_event_name` is `&self`), so the dispatch
    // reconciler never re-enters the registry.
    let hook_support = m.hook_support();
    let hook_event_names = hook_support
        .as_ref()
        .map(|s| {
            s.events
                .iter()
                .map(|&e| (e, m.hook_event_name(e)))
                .collect()
        })
        .unwrap_or_default();
    HarnessSnapshot {
        name: m.name().to_string(),
        // Phase 11 (G3, FR-024): the harness's dedicated rules sink, computed via
        // the [`rules_sink_path`] SSOT so every consumer (this snapshot + any
        // teardown / doctor path) resolves the SAME path and cannot drift.
        rules_path: rules_sink_path(m, project_root),
        rules_strategy: m.rules_file_strategy(),
        rules_frontmatter: m.rules_frontmatter(),
        block_body_style: m.block_body_style(),
        mcp_path: m.mcp_config_path(project_root, home_root),
        mcp_dialect: m.mcp_dialect(),
        mcp_manual_only: m.mcp_manual_only(),
        supports_native_agents: m.supports_native_agents(),
        // Only a `RealJson` harness with a settings path participates in real
        // hooks. A `GuardrailsOnly` harness — even one that returns a settings
        // path — is a no-op here and falls back to guardrails (US3).
        hook_settings_path: match m.hooks_strategy() {
            crate::harness::HooksStrategy::RealJson => m.hook_settings_path(project_root),
            crate::harness::HooksStrategy::GuardrailsOnly => None,
        },
        tome_session_hook_path: m.tome_session_hook_path(project_root),
        session_steering: m.session_steering(),
        hook_support,
        hook_event_names,
        guardrails_target: m.guardrails_target(project_root),
        open_plugins_root: m.open_plugins_root(project_root),
    }
}

/// Test-only constructor exposing the private [`snapshot_for`] to in-crate
/// unit tests (e.g. the agents-sink mass-delete safeguard guard in
/// `crate::harness::reconcile::agents`). Builds a faithful [`HarnessSnapshot`]
/// from a real [`HarnessModule`] via the same path the orchestrator uses, so
/// the test drives the genuine field set rather than fabricating values. Not
/// compiled into the production binary.
#[cfg(test)]
pub(crate) fn snapshot_for_test(
    m: &dyn HarnessModule,
    project_root: &Path,
    home_root: &Path,
) -> HarnessSnapshot {
    snapshot_for(m, project_root, home_root)
}

/// Group snapshots by some key path so the deduplication logic for
/// FR-482 / FR-483 can answer "who else targets this same path?".
fn group_by_path<F>(
    snapshots: &[HarnessSnapshot],
    key_of: F,
) -> BTreeMap<PathBuf, Vec<&HarnessSnapshot>>
where
    F: Fn(&HarnessSnapshot) -> &PathBuf,
{
    let mut out: BTreeMap<PathBuf, Vec<&HarnessSnapshot>> = BTreeMap::new();
    for snap in snapshots {
        out.entry(key_of(snap).clone()).or_default().push(snap);
    }
    out
}

// =====================================================================
// Settings reads
// =====================================================================
//
// R4-2: the workspace + global loaders are promoted to
// `settings::scopes` (the single source for the NotFound/parse-error
// arms). These thin wrappers adapt the orchestrator's `SyncDeps` shape
// to the promoted loaders' `(paths, workspace_name)` parameters.

fn read_workspace_settings(deps: &SyncDeps<'_>) -> Result<Option<WorkspaceSettings>, TomeError> {
    crate::settings::scopes::load_workspace_settings(deps.paths, deps.workspace_name)
}

fn read_global_settings(deps: &SyncDeps<'_>) -> Result<GlobalSettings, TomeError> {
    crate::settings::scopes::load_global_settings(deps.paths)
}

// =====================================================================
// Rules-file dispatch
// =====================================================================

/// Lowest-common-denominator body style across the live harnesses sharing one
/// rules path (F-RULES-OPENCODE, §R-8).
///
/// `Inline` wins the moment ANY live sharer requires it: an inline body is the
/// only form every sharer can consume, because a not-include-capable harness
/// (OpenCode) reads a `@.tome/RULES.md` directive as literal prose. An
/// include-capable harness resolves an inline body without issue, so `Inline`
/// is the safe floor; a group with no inline sharer keeps `AtInclude`.
///
/// `block_body_style()` is the source of truth — no harness name is hard-coded.
/// Mirrors the union-across-sharers in
/// [`crate::harness::reconcile::guardrails::reconcile_guardrails`].
fn group_body_style(live_sharers: &[&HarnessSnapshot]) -> BlockBodyStyle {
    if live_sharers
        .iter()
        .any(|s| s.block_body_style == BlockBodyStyle::Inline)
    {
        BlockBodyStyle::Inline
    } else {
        BlockBodyStyle::AtInclude
    }
}

/// Compute the block body for the given resolved [`BlockBodyStyle`]. The result
/// is the bytes that will land between the `<!-- tome:begin -->` /
/// `<!-- tome:end -->` markers for `BlockInExistingFile`, or the full file
/// contents for `StandaloneFile`.
///
/// `style` is the GROUP's lowest-common-denominator style (see
/// [`group_body_style`]), NOT necessarily the writing snapshot's own — a shared
/// path with any inline sharer is written inline so every sharer can read it.
///
/// Returns an error if reading the project marker's `RULES.md` fails
/// for any reason other than `NotFound` — absent is fine (US2 / US4
/// own the file, sync is robust to its absence), but a permissions or
/// I/O failure must surface rather than silently produce an empty block.
fn compute_rules_body(
    style: BlockBodyStyle,
    rules_path: &Path,
    project_root: &Path,
) -> Result<String, TomeError> {
    match style {
        BlockBodyStyle::AtInclude => {
            let project_rules = Paths::project_marker_rules(project_root);
            // All sharers of a group target the same `rules_path` (the grouping
            // key), so the include directive's relative path is unambiguous.
            let parent = rules_path.parent().unwrap_or(Path::new(""));
            let relative = relative_path(parent, &project_rules);
            Ok(format!("@{}", relative.display()))
        }
        BlockBodyStyle::Inline => {
            // Inline body is the verbatim contents of
            // `<project>/.tome/RULES.md`. Absent → empty block; other
            // I/O errors propagate.
            let project_rules = Paths::project_marker_rules(project_root);
            match crate::util::bounded_read_to_string(
                &project_rules,
                crate::util::HARNESS_RULES_MAX,
            ) {
                Ok(s) => Ok(s),
                Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                    Ok(String::new())
                }
                Err(e) => Err(e),
            }
        }
    }
}

fn write_rules_for_path(snap: &HarnessSnapshot, body: &str) -> Result<Action, TomeError> {
    match snap.rules_strategy {
        RulesFileStrategy::BlockInExistingFile => {
            // Classify before write so we can distinguish Created vs
            // Updated vs LeftAlone in the outcome.
            let prior = match crate::util::bounded_read_to_string(
                &snap.rules_path,
                crate::util::HARNESS_RULES_MAX,
            ) {
                Ok(s) => Some(s),
                Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e),
            };
            let prior_block = match prior.as_deref() {
                Some(contents) => rules_file::parse_block(contents)?,
                None => None,
            };
            let classification = classify_block(&prior_block, body);

            rules_file::write_block(&snap.rules_path, body, snap.block_body_style)?;
            Ok(classification)
        }
        RulesFileStrategy::StandaloneFile => {
            // Phase 11 (G3, FR-026): a harness declaring `rules_frontmatter()`
            // (kiro `inclusion: always`, jetbrains-ai apply-mode) gets a
            // Tome-owned `---`-fenced header ABOVE the directive; every other
            // harness (every Phase ≤10 module, all returning `None`) writes the
            // verbatim body with no header, so their standalone bytes are
            // byte-identical. The on-disk comparison classifies against the FULLY
            // RENDERED payload so the idempotence + Created/Updated/LeftAlone
            // distinction stays correct for both shapes.
            let desired = match snap.rules_frontmatter {
                Some(fm) => rules_file::render_standalone_with_frontmatter(&fm, body),
                None => body.to_string(),
            };
            let prior_bytes = match crate::util::bounded_read_to_string(
                &snap.rules_path,
                crate::util::HARNESS_RULES_MAX,
            ) {
                Ok(s) => Some(s),
                Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e),
            };
            let classification = match prior_bytes.as_deref() {
                None => Action::Created,
                Some(existing) if existing == desired => Action::LeftAlone,
                Some(_) => Action::Updated,
            };
            match snap.rules_frontmatter {
                Some(fm) => {
                    rules_file::write_standalone_with_frontmatter(&snap.rules_path, &fm, body)?
                }
                None => rules_file::write_standalone(&snap.rules_path, body)?,
            }
            Ok(classification)
        }
    }
}

fn clean_rules_for_path(snap: &HarnessSnapshot) -> Result<Action, TomeError> {
    match snap.rules_strategy {
        RulesFileStrategy::BlockInExistingFile => {
            let prior = match crate::util::bounded_read_to_string(
                &snap.rules_path,
                crate::util::HARNESS_RULES_MAX,
            ) {
                Ok(s) => Some(s),
                Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Action::LeftAlone);
                }
                Err(e) => return Err(e),
            };
            let had_block = match prior.as_deref() {
                Some(contents) => rules_file::parse_block(contents)?.is_some(),
                None => false,
            };
            if had_block {
                rules_file::remove_block(&snap.rules_path)?;
                Ok(Action::Removed)
            } else {
                Ok(Action::LeftAlone)
            }
        }
        RulesFileStrategy::StandaloneFile => {
            if snap.rules_path.exists() {
                rules_file::remove_standalone(&snap.rules_path)?;
                Ok(Action::Removed)
            } else {
                Ok(Action::LeftAlone)
            }
        }
    }
}

fn classify_block(prior: &Option<rules_file::ParsedBlock>, new_body: &str) -> Action {
    match prior {
        None => Action::Created,
        Some(block) if block.body == new_body => Action::LeftAlone,
        Some(_) => Action::Updated,
    }
}

// =====================================================================
// MCP config dispatch
// =====================================================================

fn write_mcp_for_harness(snap: &HarnessSnapshot, deps: &SyncDeps<'_>) -> Result<Action, TomeError> {
    let existing = mcp_config::read_entry(&snap.mcp_path, &snap.mcp_dialect)?;

    if let Some(current) = existing.as_ref()
        && !mcp_config::is_tome_owned(current)
        && !deps.force
    {
        return Err(TomeError::HarnessClash {
            path: snap.mcp_path.clone(),
            command: current.command.clone(),
            first_arg: current.args.first().cloned().unwrap_or_default(),
        });
    }

    // #337: emit the ABSOLUTE launcher (`$TOME_BIN` → `current_exe` → bare
    // `tome`) instead of a bare-PATH `"tome"`, so a PATH-less / sandboxed host
    // (CI, a non-IDE agent) can still start the MCP server. Ownership stays
    // recognisable because `is_tome_owned` now matches the launcher BASENAME
    // (`looks_like_tome_launcher`), not the exact string — so idempotence /
    // clash / removal survive a per-machine launcher.
    //
    // INTENTIONAL self-healing on launcher drift: the stored `command` is
    // rewritten to the CURRENTLY-running binary's path, so the launcher on disk
    // always points at a binary that actually exists. If the same workspace is
    // later synced by a `tome` at a DIFFERENT absolute path (Homebrew
    // relocation, a symlink vs its resolved target, cargo-install vs a system
    // package), the idempotence comparison below sees `current.command !=
    // expected.command` and performs ONE benign rewrite to the new path —
    // classified `Updated`, still owned (no clash), no orphan. This is correct,
    // not a churning bug: each machine converges its stored launcher to its own
    // valid binary and then stays idempotent. (Do NOT "fix" this by pinning a
    // bare name — that reintroduces the PATH-less-host failure #337 closes.)
    //
    // Phase 9 / US3 / FR-030: stamp `--harness <name>` so the running
    // `tome mcp` server knows which harness hosts it (the built-in `meta`
    // tool resolves the install target from it). It is a LATER arg, so the
    // ownership marker (basename `tome` + `args[0] == "mcp"`) is preserved; an
    // existing entry without it re-stamps as `Updated` on the next sync
    // (idempotent thereafter).
    let expected = mcp_config::TomeEntry::new(
        crate::harness::launcher::tome_command(),
        vec![
            "mcp".to_string(),
            "--workspace".to_string(),
            deps.workspace_name.as_str().to_string(),
            "--harness".to_string(),
            snap.name.clone(),
        ],
    );

    let classification = match existing.as_ref() {
        None => Action::Created,
        Some(current)
            if mcp_config::is_tome_owned(current)
                && current.command == expected.command
                && current.args == expected.args =>
        {
            Action::LeftAlone
        }
        Some(_) => Action::Updated,
    };

    mcp_config::write_entry(&snap.mcp_path, &snap.mcp_dialect, &expected)?;
    Ok(classification)
}

fn clean_mcp_for_harness(snap: &HarnessSnapshot) -> Result<Action, TomeError> {
    let existing = mcp_config::read_entry(&snap.mcp_path, &snap.mcp_dialect)?;
    let was_tome = matches!(existing.as_ref(), Some(e) if mcp_config::is_tome_owned(e));
    if !was_tome {
        return Ok(Action::LeftAlone);
    }
    mcp_config::remove_entry(&snap.mcp_path, &snap.mcp_dialect)?;
    Ok(Action::Removed)
}

// =====================================================================
// Path helpers
// =====================================================================

/// Compute the relative path from `from` (a directory) to `to` (a file
/// or directory). Handles the common cases needed by the sync orchestrator:
///
/// - `from = /proj`, `to = /proj/.tome/RULES.md` → `.tome/RULES.md`
/// - `from = /proj/.claude`, `to = /proj/.tome/RULES.md` → `../.tome/RULES.md`
///
/// Falls back to an absolute path when `from` and `to` are on different
/// roots (unlikely in practice but harmless). Implementation walks
/// canonical-component prefix length to keep the result short.
///
/// Component-prefix walk over [`std::path::Component`] rather than
/// allocating a `pathdiff`-style helper crate — Tome's dependency
/// surface stays trimmed.
fn relative_path(from: &Path, to: &Path) -> PathBuf {
    let mut from_parts: Vec<Component<'_>> = from.components().collect();
    let mut to_parts: Vec<Component<'_>> = to.components().collect();

    // Strip the common prefix.
    let mut common = 0;
    while common < from_parts.len()
        && common < to_parts.len()
        && from_parts[common] == to_parts[common]
    {
        common += 1;
    }

    from_parts.drain(..common);
    to_parts.drain(..common);

    // If the roots differ entirely, the loop above stops at `common = 0`.
    // Falling back to the absolute target keeps the call site honest.
    if common == 0 {
        return to.to_path_buf();
    }

    let mut buf = PathBuf::new();
    for _ in &from_parts {
        buf.push("..");
    }
    for c in &to_parts {
        buf.push(c.as_os_str());
    }
    if buf.as_os_str().is_empty() {
        buf.push(".");
    }
    buf
}

// =====================================================================
// Helper accessors for sibling modules
// =====================================================================

/// Bridge accessor for the `commands::harness` shim. `BindDeps` doesn't
/// carry the bound workspace name (it's only known after `bind_project`
/// returns), so the CLI seam constructs the `SyncDeps` from a separate
/// signature; this module's public surface is `SyncDeps` only.
pub(crate) fn build_deps<'a>(
    paths: &'a Paths,
    home_root: &'a Path,
    workspace_name: &'a WorkspaceName,
    force: bool,
) -> SyncDeps<'a> {
    SyncDeps {
        paths,
        home_root,
        workspace_name,
        force,
        only_harness: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn relative_path_inside_parent() {
        let from = PathBuf::from("/proj");
        let to = PathBuf::from("/proj/.tome/RULES.md");
        assert_eq!(relative_path(&from, &to), PathBuf::from(".tome/RULES.md"));
    }

    #[test]
    fn relative_path_one_level_up() {
        let from = PathBuf::from("/proj/.claude");
        let to = PathBuf::from("/proj/.tome/RULES.md");
        assert_eq!(
            relative_path(&from, &to),
            PathBuf::from("../.tome/RULES.md")
        );
    }

    #[test]
    fn relative_path_same_directory() {
        let from = PathBuf::from("/proj");
        let to = PathBuf::from("/proj");
        assert_eq!(relative_path(&from, &to), PathBuf::from("."));
    }
}

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
use crate::harness::agents::{self, CanonicalAgent};
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

    // -----------------------------------------------------------------
    // 3. Walk every harness in the effective registry.
    //
    // The registry view is captured *outside* `with_effective_modules`
    // so the read guard does not span the long FS work — we capture the
    // per-harness metadata into owned values up front, then drop the
    // borrow before dispatch.
    // -----------------------------------------------------------------
    let snapshots = collect_harness_snapshots(project_root, deps);
    let mut outcome = SyncOutcome::default();

    // Build the dedup maps for shared rules-file / MCP paths.
    let rules_targets_by_path = group_by_path(&snapshots, |s| &s.rules_path);
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
            Action::LeftAlone
        } else {
            // The "live" decision for a shared path is OR-of-live across
            // every harness that targets it: as long as ANY harness in
            // the effective list still wants this path, the block stays.
            let any_live = rules_targets_by_path
                .get(&snap.rules_path)
                .map(|sharers| sharers.iter().any(|s| effective_names.contains(&s.name)))
                .unwrap_or(false);
            if any_live {
                let body = compute_rules_body(snap, project_root)?;
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
        let mcp_action = if !mcp_paths_processed.insert(snap.mcp_path.clone()) {
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
            // Backfilled by the hooks + guardrails + agents reconciliation
            // passes below.
            agents_action: Action::LeftAlone,
            hooks_action: Action::LeftAlone,
            guardrails_action: Action::LeftAlone,
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
    let hooks_recon = reconcile_hooks(deps, &effective_names, &snapshots, &mut outcome)?;
    for decision in &mut outcome.decisions {
        if let Some(action) = hooks_recon.actions.get(&decision.harness) {
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
        &effective_names,
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
        &effective_names,
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

    if let Some(clash) = first_clash {
        return Err(clash);
    }
    // Surface failures in the fixed sink order hooks → guardrails → agents
    // (the earlier sink's error wins; forward progress means later sinks still
    // reconciled where possible before we return here).
    if let Some(hooks_err) = hooks_recon.first_error {
        return Err(hooks_err);
    }
    if let Some(guardrails_err) = guardrails_recon.first_error {
        return Err(guardrails_err);
    }
    if let Some(agent_err) = agents_recon.first_error {
        return Err(agent_err);
    }

    Ok(outcome)
}

// =====================================================================
// Harness-snapshot helpers
// =====================================================================

/// Per-harness data captured from the registry into owned values so
/// the rest of the orchestrator runs without holding the registry's
/// read guard.
struct HarnessSnapshot {
    name: String,
    rules_path: PathBuf,
    rules_strategy: RulesFileStrategy,
    block_body_style: BlockBodyStyle,
    mcp_path: PathBuf,
    mcp_format: crate::harness::McpConfigFormat,
    mcp_parent_key: &'static str,
    /// Phase 6 / US1: whether this harness emits native agent files. Drives
    /// the agents-reconciliation fast-exit; the actual `agent_dir` is
    /// re-derived under the registry guard at dispatch time (the trait
    /// dispatch for `translate_agent` already holds the guard).
    supports_native_agents: bool,
    /// Phase 6 / US2: the harness's machine-local hook settings file, when it
    /// has a `RealJson` hooks strategy. `None` for every `GuardrailsOnly`
    /// harness (no real-hook participation; the guardrails fallback is US3).
    hook_settings_path: Option<PathBuf>,
    /// Phase 6 / US3: the harness's guardrails sink (in-file region or Cursor
    /// standalone sibling) plus its hooks-driven suppression flag.
    guardrails_target: crate::harness::GuardrailsTarget,
}

fn collect_harness_snapshots(project_root: &Path, deps: &SyncDeps<'_>) -> Vec<HarnessSnapshot> {
    with_effective_modules(|mods| {
        mods.iter()
            .map(|m| snapshot_for(*m, project_root, deps.home_root))
            .collect()
    })
}

fn snapshot_for(m: &dyn HarnessModule, project_root: &Path, home_root: &Path) -> HarnessSnapshot {
    HarnessSnapshot {
        name: m.name().to_string(),
        rules_path: m.rules_file_target(project_root),
        rules_strategy: m.rules_file_strategy(),
        block_body_style: m.block_body_style(),
        mcp_path: m.mcp_config_path(project_root, home_root),
        mcp_format: m.mcp_config_format(),
        mcp_parent_key: m.mcp_parent_key(),
        supports_native_agents: m.supports_native_agents(),
        // Only a `RealJson` harness with a settings path participates in real
        // hooks. A `GuardrailsOnly` harness — even one that returns a settings
        // path — is a no-op here and falls back to guardrails (US3).
        hook_settings_path: match m.hooks_strategy() {
            crate::harness::HooksStrategy::RealJson => m.hook_settings_path(project_root),
            crate::harness::HooksStrategy::GuardrailsOnly => None,
        },
        guardrails_target: m.guardrails_target(project_root),
    }
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

/// Compute the block body for one snapshot. The result is the bytes
/// that will land between the `<!-- tome:begin -->` / `<!-- tome:end -->`
/// markers for `BlockInExistingFile`, or the full file contents for
/// `StandaloneFile`.
///
/// Returns an error if reading the project marker's `RULES.md` fails
/// for any reason other than `NotFound` — absent is fine (US2 / US4
/// own the file, sync is robust to its absence), but a permissions or
/// I/O failure must surface rather than silently produce an empty block.
fn compute_rules_body(snap: &HarnessSnapshot, project_root: &Path) -> Result<String, TomeError> {
    match snap.block_body_style {
        BlockBodyStyle::AtInclude => {
            let project_rules = Paths::project_marker_rules(project_root);
            let parent = snap.rules_path.parent().unwrap_or(Path::new(""));
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
                Some(existing) if existing == body => Action::LeftAlone,
                Some(_) => Action::Updated,
            };
            rules_file::write_standalone(&snap.rules_path, body)?;
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
    let existing = mcp_config::read_entry(&snap.mcp_path, snap.mcp_format, snap.mcp_parent_key)?;

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

    let expected = mcp_config::TomeEntry::new(
        "tome".to_string(),
        vec![
            "mcp".to_string(),
            "--workspace".to_string(),
            deps.workspace_name.as_str().to_string(),
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

    mcp_config::write_entry(
        &snap.mcp_path,
        snap.mcp_format,
        snap.mcp_parent_key,
        &expected,
    )?;
    Ok(classification)
}

fn clean_mcp_for_harness(snap: &HarnessSnapshot) -> Result<Action, TomeError> {
    let existing = mcp_config::read_entry(&snap.mcp_path, snap.mcp_format, snap.mcp_parent_key)?;
    let was_tome = matches!(existing.as_ref(), Some(e) if mcp_config::is_tome_owned(e));
    if !was_tome {
        return Ok(Action::LeftAlone);
    }
    mcp_config::remove_entry(&snap.mcp_path, snap.mcp_format, snap.mcp_parent_key)?;
    Ok(Action::Removed)
}

// =====================================================================
// Real-hooks reconciliation (Phase 6 / US2)
// =====================================================================

/// Result of the real-hooks reconciliation pass. Mirrors
/// [`AgentReconciliation`]: a per-harness aggregate action map keyed on
/// `name()`, plus the FIRST failure encountered (forward progress).
struct HooksReconciliation {
    actions: std::collections::HashMap<String, Action>,
    first_error: Option<TomeError>,
    /// Phase 6 / US3 (FR-013, FR-016): the `<catalog>:<plugin>` keys of every
    /// enabled plugin that ships a `hooks/hooks.json`. Computed in the hooks
    /// pass (which runs FIRST) so the Claude Code guardrails suppression
    /// predicate never reads stale state. A plugin in this set has its
    /// `CLAUDE.md` guardrails region suppressed (real hooks supersede prose).
    plugins_with_hooks_json: HashSet<String>,
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
fn reconcile_hooks(
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
// Guardrails reconciliation (Phase 6 / US3)
// =====================================================================

/// Result of the guardrails reconciliation pass. Mirrors the hooks/agents
/// reconciliation shape: a per-harness aggregate action map keyed on
/// `name()`, plus the FIRST failure encountered (forward progress).
struct GuardrailsReconciliation {
    actions: std::collections::HashMap<String, Action>,
    first_error: Option<TomeError>,
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
fn reconcile_guardrails(
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

// =====================================================================
// Native-agent reconciliation (Phase 6 / US1)
// =====================================================================

/// Result of the native-agent reconciliation pass.
struct AgentReconciliation {
    /// Per-harness aggregate action, keyed on `name()`. Used to backfill
    /// each `HarnessDecision.agents_action`.
    actions: std::collections::HashMap<String, Action>,
    /// The FIRST translation/write failure encountered (FR-084 forward
    /// progress): reconciliation attempts the rest of the agents/harnesses,
    /// then surfaces this so the CLI exits with the relevant code.
    first_error: Option<TomeError>,
}

/// A parsed source agent plus its workspace clash flag, prepared once per
/// sync and reused across every native-supporting harness.
struct PreparedAgent {
    canonical: CanonicalAgent,
    clashes: bool,
}

/// Reconcile native agent files for every harness (FR-030–FR-043, FR-084).
///
/// One pass after the rules/MCP loop:
///
/// * A live, native-supporting harness gets each enabled agent translated
///   and written (atomic, mode-preserving, symlink-refusing — reusing the
///   rules-file standalone writer), plus removal of any owned file whose
///   plugin is no longer enabled.
/// * A non-live or non-supporting harness has ALL its Tome-owned
///   `<plugin>__*` files removed (orphan cleanup).
///
/// The clash set + the enabled-agent enumeration + the per-agent
/// `CanonicalAgent` parse are computed ONCE (FR-072) and shared across
/// harnesses. A parse / translate / write failure for one agent is
/// recorded but does not abort the pass (FR-084 forward progress).
fn reconcile_agents(
    project_root: &Path,
    deps: &SyncDeps<'_>,
    effective_names: &HashSet<String>,
    snapshots: &[HarnessSnapshot],
    strip_agent_privileges: bool,
    outcome: &mut SyncOutcome,
) -> Result<AgentReconciliation, TomeError> {
    let mut recon = AgentReconciliation {
        actions: std::collections::HashMap::new(),
        first_error: None,
    };

    // Fast exit: if NO harness supports native agents there is nothing to
    // emit and nothing Tome-owned to clean up.
    if !snapshots.iter().any(|s| s.supports_native_agents) {
        return Ok(recon);
    }

    // Open the central DB read-only to enumerate enabled agents + the clash
    // set. R-1: a GENUINELY ABSENT DB means no enabled agents — emission is
    // empty and cleanup still runs (orphan removal does not need the DB). But
    // an EXISTING-yet-unopenable DB (SchemaTooNew/busy/vec-ext) must
    // PROPAGATE its error here, BEFORE the destructive cleanup pass — never
    // collapse to `None`, which would empty `enabled_plugins` and make the
    // cleanup delete every emitted `<plugin>__*` file for live harnesses.
    let conn = if deps.paths.index_db.exists() {
        Some(crate::index::open_read_only(&deps.paths.index_db)?)
    } else {
        None
    };

    let workspace = deps.workspace_name.as_str();
    // Clash set is computed ONCE per sync (FR-072) and reused for every
    // agent's displayed-name decision across every harness.
    let clash_set = match &conn {
        Some(c) => crate::index::skills::agent_name_clash_set(c, workspace)?,
        None => std::collections::BTreeSet::new(),
    };
    let enabled = match &conn {
        Some(c) => crate::index::skills::enabled_agents_for_workspace(c, workspace)?,
        None => Vec::new(),
    };

    // The set of plugins with at least one enabled agent — drives which
    // owned files survive the per-harness cleanup.
    let enabled_plugins: HashSet<String> = enabled.iter().map(|a| a.plugin.clone()).collect();

    // Parse each enabled agent once into a `CanonicalAgent` + clash flag.
    // A parse failure (malformed frontmatter, missing source) is recorded
    // as the first error but does not stop the rest from preparing
    // (FR-084 forward progress).
    let mut prepared: Vec<PreparedAgent> = Vec::with_capacity(enabled.len());
    if let Some(c) = &conn {
        for row in &enabled {
            match prepare_agent(c, deps.paths, workspace, row) {
                Ok(canonical) => {
                    let clashes = clash_set.contains(&canonical.name);
                    prepared.push(PreparedAgent { canonical, clashes });
                }
                Err(e) => {
                    if recon.first_error.is_none() {
                        recon.first_error = Some(e);
                    }
                }
            }
        }
    }

    // Dispatch translation under the registry guard so `translate_agent`
    // sees the effective module set. The DB work above is already done, so
    // the guard only spans the (fast) translate + (atomic) write.
    with_effective_modules(|mods| {
        for m in mods {
            let name = m.name();
            let is_live = effective_names.contains(name);
            let Some(dir) = m.agent_dir(project_root) else {
                // No native-agent dir → nothing to emit or clean up.
                recon.actions.insert(name.to_string(), Action::LeftAlone);
                continue;
            };
            let action = if m.supports_native_agents() && is_live {
                emit_agents_for_harness(
                    *m,
                    &dir,
                    &prepared,
                    &enabled_plugins,
                    strip_agent_privileges,
                    outcome,
                    &mut recon,
                )
            } else {
                // Non-live or non-supporting: remove all Tome-owned files.
                cleanup_all_owned_agents(name, &dir, outcome, &mut recon)
            };
            recon.actions.insert(name.to_string(), action);
        }
    });

    Ok(recon)
}

/// Parse one enabled agent row into a [`CanonicalAgent`].
///
/// Resolves the catalog-relative source path to an absolute `.md`, reads
/// the body (bounded), and parses the frontmatter. Any failure maps to
/// [`TomeError::AgentTranslationFailed`] (exit 45) so the sync surfaces the
/// offending agent.
///
/// C-4: a parse failure HERE is a post-enable source-corruption edge — the
/// agent enabled cleanly (a malformed agent cannot enable; `lifecycle`
/// rejects it at index time) but its source was corrupted afterwards. The
/// failure is recorded on the forward-progress `first_error` path; the
/// prior-sync file (if any) is left in place — loud-but-isolated. The US5
/// `doctor --fix` removes orphaned `<plugin>__*` files.
fn prepare_agent(
    conn: &rusqlite::Connection,
    paths: &Paths,
    workspace: &str,
    row: &crate::index::skills::EnabledAgent,
) -> Result<CanonicalAgent, TomeError> {
    let agent_label = format!("{}/{}/{}", row.catalog, row.plugin, row.name);
    let abs = crate::index::skills::resolve_entry_body_path(
        conn,
        paths,
        workspace,
        &row.catalog,
        &row.plugin,
        &row.path,
    )
    .map_err(|_| TomeError::AgentTranslationFailed {
        agent: agent_label.clone(),
    })?;
    let contents = crate::util::bounded_read_to_string(&abs, crate::util::HARNESS_RULES_MAX)
        .map_err(|_| TomeError::AgentTranslationFailed {
            agent: agent_label.clone(),
        })?;
    // The filename stem is the source `.md` stem (provenance fallback for
    // the agent name when frontmatter omits `name`).
    let stem = abs
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(&row.name);
    CanonicalAgent::parse(&row.catalog, &row.plugin, stem, &contents)
}

/// Translate + write every prepared agent for one live native-supporting
/// harness `m` writing into `dir`, then remove owned files for plugins no
/// longer enabled.
///
/// Returns the aggregate [`Action`] for the harness: `Created`/`Updated`
/// when any file was written, `Removed` when only removals happened, else
/// `LeftAlone`. A translate or write failure for one agent is recorded on
/// `recon.first_error` and the rest still process (FR-084).
fn emit_agents_for_harness(
    m: &dyn HarnessModule,
    dir: &Path,
    prepared: &[PreparedAgent],
    enabled_plugins: &HashSet<String>,
    strip_agent_privileges: bool,
    outcome: &mut SyncOutcome,
    recon: &mut AgentReconciliation,
) -> Action {
    let mut wrote = false;
    let mut updated = false;
    let mut removed = false;

    // The strip applies to Claude Code emission only (FR-052): it is the sole
    // harness that carries the privileged `hooks` / `mcpServers` /
    // `permissionMode` fields — the others drop them during translation, so the
    // setting is a no-op for them and we skip the per-agent clone there.
    let strip_here = strip_agent_privileges && m.name() == "claude-code";

    for agent in prepared {
        // Strip on a per-emission CLONE so the shared `prepared` canonical (the
        // privilege-audit source the US5 doctor reads) is never mutated. The
        // clear is a no-op for an agent carrying none of the three fields.
        let emit_canonical;
        let canonical = if strip_here {
            let mut c = agent.canonical.clone();
            c.hooks = None;
            c.mcp_servers = None;
            c.permission_mode = None;
            emit_canonical = c;
            &emit_canonical
        } else {
            &agent.canonical
        };
        let translated = match m.translate_agent(canonical, agent.clashes) {
            Ok(t) => t,
            Err(e) => {
                if recon.first_error.is_none() {
                    recon.first_error = Some(e);
                }
                continue;
            }
        };
        let target = dir.join(&translated.filename);
        // S-1 defence-in-depth: the agent `name` is validated as a single
        // safe path segment at index time, but assert here too that the
        // joined target stays directly inside `dir` (no `ParentDir`/separator
        // component snuck through the filename). A failed assert records
        // `AgentTranslationFailed` on the forward-progress path and SKIPS the
        // write — never write outside `dir`.
        if target.parent() != Some(dir) {
            if recon.first_error.is_none() {
                recon.first_error = Some(TomeError::AgentTranslationFailed {
                    agent: format!("{}/{}", agent.canonical.plugin, agent.canonical.name),
                });
            }
            continue;
        }
        match write_agent_file(&target, &translated.rendered) {
            Ok(AgentWrite::Created) => {
                wrote = true;
                record_action(
                    outcome,
                    m.name(),
                    SyncSubsystem::Agents,
                    &target,
                    Action::Created,
                );
            }
            Ok(AgentWrite::Updated) => {
                updated = true;
                record_action(
                    outcome,
                    m.name(),
                    SyncSubsystem::Agents,
                    &target,
                    Action::Updated,
                );
            }
            Ok(AgentWrite::Unchanged) => {
                // Idempotent re-sync: identical bytes already on disk.
                outcome.leave_alones += 1;
            }
            Err(e) => {
                if recon.first_error.is_none() {
                    recon.first_error = Some(e);
                }
            }
        }
    }

    // Remove owned files for plugins no longer enabled. We scan once per
    // plugin known to OWN a file in `dir` but no longer enabled; the owned-
    // file glob already filters by `<plugin>__` prefix. Enumerate the dir's
    // owned files for any plugin not in `enabled_plugins`.
    match removed_disabled_owned(dir, enabled_plugins) {
        Ok(paths) => {
            for path in paths {
                match rules_file::remove_standalone(&path) {
                    Ok(()) => {
                        removed = true;
                        record_action(
                            outcome,
                            m.name(),
                            SyncSubsystem::Agents,
                            &path,
                            Action::Removed,
                        );
                    }
                    Err(e) => {
                        if recon.first_error.is_none() {
                            recon.first_error = Some(e);
                        }
                    }
                }
            }
        }
        Err(e) => {
            if recon.first_error.is_none() {
                recon.first_error = Some(e);
            }
        }
    }

    if wrote {
        Action::Created
    } else if updated {
        Action::Updated
    } else if removed {
        Action::Removed
    } else {
        Action::LeftAlone
    }
}

/// Remove EVERY Tome-owned `<plugin>__*` agent file from `dir` (orphan
/// cleanup for a non-live / non-supporting harness `name`). Since this
/// harness is not emitting, ALL of its Tome-owned files are removed
/// regardless of which plugins are currently enabled.
fn cleanup_all_owned_agents(
    name: &str,
    dir: &Path,
    outcome: &mut SyncOutcome,
    recon: &mut AgentReconciliation,
) -> Action {
    let mut any_removed = false;
    match all_owned_in_dir(dir) {
        Ok(paths) => {
            for path in paths {
                match rules_file::remove_standalone(&path) {
                    Ok(()) => {
                        any_removed = true;
                        record_action(outcome, name, SyncSubsystem::Agents, &path, Action::Removed);
                    }
                    Err(e) => {
                        if recon.first_error.is_none() {
                            recon.first_error = Some(e);
                        }
                    }
                }
            }
        }
        Err(e) => {
            if recon.first_error.is_none() {
                recon.first_error = Some(e);
            }
        }
    }
    if any_removed {
        Action::Removed
    } else {
        Action::LeftAlone
    }
}

/// Collect every Tome-owned agent file in `dir` whose plugin is NOT in
/// `enabled_plugins` (the per-plugin removal contract, FR-043). A missing
/// directory yields an empty Vec.
fn removed_disabled_owned(
    dir: &Path,
    enabled_plugins: &HashSet<String>,
) -> Result<Vec<PathBuf>, TomeError> {
    let read = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(TomeError::Io(e)),
    };
    let mut out = Vec::new();
    for entry in read {
        let entry = entry.map_err(TomeError::Io)?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        // A Tome-owned file is `<plugin>__<name>.<ext>`. Recover `<plugin>`
        // via the agents-module SSOT split and check whether it is still
        // enabled.
        let Some(plugin) = agents::plugin_of_owned_file(file_name) else {
            continue;
        };
        if !enabled_plugins.contains(plugin) {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

/// Collect every Tome-owned `<plugin>__*` agent file in `dir` (orphan
/// cleanup for a non-emitting harness — every owned file goes).
fn all_owned_in_dir(dir: &Path) -> Result<Vec<PathBuf>, TomeError> {
    let read = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(TomeError::Io(e)),
    };
    let mut out = Vec::new();
    for entry in read {
        let entry = entry.map_err(TomeError::Io)?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if agents::plugin_of_owned_file(file_name).is_some() {
            out.push(entry.path());
        }
    }
    out.sort();
    Ok(out)
}

/// Outcome of an atomic agent-file write.
enum AgentWrite {
    Created,
    Updated,
    Unchanged,
}

/// Write one translated agent file atomically, reusing the rules-file
/// writer's discipline (symlink refusal + mode preservation +
/// umask-governed `create_dir_all` of the parent + idempotent no-op when
/// bytes already match). Classifies the result so the per-file
/// `added`/`updated`/`leave_alones` bookkeeping is accurate.
fn write_agent_file(target: &Path, rendered: &str) -> Result<AgentWrite, TomeError> {
    let prior = match crate::util::bounded_read_to_string(target, crate::util::HARNESS_RULES_MAX) {
        Ok(s) => Some(s),
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e),
    };
    let classification = match prior.as_deref() {
        None => AgentWrite::Created,
        Some(existing) if existing == rendered => return Ok(AgentWrite::Unchanged),
        Some(_) => AgentWrite::Updated,
    };
    // `write_standalone` is idempotent + atomic + symlink-refusing + creates
    // the parent dir via umask-governed `create_dir_all` — exactly the
    // agent-file discipline.
    rules_file::write_standalone(target, rendered)?;
    Ok(classification)
}

// =====================================================================
// Bookkeeping
// =====================================================================

fn record_action(
    outcome: &mut SyncOutcome,
    harness: &str,
    subsystem: SyncSubsystem,
    path: &Path,
    action: Action,
) {
    let change = SyncChange {
        harness: harness.to_string(),
        subsystem,
        path: path.to_path_buf(),
    };
    match action {
        Action::Created => outcome.added.push(change),
        Action::Updated => outcome.updated.push(change),
        Action::Removed => outcome.removed.push(change),
        Action::LeftAlone => outcome.leave_alones += 1,
    }
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

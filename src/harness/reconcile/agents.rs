//! Native-agent reconciliation (Phase 6 / US1) — the AGENTS sink.
//!
//! Extracted verbatim from the `sync.rs` orchestrator in Phase 7 (FR-011, the
//! `reconcile/` decomposition). The logic is unchanged: this module owns the
//! one-pass native-agent reconciler plus its private helpers (per-agent parse,
//! emission, owned-file cleanup, the atomic single-file writer) and the shared
//! [`record_action`] bookkeeping the orchestrator and the other sink
//! reconcilers also call.
//!
//! See [`crate::harness::reconcile`] for the fixed sink order and the
//! first-error precedence the orchestrator enforces across the three sinks.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::TomeError;
use crate::harness::agents::{self, CanonicalAgent};
use crate::harness::sync::{
    Action, HarnessSnapshot, SyncChange, SyncDeps, SyncOutcome, SyncSubsystem,
};
use crate::harness::{HarnessModule, rules_file, with_effective_modules};
use crate::paths::Paths;

// =====================================================================
// Native-agent reconciliation (Phase 6 / US1)
// =====================================================================

/// Result of the native-agent reconciliation pass.
pub(crate) struct AgentReconciliation {
    /// Per-harness aggregate action, keyed on `name()`. Used to backfill
    /// each `HarnessDecision.agents_action`.
    pub(crate) actions: std::collections::HashMap<String, Action>,
    /// The FIRST translation/write failure encountered (FR-084 forward
    /// progress): reconciliation attempts the rest of the agents/harnesses,
    /// then surfaces this so the CLI exits with the relevant code.
    pub(crate) first_error: Option<TomeError>,
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
pub(crate) fn reconcile_agents(
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

/// Record one on-disk change against the running [`SyncOutcome`].
///
/// Shared across every sink reconciler (agents/hooks/guardrails) and the
/// orchestrator's rules/MCP loop — `pub(crate)` so the still-in-`sync.rs`
/// callers can reuse the one bookkeeping path.
pub(crate) fn record_action(
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

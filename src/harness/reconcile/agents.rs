//! Native-agent reconciliation (Phase 6 / US1) — the AGENTS sink.
//!
//! Extracted verbatim from the `sync.rs` orchestrator in Phase 7 (FR-011, the
//! `reconcile/` decomposition). The logic is unchanged: this module owns the
//! one-pass native-agent reconciler plus its private helpers (per-agent parse,
//! emission, owned-file cleanup, the atomic single-file writer). It reuses the
//! shared [`record_action`](crate::harness::reconcile::record_action)
//! bookkeeping the orchestrator and the other sink reconcilers also call.
//!
//! See [`crate::harness::reconcile`] for the fixed sink order and the
//! first-error precedence the orchestrator enforces across the three sinks.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::error::TomeError;
use crate::harness::agents::{self, CanonicalAgent};
use crate::harness::reconcile::record_action;
use crate::harness::sync::{Action, HarnessSnapshot, SyncDeps, SyncOutcome, SyncSubsystem};
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

    // Phase 1: load the model registry ONCE per sync (override-if-valid else
    // baked) and thread it into every agent translation.
    let model_registry = crate::model_registry::ModelRegistry::load(deps.paths);

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
    //
    // `snapshots` is already filtered by `SyncDeps.only_harness` upstream
    // (`collect_harness_snapshots`), so this sink honours the same filter by
    // gating its registry walk on the snapshotted name set — exactly like the
    // other sinks, which iterate `snapshots` directly. We still need the full
    // `with_effective_modules` registry view here for `translate_agent`, so we
    // keep the walk but `continue` past any module absent from the snapshots.
    // When `only_harness` is `None` every effective module is snapshotted, so
    // the set contains them all and the `continue` never fires (the full
    // reconcile is unchanged).
    let snap_names: HashSet<&str> = snapshots.iter().map(|s| s.name.as_str()).collect();
    with_effective_modules(|mods| {
        // Co-ownership (Copilot): a non-live harness must NOT mass-clean an
        // agent_dir that a LIVE native-supporting harness also owns — otherwise
        // selecting only one of two co-owners (copilot / copilot-cli, same
        // `.github/agents/`) would delete the live one's freshly-written files.
        // Computed once, up front, so the rule is order-independent.
        let live_owned_dirs: HashSet<PathBuf> = mods
            .iter()
            .filter(|m| {
                snap_names.contains(m.name())
                    && effective_names.contains(m.name())
                    && m.supports_native_agents()
            })
            .filter_map(|m| m.agent_dir(project_root))
            .collect();

        for m in mods {
            let name = m.name();
            if !snap_names.contains(name) {
                // Not in the (possibly `only_harness`-filtered) snapshot set →
                // leave this harness's agent dir completely untouched.
                continue;
            }
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
                    &model_registry,
                    outcome,
                    &mut recon,
                )
            } else if live_owned_dirs.contains(&dir) {
                // A live co-owner maintains this dir; leave it untouched.
                Action::LeftAlone
            } else {
                // Non-live or non-supporting: remove all Tome-owned files/dirs.
                cleanup_all_owned_agents(name, &dir, m.agent_path_strategy(), outcome, &mut recon)
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
///
/// SSOT: shared with the read-only `harness preview` (issue #288) so the preview
/// parses each enabled agent identically to the sync pass — same
/// `resolve_entry_body_path`, same bounded read, same `CanonicalAgent::parse` —
/// and therefore feeds `translate_agent` the same input, matching sync's output.
pub(crate) fn prepare_agent(
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
#[allow(clippy::too_many_arguments)]
fn emit_agents_for_harness(
    m: &dyn HarnessModule,
    dir: &Path,
    prepared: &[PreparedAgent],
    enabled_plugins: &HashSet<String>,
    strip_agent_privileges: bool,
    model_registry: &crate::model_registry::ModelRegistry,
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
        let translated = match m.translate_agent(canonical, agent.clashes, model_registry) {
            Ok(t) => t,
            Err(e) => {
                if recon.first_error.is_none() {
                    recon.first_error = Some(e);
                }
                continue;
            }
        };
        let strategy = m.agent_path_strategy();
        let (target, containment_dir): (PathBuf, PathBuf) = match strategy {
            crate::harness::AgentPathStrategy::FlatFile => {
                (dir.join(&translated.filename), dir.to_path_buf())
            }
            crate::harness::AgentPathStrategy::DirPerAgent { inner_filename } => {
                let sub = dir.join(&translated.filename); // filename == `<plugin>__<name>`
                (sub.join(inner_filename), sub)
            }
        };
        // Containment: the agent's own node (file, or dir-per-agent subdir) must
        // sit directly inside `dir`. For DirPerAgent that is `containment_dir`'s
        // parent; for FlatFile it is the file's parent.
        let parent_ok = match strategy {
            crate::harness::AgentPathStrategy::FlatFile => target.parent() == Some(dir),
            crate::harness::AgentPathStrategy::DirPerAgent { .. } => {
                containment_dir.parent() == Some(dir)
            }
        };
        if !parent_ok {
            if recon.first_error.is_none() {
                recon.first_error = Some(TomeError::AgentTranslationFailed {
                    agent: format!("{}/{}", agent.canonical.plugin, agent.canonical.name),
                });
            }
            continue;
        }
        let agent_label = format!("{}/{}", agent.canonical.plugin, agent.canonical.name);
        match write_agent_file(&target, &translated.rendered, &agent_label) {
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

    // Remove owned files/dirs for plugins no longer enabled. We scan once per
    // plugin known to OWN a file in `dir` but no longer enabled; the owned-
    // file glob already filters by `<plugin>__` prefix. Enumerate the dir's
    // owned files for any plugin not in `enabled_plugins`.
    let removal_strategy = m.agent_path_strategy();
    match removed_disabled_owned(dir, enabled_plugins) {
        Ok(paths) => {
            for path in paths {
                match remove_owned_agent(&path, removal_strategy) {
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

/// Remove EVERY Tome-owned `<plugin>__*` agent file/dir from `dir` (orphan
/// cleanup for a non-live / non-supporting harness `name`). Since this
/// harness is not emitting, ALL of its Tome-owned entries are removed
/// regardless of which plugins are currently enabled.
fn cleanup_all_owned_agents(
    name: &str,
    dir: &Path,
    strategy: crate::harness::AgentPathStrategy,
    outcome: &mut SyncOutcome,
    recon: &mut AgentReconciliation,
) -> Action {
    let mut any_removed = false;
    match all_owned_in_dir(dir) {
        Ok(paths) => {
            for path in paths {
                match remove_owned_agent(&path, strategy) {
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

/// Remove one owned agent node, honouring the harness's path strategy. Refuses
/// a symlinked component first → exit 45 (the agents-sink dedicated code).
fn remove_owned_agent(
    path: &Path,
    strategy: crate::harness::AgentPathStrategy,
) -> Result<(), TomeError> {
    crate::util::refuse_symlinked_component(path).map_err(|_| {
        TomeError::AgentTranslationFailed {
            agent: path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("agent")
                .to_string(),
        }
    })?;
    match strategy {
        crate::harness::AgentPathStrategy::FlatFile => rules_file::remove_standalone(path),
        crate::harness::AgentPathStrategy::DirPerAgent { .. } => {
            std::fs::remove_dir_all(path).map_err(TomeError::Io)
        }
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
#[derive(Debug, PartialEq, Eq)]
enum AgentWrite {
    Created,
    Updated,
    Unchanged,
}

/// Write one translated agent file atomically, reusing the rules-file
/// writer's discipline (mode preservation + umask-governed `create_dir_all`
/// of the parent + idempotent no-op when bytes already match). Classifies the
/// result so the per-file `added`/`updated`/`leave_alones` bookkeeping is
/// accurate.
///
/// Symlink refusal is the agents sink's dedicated concern: a symlinked
/// component on the agent-file write path (intermediate dir OR the final node)
/// surfaces [`TomeError::AgentTranslationFailed`] (exit 45), NOT generic `Io`
/// (7). We therefore run the SSOT guard (`util::symlink_safe`, FR-007
/// intermediate-component hardening) explicitly here and map its refusal onto
/// the agents variant before delegating the bytes to `write_standalone` (which
/// re-checks via the same SSOT guard — idempotent on an already-cleared path).
fn write_agent_file(
    target: &Path,
    rendered: &str,
    agent_label: &str,
) -> Result<AgentWrite, TomeError> {
    // Map the symlink refusal to THIS sink's dedicated exit code (45), never
    // a regression to `Io` (7). Non-symlink IO from the read/write below keeps
    // its own classification.
    crate::util::refuse_symlinked_component(target).map_err(|_| {
        TomeError::AgentTranslationFailed {
            agent: agent_label.to_string(),
        }
    })?;
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

#[cfg(test)]
mod tests {
    // `super::*` already brings `Paths`, `SyncDeps`, `SyncOutcome`,
    // `HashSet`, `HarnessModule`, `reconcile_agents`, etc. into scope; only
    // the test-specific seams are imported here.
    use super::*;
    use crate::harness::{AgentFormat, StubHarness};
    use crate::index::{self, MetaSeed, OpenOptions};
    use crate::workspace::WorkspaceName;
    use tempfile::TempDir;

    fn stub_seed() -> MetaSeed {
        MetaSeed {
            name: "stub".into(),
            version: "0".into(),
        }
    }

    /// Bootstrap an on-disk central DB at `paths.index_db`. `index::open`
    /// registers the vec0 extension + seeds the privileged `global`
    /// workspace, so the agents reconciler's read-only re-open later sees a
    /// genuine DB (not a hand-rolled `meta`-only fixture).
    fn bootstrap_db(paths: &Paths) {
        index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: stub_seed(),
                reranker: stub_seed(),
                summariser: stub_seed(),
                profile: None,
            },
        )
        .expect("bootstrap central index db");
    }

    /// Insert one enabled `agent`-kind row under the `global` workspace so
    /// the enabled set the reconciler WOULD enumerate is non-empty. A
    /// swallowed open error would collapse exactly this set to empty.
    fn insert_enabled_agent(paths: &Paths, catalog: &str, plugin: &str, name: &str) {
        let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw");
        conn.execute(
            "INSERT INTO skills
                (catalog, plugin, name, kind, description, plugin_version,
                 path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
             VALUES (?1, ?2, ?3, 'agent', 'd', '0.0.0', ?4, 'h', 0, 0, NULL, '1970-01-01T00:00:00Z')",
            rusqlite::params![catalog, plugin, name, format!("agents/{name}.md")],
        )
        .expect("insert agent row");
        let skill_id: i64 = conn
            .query_row(
                "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind='agent' AND name=?3",
                rusqlite::params![catalog, plugin, name],
                |r| r.get(0),
            )
            .expect("agent skill id");
        let ws_id: i64 = conn
            .query_row("SELECT id FROM workspaces WHERE name = 'global'", [], |r| {
                r.get(0)
            })
            .expect("global ws id");
        conn.execute(
            "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
            rusqlite::params![ws_id, skill_id],
        )
        .expect("enrol agent in global");
    }

    /// Corrupt the EXISTING DB so `open_read_only` fails deterministically:
    /// store a `schema_version` one above the compiled `SCHEMA_VERSION`,
    /// which the read-only open gate rejects with `SchemaTooNew` (exit 52).
    /// The file still exists, so the reconciler takes the
    /// "existing-but-unopenable" branch — the one the safeguard must
    /// PROPAGATE rather than collapse to an empty enabled set.
    fn poison_schema_version_too_new(paths: &Paths) {
        let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw for poison");
        let too_new = index::SCHEMA_VERSION + 1;
        conn.execute(
            "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
            rusqlite::params![too_new.to_string()],
        )
        .expect("bump schema_version");
    }

    /// Direct unit guard for the AGENTS-sink mass-delete safeguard
    /// (agents.rs ~L86-87): an EXISTING-but-unopenable central DB must make
    /// `reconcile_agents` return `Err` (the propagated open error), NOT `Ok`
    /// with an empty enabled set — because an empty set drives the cleanup
    /// pass to mass-delete every owned `<plugin>__*` file for a live
    /// native-supporting harness.
    ///
    /// This sink is unit-tested DIRECTLY because the orchestrator masks it:
    /// `reconcile_guardrails` opens the same DB unconditionally and earlier
    /// in the fixed sink order, so an integration test through
    /// `sync_project` can never isolate the agents open-propagation — the
    /// guardrails sink aborts the sync first. See the doc-comment on
    /// `tests/harness_sync_mass_delete_safeguard.rs` for the
    /// orchestrator-level invariant this complements.
    #[test]
    fn existing_unopenable_db_propagates_and_preserves_owned_agent_file() {
        let home = TempDir::new().expect("home tempdir");
        let paths = Paths::from_root(home.path().join(".tome"));
        std::fs::create_dir_all(&paths.root).expect("create tome root");

        let project = home.path().join("project");
        std::fs::create_dir_all(&project).expect("create project root");

        // A live native-supporting stub: its `agent_dir` is
        // `<project>/.stub/agents` and it would emit `<plugin>__<name>.md`.
        let stub = StubHarness::default().with_native_agents(AgentFormat::MarkdownYaml);

        // Pre-seed a Tome-owned `<plugin>__*` agent file under the stub's
        // agent_dir — the artefact a swallowed open error would mass-delete.
        let agent_dir = stub
            .agent_dir(&project)
            .expect("native-supporting stub yields an agent_dir");
        std::fs::create_dir_all(&agent_dir).expect("create stub agent_dir");
        let owned = agent_dir.join("plugin-keep__reviewer.md");
        std::fs::write(&owned, "---\nname: reviewer\n---\nbody\n").expect("seed owned agent file");
        assert!(owned.is_file(), "precondition: owned file seeded");

        // Bootstrap a genuine central DB, enable one healthy agent, then
        // poison it so the read-only open fails with SchemaTooNew (exit 52).
        bootstrap_db(&paths);
        insert_enabled_agent(&paths, "cat-keep", "plugin-keep", "reviewer");
        assert!(
            paths.index_db.exists(),
            "precondition: the central DB exists (drives the existing-but-unopenable branch)"
        );
        poison_schema_version_too_new(&paths);

        let workspace = WorkspaceName::global();
        let deps = SyncDeps {
            paths: &paths,
            home_root: home.path(),
            workspace_name: &workspace,
            force: false,
            only_harness: None,
        };

        // Build a faithful snapshot via the same path the orchestrator uses
        // (one native-supporting harness ⇒ the agents reconciler runs past
        // its fast-exit and reaches the DB open). No `HARNESS_MODULES_OVERRIDE`
        // install is needed: the override is only consulted by the
        // `with_effective_modules` dispatch that runs AFTER a successful open,
        // and here the open fails first.
        let snapshots = vec![crate::harness::sync::snapshot_for_test(
            &stub,
            &project,
            home.path(),
        )];
        let effective_names: HashSet<String> = std::iter::once(stub.name().to_string()).collect();
        let mut outcome = SyncOutcome::default();

        let result = reconcile_agents(
            &project,
            &deps,
            &effective_names,
            &snapshots,
            false,
            &mut outcome,
        );

        // (a) The open error PROPAGATES as `Err` with exit 52 — proving it
        //     was not `.ok()`-swallowed into an empty enabled set. Match
        //     manually rather than `expect_err` (the `Ok` payload,
        //     `AgentReconciliation`, is intentionally not `Debug`).
        let err = match result {
            Ok(_) => panic!(
                "an existing-but-unopenable DB must propagate (Err), not return Ok with empties"
            ),
            Err(e) => e,
        };
        assert_eq!(
            err.exit_code(),
            52,
            "propagated SchemaTooNew (exit 52) proves the open error was not swallowed; got {err:?}"
        );

        // (b) The pre-seeded owned file STILL EXISTS — a swallowed open error
        //     would have emptied the enabled set and the cleanup pass would
        //     have mass-deleted it.
        assert!(
            owned.is_file(),
            "the owned agent file must NOT be mass-deleted when the DB open errors"
        );
    }

    // -----------------------------------------------------------------------
    // FR-007: the agents sink refuses a symlinked component on its write path
    // with its DEDICATED exit code (45 / AgentTranslationFailed), never a
    // regression to generic `Io` (7). `write_agent_file` is the private write
    // path that owns the symlink→45 mapping (the public `reconcile_agents`
    // entry needs the full DB/registry plumbing; the dedicated-code mapping is
    // proven here directly). The five sinks with PUBLIC writers (hooks → 44,
    // guardrails → 46, rules/mcp/atomic_dir → 7) are proven in
    // `tests/symlink_intermediate_guard.rs`. macOS exercises the portable
    // `openat`+NOFOLLOW walk; Linux exercises `openat2(RESOLVE_NO_SYMLINKS)` —
    // both via the one SSOT primitive.
    #[cfg(unix)]
    #[test]
    fn agents_write_refuses_symlinked_intermediate_with_exit_45() {
        use std::os::unix::fs::symlink;
        let root = TempDir::new().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        let real_dir = base.join("real_agents");
        std::fs::create_dir(&real_dir).expect("mkdir real_agents");
        // link_agents -> real_agents; writing under link_agents/ traverses a
        // symlinked INTERMEDIATE directory component.
        symlink(&real_dir, base.join("link_agents")).expect("symlink intermediate");

        let target = base.join("link_agents").join("plugin__reviewer.md");
        let err = write_agent_file(
            &target,
            "---\nname: reviewer\n---\nbody\n",
            "plugin/reviewer",
        )
        .expect_err("symlinked intermediate component must be refused");
        assert_eq!(
            err.exit_code(),
            45,
            "agents sink refusal must map to AgentTranslationFailed (45), not Io (7); got {err:?}"
        );
        assert!(
            matches!(err, TomeError::AgentTranslationFailed { .. }),
            "expected AgentTranslationFailed, got {err:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn agents_write_refuses_symlinked_final_node_with_exit_45() {
        use std::os::unix::fs::symlink;
        let root = TempDir::new().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        let dir = base.join("agents");
        std::fs::create_dir(&dir).expect("mkdir agents");
        // The final node itself is a symlink (the prior final-node guarantee,
        // now through the SSOT primitive).
        std::fs::write(base.join("decoy.md"), b"x").expect("write decoy");
        let target = dir.join("plugin__reviewer.md");
        symlink(base.join("decoy.md"), &target).expect("symlink final node");

        let err = write_agent_file(
            &target,
            "---\nname: reviewer\n---\nbody\n",
            "plugin/reviewer",
        )
        .expect_err("symlinked final node must be refused");
        assert_eq!(
            err.exit_code(),
            45,
            "agents final-node refusal must map to 45; got {err:?}"
        );
    }

    /// Sanity: a clean path through a real directory writes successfully and
    /// classifies as Created — proves the guard does NOT reject normal writes.
    #[cfg(unix)]
    #[test]
    fn agents_write_clean_path_succeeds() {
        let root = TempDir::new().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        let dir = base.join("agents");
        std::fs::create_dir(&dir).expect("mkdir agents");
        let target = dir.join("plugin__reviewer.md");
        let out = write_agent_file(
            &target,
            "---\nname: reviewer\n---\nbody\n",
            "plugin/reviewer",
        )
        .expect("clean agent write must succeed");
        assert_eq!(out, AgentWrite::Created);
        assert!(target.is_file());
    }

    // -----------------------------------------------------------------------
    // CON-1 (phase-wide review MAJOR-1): the agents sink's orphan-CLEANUP path
    // (`cleanup_all_owned_agents`, the non-live / non-supporting harness branch)
    // must refuse a symlinked owned file with the SAME dedicated exit code (45 /
    // AgentTranslationFailed) the LIVE-removal path uses — never a regression to
    // generic `Io` (7). Before the fix this path called `remove_standalone`
    // directly, whose `refuse_symlink` mapped the refusal to `Io` (7): same
    // logical operation (refuse to unlink a symlinked Tome-owned agent file),
    // two different exit codes depending on whether the harness was live. The
    // live-removal path (`emit_agents_for_harness`) already pre-checks via
    // `refuse_symlinked_component` → 45; this proves the cleanup path now mirrors
    // it. Reuses the existing fixture style (real tempdir + `std::os::unix` symlink,
    // canonicalised base) and drives the private cleanup fn directly.
    #[cfg(unix)]
    #[test]
    fn agents_cleanup_refuses_symlinked_owned_file_with_exit_45() {
        use std::os::unix::fs::symlink;
        let root = TempDir::new().expect("tempdir");
        let base = root.path().canonicalize().expect("canonicalize");
        let dir = base.join("agents");
        std::fs::create_dir(&dir).expect("mkdir agents");

        // A Tome-owned agent file (`<plugin>__<name>.md`) for a now non-live /
        // non-supporting harness — the orphan the cleanup pass is meant to
        // unlink — is planted as a SYMLINK to a decoy outside the dir. Refusing
        // to unlink through it is the agents sink's concern → exit 45.
        std::fs::write(base.join("decoy.md"), b"x").expect("write decoy");
        let owned = dir.join("plugin-gone__reviewer.md");
        symlink(base.join("decoy.md"), &owned).expect("symlink owned agent file");
        assert!(
            agents::plugin_of_owned_file("plugin-gone__reviewer.md").is_some(),
            "precondition: the planted name is recognised as a Tome-owned agent file"
        );

        let mut outcome = SyncOutcome::default();
        let mut recon = AgentReconciliation {
            actions: std::collections::HashMap::new(),
            first_error: None,
        };
        let action = cleanup_all_owned_agents(
            "stub",
            &dir,
            crate::harness::AgentPathStrategy::FlatFile,
            &mut outcome,
            &mut recon,
        );

        // The refusal is recorded on the forward-progress `first_error` (the
        // shape this fn already uses) and must carry the agents-sink dedicated
        // code 45, NOT generic `Io` (7).
        let err = recon
            .first_error
            .expect("symlinked owned file must be refused and recorded as first_error");
        assert_eq!(
            err.exit_code(),
            45,
            "agents cleanup-removal refusal must map to AgentTranslationFailed (45), not Io (7); got {err:?}"
        );
        assert!(
            matches!(err, TomeError::AgentTranslationFailed { .. }),
            "expected AgentTranslationFailed, got {err:?}"
        );
        // Fail-closed: the symlink is NOT removed (the unlink was refused), and
        // its decoy target is untouched.
        assert!(
            owned.symlink_metadata().is_ok(),
            "the symlinked owned file must NOT be unlinked (fail-closed refusal)"
        );
        assert!(base.join("decoy.md").is_file(), "decoy target untouched");
        // No removal succeeded → the aggregate action is LeftAlone, not Removed.
        assert_eq!(action, Action::LeftAlone);
    }

    /// Co-ownership: a not-live native-supporting harness must NOT delete files
    /// in a dir that a LIVE native-supporting harness co-owns (the Copilot
    /// `.github/agents/` hazard). Two stubs share one agent_dir; one is live.
    #[test]
    fn coowned_dir_is_not_cleaned_by_a_non_live_coowner() {
        // Serialize all lib tests that write HARNESS_MODULES_OVERRIDE so cargo's
        // parallel test runner cannot let two override-installing tests clobber
        // each other (Task 11 will add a second override-installing test here).
        let _override_guard = crate::harness::HARNESS_OVERRIDE_TEST_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let home = TempDir::new().expect("home tempdir");
        let paths = Paths::from_root(home.path().join(".tome"));
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        let project = home.path().join("project");
        std::fs::create_dir_all(&project).expect("create project root");

        // Two native-supporting stubs sharing ONE agent_dir; one live, one not.
        let shared = project.join(".shared/agents");
        let live = StubHarness::default()
            .with_name("co-live")
            .with_native_agents(AgentFormat::MarkdownYaml)
            .with_agent_dir(shared.clone());
        let idle = StubHarness::default()
            .with_name("co-idle")
            .with_native_agents(AgentFormat::MarkdownYaml)
            .with_agent_dir(shared.clone());

        std::fs::create_dir_all(&shared).expect("create shared agent_dir");
        let owned = shared.join("plugin-keep__reviewer.md");
        std::fs::write(&owned, "---\nname: reviewer\n---\nbody\n").expect("seed owned agent file");

        bootstrap_db(&paths);
        insert_enabled_agent(&paths, "cat", "plugin-keep", "reviewer");

        let workspace = WorkspaceName::global();
        let deps = SyncDeps {
            paths: &paths,
            home_root: home.path(),
            workspace_name: &workspace,
            force: false,
            only_harness: None,
        };
        let snapshots = vec![
            crate::harness::sync::snapshot_for_test(&live, &project, home.path()),
            crate::harness::sync::snapshot_for_test(&idle, &project, home.path()),
        ];
        // Only `co-live` is live; `co-idle` is NOT in the effective set.
        let effective_names: HashSet<String> = std::iter::once("co-live".to_string()).collect();
        let mut outcome = SyncOutcome::default();

        // Install both stubs into the process-global registry so
        // `with_effective_modules` sees them (otherwise it walks
        // `SUPPORTED_HARNESSES` whose real harnesses never match "co-live" /
        // "co-idle", making the rule a no-op vacuously — not a real proof).
        *crate::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("override write lock") =
            Some(vec![Box::new(live.clone()), Box::new(idle.clone())]);
        let result = reconcile_agents(
            &project,
            &deps,
            &effective_names,
            &snapshots,
            false,
            &mut outcome,
        );
        // Restore the override immediately — before any assert that could panic.
        *crate::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("override write lock (restore)") = None;

        result.expect("reconcile ok");

        // The owned file written by the live co-owner survives the idle co-owner's pass.
        assert!(
            owned.is_file(),
            "a non-live co-owner must NOT delete a live co-owner's file in a shared agent_dir"
        );
    }

    /// Phase 2 — Copilot co-ownership (Task 10 / Task 9 reconciler rule).
    ///
    /// Selecting only `copilot` (live) while `copilot-cli` is NOT in the
    /// effective set must NOT let `copilot-cli`'s cleanup pass delete
    /// `.github/agents/<plugin>__<name>.agent.md` that `copilot` just wrote.
    ///
    /// Uses the REAL `COPILOT` + `COPILOT_CLI` modules (via
    /// `HARNESS_MODULES_OVERRIDE`) instead of anonymous stubs, so this test
    /// directly proves the production co-ownership is wired correctly.
    #[test]
    fn copilot_coownership_copilot_live_copilot_cli_idle_file_survives() {
        use crate::harness::copilot::COPILOT;
        use crate::harness::copilot_cli::COPILOT_CLI;

        // Serialize all lib tests that write HARNESS_MODULES_OVERRIDE so cargo's
        // parallel test runner cannot let two override-installing tests clobber
        // each other.
        let _override_guard = crate::harness::HARNESS_OVERRIDE_TEST_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let home = TempDir::new().expect("home tempdir");
        let paths = Paths::from_root(home.path().join(".tome"));
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        let project = home.path().join("project");
        std::fs::create_dir_all(&project).expect("create project root");

        // The shared `.github/agents/` directory that both modules point at.
        let shared_dir = project.join(".github/agents");
        std::fs::create_dir_all(&shared_dir).expect("create .github/agents");

        // Pre-seed the owned agent file (as if a prior sync wrote it).
        // `.agent.md` double extension — same as translate_copilot_agent emits.
        let owned = shared_dir.join("myplugin__reviewer.agent.md");
        std::fs::write(
            &owned,
            "---\nname: reviewer\ndescription: Reviews code.\n---\nYou review code.\n",
        )
        .expect("seed owned agent file");

        bootstrap_db(&paths);
        insert_enabled_agent(&paths, "cat", "myplugin", "reviewer");

        let workspace = WorkspaceName::global();
        let deps = SyncDeps {
            paths: &paths,
            home_root: home.path(),
            workspace_name: &workspace,
            force: false,
            only_harness: None,
        };

        // Snapshot both real modules.
        let snapshots = vec![
            crate::harness::sync::snapshot_for_test(&COPILOT, &project, home.path()),
            crate::harness::sync::snapshot_for_test(&COPILOT_CLI, &project, home.path()),
        ];
        // Only `copilot` is live; `copilot-cli` is NOT in the effective set.
        let effective_names: HashSet<String> = std::iter::once("copilot".to_string()).collect();
        let mut outcome = SyncOutcome::default();

        // Install both real modules so `with_effective_modules` resolves them.
        *crate::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("override write lock") = Some(vec![
            Box::new(crate::harness::copilot::Copilot),
            Box::new(crate::harness::copilot_cli::CopilotCli),
        ]);

        let result = reconcile_agents(
            &project,
            &deps,
            &effective_names,
            &snapshots,
            false,
            &mut outcome,
        );

        // Restore the override before any assert that could panic.
        *crate::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("override write lock (restore)") = None;

        result.expect("reconcile ok");

        // The owned `.agent.md` file survives: `copilot-cli` (not live) must
        // NOT clean up the shared `.github/agents/` dir because `copilot`
        // (live) co-owns it.
        assert!(
            owned.is_file(),
            "copilot-cli (idle) must NOT delete files in .github/agents/ while copilot (live) co-owns it"
        );
    }

    /// Task 11: A `DirPerAgent { inner_filename: "AGENT.md" }` stub emits a
    /// subdirectory per agent (containing the inner file) and removal cleans
    /// the entire subdirectory.
    ///
    /// Asserts:
    ///   - After a sync with one enabled agent, `<dir>/<plugin>__<name>/AGENT.md` exists.
    ///   - After disabling the plugin (re-reconcile with the plugin not in
    ///     `enabled_plugins`), the whole `<dir>/<plugin>__<name>/` dir is gone.
    ///
    /// The test seeds the full catalog → plugin → agent source-file chain so
    /// `prepare_agent` succeeds and the translate+emit path runs for real.
    #[test]
    fn dir_per_agent_emits_subdir_and_cleanup_removes_directory() {
        use crate::index::workspace_catalogs;

        // Serialize all lib tests that write HARNESS_MODULES_OVERRIDE so cargo's
        // parallel test runner cannot let two override-installing tests clobber
        // each other.
        let _override_guard = crate::harness::HARNESS_OVERRIDE_TEST_MUTEX
            .lock()
            .unwrap_or_else(|e| e.into_inner());

        let home = TempDir::new().expect("home tempdir");
        let paths = Paths::from_root(home.path().join(".tome"));
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        let project = home.path().join("project");
        std::fs::create_dir_all(&project).expect("create project root");

        let agent_dir = project.join(".stub-devin/agents");
        let stub = StubHarness::default()
            .with_name("stub-devin")
            .with_native_agents(AgentFormat::MarkdownYaml)
            .with_agent_dir(agent_dir.clone())
            .with_agent_path_strategy(crate::harness::AgentPathStrategy::DirPerAgent {
                inner_filename: "AGENT.md",
            });

        // Bootstrap a genuine central DB.
        bootstrap_db(&paths);

        // Enrol a catalog (URL `test://cat`) in the global workspace so
        // `prepare_agent`'s `resolve_entry_body_path` can locate the plugin dir.
        const CATALOG_URL: &str = "test://cat";
        {
            let conn =
                rusqlite::Connection::open(&paths.index_db).expect("open rw for catalog enrol");
            workspace_catalogs::insert(&conn, "global", "cat", CATALOG_URL, "HEAD")
                .expect("enrol test catalog");
        }

        // Create the plugin directory + agent source file on disk at the path
        // `prepare_agent` will derive: `paths.cache_dir_for(URL)/myplugin/agents/myagent.md`.
        // (No catalog manifest → plugin dir is `<catalog_cache>/myplugin`.)
        let catalog_cache = paths.cache_dir_for(CATALOG_URL);
        let plugin_dir = catalog_cache.join("myplugin");
        let agents_source_dir = plugin_dir.join("agents");
        std::fs::create_dir_all(&agents_source_dir).expect("create source agents dir");
        std::fs::write(
            agents_source_dir.join("myagent.md"),
            "---\nname: myagent\ndescription: A test agent.\n---\nYou are a test agent.\n",
        )
        .expect("write source agent file");

        // Insert the skill + workspace_skills rows (reusing the existing helper
        // which references `path = 'agents/myagent.md'` — the same relative path
        // we just created under the plugin dir above).
        insert_enabled_agent(&paths, "cat", "myplugin", "myagent");

        let workspace = WorkspaceName::global();
        let deps = SyncDeps {
            paths: &paths,
            home_root: home.path(),
            workspace_name: &workspace,
            force: false,
            only_harness: None,
        };
        let snapshots = vec![crate::harness::sync::snapshot_for_test(
            &stub,
            &project,
            home.path(),
        )];
        let effective_names: HashSet<String> = std::iter::once("stub-devin".to_string()).collect();
        let mut outcome = SyncOutcome::default();

        // Install stub into the override so `with_effective_modules` resolves it.
        *crate::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("override write lock") = Some(vec![Box::new(stub.clone())]);

        let result = reconcile_agents(
            &project,
            &deps,
            &effective_names,
            &snapshots,
            false,
            &mut outcome,
        );
        // Clear the override before any assert that could panic.
        *crate::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("override write lock (restore)") = None;

        result.expect("first reconcile (emit) must succeed");

        // The inner file inside the per-agent subdirectory must exist.
        let subdir = agent_dir.join("myplugin__myagent");
        let inner_file = subdir.join("AGENT.md");
        assert!(
            inner_file.is_file(),
            "DirPerAgent: inner file <dir>/<plugin>__<name>/AGENT.md must be emitted; got subdir={subdir:?}"
        );
        assert!(
            subdir.is_dir(),
            "DirPerAgent: <plugin>__<name>/ must be a directory"
        );

        // --- Second reconcile: disable the plugin. ---
        // Clear workspace_skills so `enabled_agents_for_workspace` returns empty →
        // `enabled_plugins` is empty → `removed_disabled_owned` enumerates
        // `myplugin__myagent` (owned, plugin not enabled) → removes the subdir.
        {
            let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw for disable");
            conn.execute("DELETE FROM workspace_skills", [])
                .expect("clear workspace_skills");
        }

        let mut outcome2 = SyncOutcome::default();

        *crate::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("override write lock (2nd)") = Some(vec![Box::new(stub.clone())]);

        let result2 = reconcile_agents(
            &project,
            &deps,
            &effective_names,
            &snapshots,
            false,
            &mut outcome2,
        );
        *crate::harness::HARNESS_MODULES_OVERRIDE
            .write()
            .expect("override write lock (restore 2nd)") = None;

        result2.expect("second reconcile (cleanup) must succeed");

        // The whole subdirectory must be gone.
        assert!(
            !subdir.exists(),
            "DirPerAgent removal must delete the entire <plugin>__<name>/ subdirectory"
        );
    }
}

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

    // Remove owned files for plugins no longer enabled. We scan once per
    // plugin known to OWN a file in `dir` but no longer enabled; the owned-
    // file glob already filters by `<plugin>__` prefix. Enumerate the dir's
    // owned files for any plugin not in `enabled_plugins`.
    match removed_disabled_owned(dir, enabled_plugins) {
        Ok(paths) => {
            for path in paths {
                // Symlink refusal on this owned-file removal is the agents
                // sink's concern → exit 45, never `Io` (7). Run the SSOT guard
                // (FR-007) explicitly and map its refusal onto the agents
                // variant; `remove_standalone` then performs the actual unlink
                // (it re-checks via the same guard, idempotent here).
                if crate::util::refuse_symlinked_component(&path).is_err() {
                    if recon.first_error.is_none() {
                        let label = path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("agent")
                            .to_string();
                        recon.first_error =
                            Some(TomeError::AgentTranslationFailed { agent: label });
                    }
                    continue;
                }
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
}

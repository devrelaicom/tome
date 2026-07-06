//! Phase 7 (FR-011, NFR-005) — the MASS-DELETE SAFEGUARD regression.
//!
//! Each per-sink reconciler in `src/harness/reconcile/` opens the central index
//! DB read-only to enumerate the enabled plugins/agents that drive the desired
//! on-disk state. The single biggest behaviour-preservation risk of the Phase 7
//! decomposition is that a reconciler `.ok()`-swallows the open error for an
//! *existing-but-unopenable* DB: the enabled set would collapse to empty and the
//! cleanup pass would mass-delete every Tome-owned file it had previously
//! written (every `<plugin>__*` agent file for a live harness, every owned hook
//! entry, every guardrails region).
//!
//! The contract (carried verbatim into each module) is: a genuinely ABSENT DB
//! means "no enabled entries"; an EXISTING-yet-unopenable DB must PROPAGATE the
//! open error, BEFORE the destructive cleanup runs.
//!
//! ## What this file verifies: the ORCHESTRATOR-LEVEL invariant
//!
//! This test drives `sync_project` end-to-end and asserts the orchestrator-wide
//! guarantee: a poisoned (existing-but-unopenable) DB ABORTS the whole sync at
//! the first sink that reads it, so NO owned file is mass-deleted and the error
//! propagates (exit 52). It does NOT — and structurally CANNOT — isolate the
//! agents sink: the orchestrator runs the sinks in the fixed order hooks →
//! guardrails → agents, and `reconcile_guardrails` opens the same central DB
//! unconditionally (no fast-exit). So whichever of hooks/guardrails reads the
//! poisoned DB first aborts the sync with `?`, masking the agents sink before
//! it ever runs. The exit-52 + file-survival assertions below therefore prove
//! the orchestrator-level invariant, not which specific sink propagated.
//!
//! ## Where the agents sink is guarded directly
//!
//! The per-sink open-propagation — and especially the fail-DANGEROUS AGENTS
//! sink, where a swallowed open error empties the enabled set and the cleanup
//! pass mass-deletes every owned `<plugin>__*` file — is unit-tested DIRECTLY in
//! `src/harness/reconcile/agents.rs` (`tests::existing_unopenable_db_propagates_
//! and_preserves_owned_agent_file`). That in-crate test calls `reconcile_agents`
//! on its own, so it observes the agents open-propagation that this
//! orchestrator-level test cannot reach. (The hooks-presence set, by contrast,
//! is intentionally fail-SAFE — an empty set there does not mass-delete.)
//!
//! Steps below:
//!
//!   1. Seed + enable one healthy native agent, sync once → the owned
//!      `<plugin>__<name>.md` file lands on disk.
//!   2. Corrupt the EXISTING DB so `open_read_only` fails deterministically
//!      (bump `meta.schema_version` above the compiled `SCHEMA_VERSION` →
//!      `SchemaTooNew`, exit 52).
//!   3. Re-sync. The safeguard makes the sync ABORT with the propagated error
//!      rather than returning `Ok` with an empty enabled set — and the
//!      previously-emitted owned file MUST still be on disk (not mass-deleted).

use std::path::PathBuf;

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps};
use tome::harness::{AgentFormat, HooksStrategy, StubHarness};
use tome::workspace::WorkspaceName;

struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    fn build(workspace_name: &str, harnesses: &str) -> Self {
        let env = ToolEnv::new();
        let paths = paths_for(&env);
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        seed_workspace(&paths, workspace_name);
        let workspace = WorkspaceName::parse(workspace_name).expect("parse workspace");

        let project = env.home_path().join("project");
        std::fs::create_dir_all(&project).expect("create project");
        let marker_dir = project.join(".tome");
        std::fs::create_dir_all(&marker_dir).expect("create marker dir");
        std::fs::write(
            marker_dir.join("config.toml"),
            format!("workspace = \"{workspace_name}\"\nharnesses = [{harnesses}]\n"),
        )
        .expect("write marker");
        std::fs::write(marker_dir.join("RULES.md"), "# rules\n").expect("write rules");

        Fixture {
            _home: env.home,
            paths,
            project,
            workspace,
        }
    }

    fn deps(&self) -> SyncDeps<'_> {
        SyncDeps {
            paths: &self.paths,
            home_root: self._home.path(),
            workspace_name: &self.workspace,
            force: false,
            only_harness: None,
            dry_run: false,
        }
    }
}

/// Install the all-sinks stub harness (RealJson hooks + settings + native
/// agents) so the agents reconciler runs and emits a Tome-owned file.
fn install_stub() -> HarnessModulesGuard {
    HarnessModulesGuard::install(vec![Box::new(
        StubHarness::default()
            .with_hooks_strategy(HooksStrategy::RealJson)
            .with_hook_settings()
            .with_native_agents(AgentFormat::MarkdownYaml),
    )])
}

fn plugin_url(plugin: &str) -> String {
    format!("https://example.test/{plugin}.git")
}

fn seed_agent_source(paths: &tome::paths::Paths, plugin: &str, name: &str, body: &str) {
    let dir = paths
        .cache_dir_for(&plugin_url(plugin))
        .join(plugin)
        .join("agents");
    std::fs::create_dir_all(&dir).expect("create agent source dir");
    std::fs::write(dir.join(format!("{name}.md")), body).expect("write source agent");
}

fn enrol_catalog(paths: &tome::paths::Paths, ws: &str, catalog: &str, plugin: &str) {
    let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, ws, catalog, &plugin_url(plugin), "main")
        .expect("enrol catalog");
}

/// Insert + enrol one enabled `agent`-kind row so the plugin participates in the
/// agents sink (and its owned file is emitted).
fn insert_enabled_agent_row(
    paths: &tome::paths::Paths,
    ws: &str,
    catalog: &str,
    plugin: &str,
    name: &str,
) {
    let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw");
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES (?1, ?2, ?3, 'agent', 'desc', '0.0.0', ?4, 'h', 0, 0, NULL, '1970-01-01T00:00:00Z')",
        rusqlite::params![catalog, plugin, name, format!("agents/{name}.md")],
    )
    .expect("insert agent row");
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind='agent'",
            rusqlite::params![catalog, plugin],
            |r| r.get(0),
        )
        .expect("skill id");
    let ws_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![ws],
            |r| r.get(0),
        )
        .expect("ws id");
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
        rusqlite::params![ws_id, skill_id],
    )
    .expect("enrol skill");
}

/// Corrupt the EXISTING DB so `open_read_only` fails deterministically: store a
/// schema version one above the compiled `SCHEMA_VERSION`, which the read-only
/// open gate rejects with `SchemaTooNew` (exit 52). The DB file still exists, so
/// the reconcilers take the "existing-but-unopenable" branch — the one the
/// safeguard must PROPAGATE rather than collapse to an empty enabled set.
fn poison_db_schema_version_too_new(paths: &tome::paths::Paths) {
    let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw for poison");
    let too_new = tome::index::schema::SCHEMA_VERSION + 1;
    conn.execute(
        "UPDATE meta SET value = ?1 WHERE key = 'schema_version'",
        rusqlite::params![too_new.to_string()],
    )
    .expect("bump schema_version");
}

#[test]
fn existing_unopenable_db_aborts_sync_without_mass_deleting_owned_files() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();

    let fx = Fixture::build("test-ws", "\"stub\"");

    // Seed + enable one healthy native agent.
    seed_agent_source(
        &fx.paths,
        "plugin-keep",
        "reviewer",
        "---\nname: reviewer\ndescription: Reviews code.\n---\nYou review code.\n",
    );
    enrol_catalog(&fx.paths, "test-ws", "cat-keep", "plugin-keep");
    insert_enabled_agent_row(&fx.paths, "test-ws", "cat-keep", "plugin-keep", "reviewer");

    // (1) First sync with a HEALTHY DB lands the owned agent file.
    let owned = fx.project.join(".stub/agents/plugin-keep__reviewer.md");
    sync::sync_project(&fx.project, &fx.deps()).expect("first sync succeeds");
    assert!(
        owned.is_file(),
        "the enabled agent's owned file must land on the first (healthy) sync"
    );

    // (2) Corrupt the EXISTING DB so the read-only open fails.
    poison_db_schema_version_too_new(&fx.paths);

    // (3) Re-sync. The mass-delete safeguard makes the WHOLE sync ABORT with
    // the propagated open error rather than returning Ok with an empty enabled
    // set. The abort fires at the first sink that reads the poisoned DB — in the
    // fixed hooks → guardrails → agents order that is guardrails (it opens the
    // DB unconditionally), so the agents sink is masked here; its own
    // open-propagation is guarded directly in src/harness/reconcile/agents.rs.
    let err = sync::sync_project(&fx.project, &fx.deps()).expect_err(
        "an existing-but-unopenable DB must propagate (abort the sync), not return Ok with empties",
    );
    assert_eq!(
        err.exit_code(),
        52,
        "the propagated error is SchemaTooNew (exit 52), proving the open error was not swallowed; got {err:?}"
    );

    // The previously-emitted owned file MUST survive: a swallowed open error
    // would have emptied the enabled set and mass-deleted it during cleanup.
    assert!(
        owned.is_file(),
        "the owned agent file must NOT be mass-deleted when the DB open errors"
    );
}

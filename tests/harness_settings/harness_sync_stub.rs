//! End-to-end tests for `harness::sync::sync_project` against the
//! `StubHarness` fixture (Phase 4 / US1.b-3).
//!
//! The orchestrator is exercised library-API style — no CLI binary, no
//! real harness modules. Each test installs a single-entry override
//! containing [`tome::harness::StubHarness`], builds a project marker
//! by hand, then runs `sync_project` and asserts on the resulting
//! `SyncOutcome` + on-disk state.
//!
//! ## Process-global serialisation
//!
//! `HARNESS_MODULES_OVERRIDE` is a `RwLock<Option<...>>` and cargo runs
//! `#[test]` cases in parallel by default. Tests that install the
//! override must hold a process-local mutex for their entire duration
//! so they don't clobber one another. We follow the
//! `harness_skeleton.rs` convention: a single `crate::common::HARNESS_OVERRIDE_MUTEX` inside
//! this file, locked at test entry. The mutex is `parking_lot`-style
//! safe across panics — `std::sync::Mutex` poisoning is unwrapped to
//! continue scheduling.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::common::{HarnessModulesGuard, HomeGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::StubHarness;
use tome::harness::sync::{self, Action, SyncDeps, SyncSubsystem};
use tome::workspace::WorkspaceName;

/// Snapshot of state shared across tests.
struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    fn build(workspace_name: &str, harnesses_toml: Option<&str>) -> Self {
        let env = ToolEnv::new();
        let paths = paths_for(&env);
        std::fs::create_dir_all(&paths.root).expect("create tome root");
        seed_workspace(&paths, workspace_name);
        let workspace = WorkspaceName::parse(workspace_name).expect("parse workspace");

        let project = env.home_path().join("project");
        std::fs::create_dir_all(&project).expect("create project");

        // Build the project marker config.toml.
        let marker_dir = project.join(".tome");
        std::fs::create_dir_all(&marker_dir).expect("create marker dir");
        let mut body = format!("workspace = \"{workspace_name}\"\n");
        if let Some(harnesses) = harnesses_toml {
            body.push_str(harnesses);
            body.push('\n');
        }
        std::fs::write(marker_dir.join("config.toml"), body).expect("write marker config");

        Fixture {
            _home: env.home,
            paths,
            project,
            workspace,
        }
    }

    fn deps<'a>(&'a self, force: bool) -> SyncDeps<'a> {
        // We don't expose `SyncDeps`'s fields publicly via constructor;
        // build via the public struct literal.
        SyncDeps {
            paths: &self.paths,
            home_root: self._home.path(),
            workspace_name: &self.workspace,
            force,
            only_harness: None,
        }
    }
}

fn install_stub() -> HarnessModulesGuard {
    HarnessModulesGuard::install(vec![Box::new(StubHarness::default())])
}

fn mtime(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .unwrap_or_else(|e| panic!("stat {}: {e}", path.display()))
        .modified()
        .expect("modified time")
}

// ---------------------------------------------------------------------------
// 1. Bind path: sync writes the stub rules block + MCP entry on first run.
// ---------------------------------------------------------------------------

#[test]
fn bind_writes_stub_rules_block_and_mcp_entry() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let fx = Fixture::build("test-workspace", Some("harnesses = [\"stub\"]"));

    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync");

    // Rules-file: STUB_RULES.md exists and contains the Tome block.
    let rules_path = fx.project.join("STUB_RULES.md");
    assert!(rules_path.is_file(), "STUB_RULES.md must exist after sync");
    let rules_body = std::fs::read_to_string(&rules_path).unwrap();
    assert!(
        rules_body.contains("<!-- tome:begin -->") && rules_body.contains("<!-- tome:end -->"),
        "rules file must carry the Tome block; got: {rules_body}"
    );

    // MCP entry: stub.mcp.json carries the canonical Tome entry.
    let mcp_path = fx.project.join("stub.mcp.json");
    assert!(mcp_path.is_file(), "stub.mcp.json must exist after sync");
    let mcp_body = std::fs::read_to_string(&mcp_path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&mcp_body).unwrap();
    let entry = parsed
        .get("mcpServers")
        .and_then(|v| v.get("tome"))
        .expect("tome entry under mcpServers");
    assert_eq!(entry["command"], "tome");
    let args = entry["args"].as_array().unwrap();
    assert_eq!(args[0], "mcp");
    assert_eq!(args[1], "--workspace");
    assert_eq!(args[2], "test-workspace");

    // Outcome bookkeeping.
    assert_eq!(outcome.added.len(), 2, "rules + mcp both added");
    assert!(outcome.updated.is_empty());
    assert!(outcome.removed.is_empty());
    let decision = &outcome.decisions[0];
    assert_eq!(decision.harness, "stub");
    assert!(decision.in_effective_list);
    assert_eq!(decision.rules_action, Action::Created);
    assert_eq!(decision.mcp_action, Action::Created);
}

// ---------------------------------------------------------------------------
// 2. Rebind: changing workspace updates the MCP entry's --workspace arg.
// ---------------------------------------------------------------------------

#[test]
fn rebind_rewrites_mcp_workspace_arg() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let mut fx = Fixture::build("ws-a", Some("harnesses = [\"stub\"]"));

    sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 1");

    // Seed the second workspace + flip the project marker.
    seed_workspace(&fx.paths, "ws-b");
    fx.workspace = WorkspaceName::parse("ws-b").unwrap();
    let marker = fx.project.join(".tome/config.toml");
    std::fs::write(&marker, "workspace = \"ws-b\"\nharnesses = [\"stub\"]\n").unwrap();

    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 2");

    // MCP entry args updated.
    let mcp_path = fx.project.join("stub.mcp.json");
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    let args = parsed["mcpServers"]["tome"]["args"].as_array().unwrap();
    assert_eq!(args[2], "ws-b", "stale workspace arg must be rewritten");

    // Outcome carries an `updated` change.
    assert_eq!(outcome.updated.len(), 1);
    assert_eq!(outcome.updated[0].subsystem, SyncSubsystem::Mcp);
}

// ---------------------------------------------------------------------------
// 3. Harness clash: pre-populated user-owned `tome` entry → exit 19,
//    other writes still happen.
// ---------------------------------------------------------------------------

#[test]
fn harness_clash_returns_exit_19_and_continues_other_writes() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let fx = Fixture::build("test-workspace", Some("harnesses = [\"stub\"]"));

    // Pre-populate stub.mcp.json with a user-owned `tome` entry.
    let mcp_path = fx.project.join("stub.mcp.json");
    let conflict = serde_json::json!({
        "mcpServers": {
            "tome": {
                "command": "evil",
                "args": ["serve"]
            }
        }
    });
    std::fs::write(&mcp_path, serde_json::to_string_pretty(&conflict).unwrap()).unwrap();

    let err = sync::sync_project(&fx.project, &fx.deps(false)).expect_err("must clash");
    assert_eq!(err.exit_code(), 19, "want HarnessClash; got {err:?}");

    match &err {
        tome::error::TomeError::HarnessClash {
            command, first_arg, ..
        } => {
            assert_eq!(command, "evil");
            assert_eq!(first_arg, "serve");
        }
        other => panic!("expected HarnessClash, got {other:?}"),
    }

    // Forward-progress: the rules-file write still happened.
    let rules_path = fx.project.join("STUB_RULES.md");
    assert!(
        rules_path.is_file(),
        "rules file must be written even when MCP clashes (FR-403)",
    );

    // The MCP file is left as the user authored it.
    let mcp_body = std::fs::read_to_string(&mcp_path).unwrap();
    assert!(
        mcp_body.contains("evil"),
        "user-owned MCP entry must survive the clash; got: {mcp_body}",
    );
}

// ---------------------------------------------------------------------------
// 4. --force overrides a clash and preserves env.
// ---------------------------------------------------------------------------

#[test]
fn force_overrides_user_owned_clash_drops_unowned_env() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let fx = Fixture::build("test-workspace", Some("harnesses = [\"stub\"]"));

    let mcp_path = fx.project.join("stub.mcp.json");
    let conflict = serde_json::json!({
        "mcpServers": {
            "tome": {
                "command": "evil",
                "args": ["serve"],
                "env": { "MY_FLAG": "1", "OTHER_FLAG": "2" }
            }
        }
    });
    std::fs::write(&mcp_path, serde_json::to_string_pretty(&conflict).unwrap()).unwrap();

    let _outcome = sync::sync_project(&fx.project, &fx.deps(true)).expect("force must succeed");

    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    let entry = &parsed["mcpServers"]["tome"];

    // The entry was rewritten to Tome-owned shape.
    assert_eq!(entry["command"], "tome");
    let args = entry["args"].as_array().unwrap();
    assert_eq!(args[0], "mcp");

    // env is intentionally NOT preserved when the existing entry was
    // user-owned — `is_tome_owned` returns false for `command="evil"`,
    // so the env-preservation branch in `write_entry_json` doesn't
    // fire. This is by design (FR-503 preserves env only for already-
    // Tome-owned entries); document the observed behaviour explicitly.
    assert!(
        entry.get("env").is_none(),
        "env not preserved when rewriting user-owned"
    );
}

// ---------------------------------------------------------------------------
// 5. Idempotence: re-running sync produces zero disk changes (FR-525).
// ---------------------------------------------------------------------------

#[test]
fn idempotent_resync_no_disk_changes() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let fx = Fixture::build("test-workspace", Some("harnesses = [\"stub\"]"));

    sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 1");
    let rules_path = fx.project.join("STUB_RULES.md");
    let mcp_path = fx.project.join("stub.mcp.json");

    let rules_mtime_1 = mtime(&rules_path);
    let mcp_mtime_1 = mtime(&mcp_path);

    // Wait so mtime granularity can advance if a rewrite happens.
    std::thread::sleep(Duration::from_millis(1500));

    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 2");
    assert!(
        outcome.added.is_empty(),
        "added must be empty on idempotent re-run"
    );
    assert!(
        outcome.updated.is_empty(),
        "updated must be empty on idempotent re-run"
    );
    assert!(
        outcome.removed.is_empty(),
        "removed must be empty on idempotent re-run"
    );
    assert!(outcome.leave_alones >= 2, "rules + mcp both left alone");

    assert_eq!(
        mtime(&rules_path),
        rules_mtime_1,
        "rules mtime must not advance"
    );
    assert_eq!(mtime(&mcp_path), mcp_mtime_1, "mcp mtime must not advance");
}

// ---------------------------------------------------------------------------
// 6. Cleanup: removing the harness from the list removes its entries.
// ---------------------------------------------------------------------------

#[test]
fn cleanup_removes_stub_entries_when_harness_drops_from_list() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let fx = Fixture::build("test-workspace", Some("harnesses = [\"stub\"]"));

    sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 1");
    let rules_path = fx.project.join("STUB_RULES.md");
    let mcp_path = fx.project.join("stub.mcp.json");
    assert!(rules_path.is_file());
    assert!(mcp_path.is_file());

    // Drop the stub from the effective list.
    std::fs::write(
        fx.project.join(".tome/config.toml"),
        "workspace = \"test-workspace\"\nharnesses = []\n",
    )
    .unwrap();

    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 2");

    // Rules-file: the block is gone. The file itself may still exist
    // (it was empty before — `remove_block` collapses to empty content
    // when no surrounding text exists).
    let rules_body = std::fs::read_to_string(&rules_path).unwrap();
    assert!(
        !rules_body.contains("<!-- tome:begin -->"),
        "rules block must be removed; got: {rules_body}",
    );

    // MCP: the entry is gone (and `mcpServers` object survives empty).
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    assert!(
        parsed["mcpServers"].get("tome").is_none(),
        "tome MCP entry must be removed",
    );

    assert_eq!(outcome.removed.len(), 2, "rules + mcp both removed");
}

// ---------------------------------------------------------------------------
// 7. Empty effective list: nothing is written.
// ---------------------------------------------------------------------------

#[test]
fn effective_list_empty_is_noop() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    // No `harnesses` key in the marker → priority walk finds no
    // declarer → empty effective list. Also no workspace/global
    // settings.toml shadow files.
    let fx = Fixture::build("test-workspace", None);

    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync");

    assert!(outcome.added.is_empty());
    assert!(outcome.updated.is_empty());
    assert!(outcome.removed.is_empty());

    // No files created. (`STUB_RULES.md` and `stub.mcp.json` should NOT
    // exist — the cleanup pass leaves absent files absent.)
    assert!(
        !fx.project.join("STUB_RULES.md").exists(),
        "empty effective list must not create rules file",
    );
    assert!(
        !fx.project.join("stub.mcp.json").exists(),
        "empty effective list must not create MCP file",
    );
}

// ---------------------------------------------------------------------------
// 8. Workspace settings override the project marker — when the marker
//    omits `harnesses`, the workspace's `settings.toml` declares it.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// 9. Native agents: a StubHarness with native-agent support emits one file
//    per enabled agent, removes files for plugins no longer enabled, and a
//    re-sync with no change rewrites nothing (Phase 6 / US1, FR-030/043/081).
// ---------------------------------------------------------------------------

/// Seed a manifest-less catalog enrolment plus an on-disk source agent
/// `.md`, returning the catalog name + URL. The agent body lives at
/// `<cache_dir_for(url)>/<plugin>/agents/<name>.md` so
/// `resolve_entry_body_path` (manifest-less fallback) finds it.
fn seed_agent_source(paths: &tome::paths::Paths, plugin: &str, name: &str, body: &str) -> String {
    let url = format!("https://example.test/{plugin}.git");
    let cache = paths.cache_dir_for(&url);
    let agent_dir = cache.join(plugin).join("agents");
    std::fs::create_dir_all(&agent_dir).expect("create agent source dir");
    std::fs::write(agent_dir.join(format!("{name}.md")), body).expect("write source agent");
    url
}

/// Insert an enabled `agent`-kind row for `(catalog, plugin, name)` enrolled
/// in `workspace`, pointing at the catalog-relative `agents/<name>.md` path.
fn insert_enabled_agent_row(
    paths: &tome::paths::Paths,
    workspace: &str,
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
            "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind='agent' AND name=?3",
            rusqlite::params![catalog, plugin, name],
            |r| r.get(0),
        )
        .expect("agent id");
    let ws_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![workspace],
            |r| r.get(0),
        )
        .expect("ws id");
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
        rusqlite::params![ws_id, skill_id],
    )
    .expect("enrol agent");
}

#[test]
fn native_agents_emit_orphan_removal_and_idempotence() {
    use tome::harness::AgentFormat;

    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(
        StubHarness::default().with_native_agents(AgentFormat::MarkdownYaml),
    )]);

    let fx = Fixture::build("test-workspace", Some("harnesses = [\"stub\"]"));

    // Seed catalog enrolment "cat" + two source agents under two plugins.
    let url_a = seed_agent_source(
        &fx.paths,
        "plugin-a",
        "reviewer",
        "---\nname: reviewer\ndescription: Reviews code\n---\nYou review code.\n",
    );
    let url_b = seed_agent_source(
        &fx.paths,
        "plugin-b",
        "builder",
        "---\nname: builder\ndescription: Builds things\n---\nYou build.\n",
    );
    // Enrol both catalogs (one per plugin URL) for the workspace.
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    for (cat, url) in [("cat-a", &url_a), ("cat-b", &url_b)] {
        tome::index::workspace_catalogs::insert(&conn, "test-workspace", cat, url, "main")
            .expect("enrol catalog");
    }
    drop(conn);
    insert_enabled_agent_row(&fx.paths, "test-workspace", "cat-a", "plugin-a", "reviewer");
    insert_enabled_agent_row(&fx.paths, "test-workspace", "cat-b", "plugin-b", "builder");

    // ----- sync 1: both agents emitted -----
    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 1");
    let agent_dir = fx.project.join(".stub/agents");
    let file_a = agent_dir.join("plugin-a__reviewer.md");
    let file_b = agent_dir.join("plugin-b__builder.md");
    assert!(file_a.is_file(), "plugin-a agent emitted");
    assert!(file_b.is_file(), "plugin-b agent emitted");
    let agent_changes = outcome
        .added
        .iter()
        .filter(|c| c.subsystem == SyncSubsystem::Agents)
        .count();
    assert_eq!(agent_changes, 2, "two agent files added on first sync");

    // ----- sync 2: idempotent (no rewrite) -----
    let a_mtime = mtime(&file_a);
    std::thread::sleep(Duration::from_millis(1100));
    let outcome2 = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 2");
    assert!(
        outcome2
            .added
            .iter()
            .chain(&outcome2.updated)
            .all(|c| c.subsystem != SyncSubsystem::Agents),
        "idempotent re-sync must not add/update agent files",
    );
    assert_eq!(mtime(&file_a), a_mtime, "agent file mtime must not advance");

    // ----- sync 3: disable plugin-b's agent → its file is removed -----
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    conn.execute(
        "DELETE FROM workspace_skills WHERE skill_id IN
            (SELECT id FROM skills WHERE plugin = 'plugin-b')",
        [],
    )
    .expect("disable plugin-b agent");
    drop(conn);

    let outcome3 = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 3");
    assert!(file_a.is_file(), "plugin-a agent survives");
    assert!(
        !file_b.exists(),
        "plugin-b agent file removed after disable (FR-043)",
    );
    let removed_agents = outcome3
        .removed
        .iter()
        .filter(|c| c.subsystem == SyncSubsystem::Agents)
        .count();
    assert_eq!(removed_agents, 1, "exactly one agent file removed");
}

// ---------------------------------------------------------------------------
// 9b. T-1: symlink refusal on an agent write → exit 45
//     (TomeError::AgentTranslationFailed), target not overwritten.
// ---------------------------------------------------------------------------

#[test]
#[cfg(unix)]
fn agent_write_through_symlink_is_refused_exit_45() {
    use tome::harness::AgentFormat;

    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(
        StubHarness::default().with_native_agents(AgentFormat::MarkdownYaml),
    )]);

    let fx = Fixture::build("test-workspace", Some("harnesses = [\"stub\"]"));

    let url = seed_agent_source(
        &fx.paths,
        "plugin-a",
        "reviewer",
        "---\nname: reviewer\ndescription: Reviews code\n---\nYou review.\n",
    );
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-workspace", "cat-a", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_agent_row(&fx.paths, "test-workspace", "cat-a", "plugin-a", "reviewer");

    // Pre-plant a symlink at the agent target. The write path must refuse to
    // follow it rather than clobber whatever it points at.
    let agent_dir = fx.project.join(".stub/agents");
    std::fs::create_dir_all(&agent_dir).expect("create agent dir");
    let decoy = fx.project.join("decoy.md");
    std::fs::write(&decoy, "ORIGINAL DECOY CONTENT\n").expect("write decoy");
    let target = agent_dir.join("plugin-a__reviewer.md");
    std::os::unix::fs::symlink(&decoy, &target).expect("plant symlink");

    let err = sync::sync_project(&fx.project, &fx.deps(false)).expect_err("symlink must refuse");
    assert_eq!(
        err.exit_code(),
        45,
        "agents-sink symlink refusal surfaces exit 45 (AgentTranslationFailed); got {err:?}"
    );

    // The decoy the symlink pointed at is untouched.
    let decoy_body = std::fs::read_to_string(&decoy).expect("read decoy");
    assert_eq!(
        decoy_body, "ORIGINAL DECOY CONTENT\n",
        "the symlink target must NOT be overwritten",
    );
    // The planted symlink is still a symlink (not replaced by a regular file).
    let meta = std::fs::symlink_metadata(&target).expect("stat target");
    assert!(
        meta.file_type().is_symlink(),
        "the agent target must remain the planted symlink",
    );
}

// ---------------------------------------------------------------------------
// 9c. T-4: agent forward-progress — one corrupt source + one good agent.
//     The good agent emits AND the sync returns exit 45 (FR-084).
// ---------------------------------------------------------------------------

#[test]
fn agent_forward_progress_one_corrupt_one_good() {
    use tome::harness::AgentFormat;

    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(
        StubHarness::default().with_native_agents(AgentFormat::MarkdownYaml),
    )]);

    let fx = Fixture::build("test-workspace", Some("harnesses = [\"stub\"]"));

    // Good agent.
    let url_a = seed_agent_source(
        &fx.paths,
        "plugin-a",
        "reviewer",
        "---\nname: reviewer\ndescription: Reviews code\n---\nYou review.\n",
    );
    // Corrupt agent: a well-formed source row is enabled, then the on-disk
    // source is overwritten with malformed frontmatter (no closing delimiter)
    // — the post-enable source-corruption edge (C-4). `prepare_agent` fails
    // for this one with exit 45 but the good agent still emits.
    let url_b = seed_agent_source(
        &fx.paths,
        "plugin-b",
        "builder",
        "---\nname: builder\nno closing delimiter here\n",
    );

    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    for (cat, url) in [("cat-a", &url_a), ("cat-b", &url_b)] {
        tome::index::workspace_catalogs::insert(&conn, "test-workspace", cat, url, "main")
            .expect("enrol catalog");
    }
    drop(conn);
    insert_enabled_agent_row(&fx.paths, "test-workspace", "cat-a", "plugin-a", "reviewer");
    insert_enabled_agent_row(&fx.paths, "test-workspace", "cat-b", "plugin-b", "builder");

    let err = sync::sync_project(&fx.project, &fx.deps(false))
        .expect_err("a corrupt agent source must surface an error");
    assert_eq!(
        err.exit_code(),
        45,
        "corrupt agent source → AgentTranslationFailed (exit 45); got {err:?}",
    );

    // Forward progress: the GOOD agent emitted despite the corrupt sibling.
    let good = fx.project.join(".stub/agents/plugin-a__reviewer.md");
    assert!(
        good.is_file(),
        "the well-formed agent must emit despite the corrupt sibling (FR-084)",
    );
}

// ---------------------------------------------------------------------------
// 9d. T-3: multi-harness single-sync fan-out — two native harnesses each get
//     the enabled agent file in their own dir.
// ---------------------------------------------------------------------------

#[test]
fn agent_fans_out_to_multiple_native_harnesses() {
    use tome::harness::AgentFormat;

    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Two distinct native harnesses: the real ClaudeCode + a native stub.
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(tome::harness::claude_code::CLAUDE_CODE),
        Box::new(StubHarness::default().with_native_agents(AgentFormat::MarkdownYaml)),
    ]);

    let fx = Fixture::build(
        "test-workspace",
        Some("harnesses = [\"stub\", \"claude-code\"]"),
    );

    let url = seed_agent_source(
        &fx.paths,
        "plugin-a",
        "reviewer",
        "---\nname: reviewer\ndescription: Reviews code\n---\nYou review code.\n",
    );
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-workspace", "cat-a", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_agent_row(&fx.paths, "test-workspace", "cat-a", "plugin-a", "reviewer");

    sync::sync_project(&fx.project, &fx.deps(false)).expect("sync");

    // Stub dir gets the file (echoes the body via the stub translation).
    let stub_file = fx.project.join(".stub/agents/plugin-a__reviewer.md");
    assert!(stub_file.is_file(), "stub harness got the agent file");

    // Claude Code dir gets the file with real MD+YAML frontmatter.
    let cc_file = fx.project.join(".claude/agents/plugin-a__reviewer.md");
    assert!(cc_file.is_file(), "claude-code harness got the agent file");
    let cc_body = std::fs::read_to_string(&cc_file).expect("read cc agent");
    assert!(
        cc_body.starts_with("---\n") && cc_body.contains("name: reviewer"),
        "claude-code agent carries MD+YAML frontmatter:\n{cc_body}",
    );
}

// ---------------------------------------------------------------------------
// 10. Real hooks (Phase 6 / US2): claude-code merges an enabled plugin's
//     rewritten hooks into `.claude/settings.local.json` (never settings.json),
//     idempotent re-sync rewrites nothing, and dropping claude-code from the
//     effective list removes the owned entry + prunes the empty event.
// ---------------------------------------------------------------------------

/// Seed a manifest-less catalog enrolment plus an on-disk plugin
/// `hooks/hooks.json`, returning the catalog URL. The hooks live at
/// `<cache_dir_for(url)>/<plugin>/hooks/hooks.json` so the hooks pass's
/// `plugin_root_dir` (manifest-less fallback) finds them.
fn seed_hooks_source(paths: &tome::paths::Paths, plugin: &str, body: &str) -> String {
    let url = format!("https://example.test/{plugin}.git");
    let cache = paths.cache_dir_for(&url);
    let hooks_dir = cache.join(plugin).join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks source dir");
    std::fs::write(hooks_dir.join("hooks.json"), body).expect("write source hooks.json");
    url
}

/// Insert an enabled `skill`-kind row for `(catalog, plugin)` so the plugin
/// shows up in the workspace's enabled-plugin enumeration.
fn insert_enabled_skill_row(
    paths: &tome::paths::Paths,
    workspace: &str,
    catalog: &str,
    plugin: &str,
) {
    let conn = rusqlite::Connection::open(&paths.index_db).expect("open rw");
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES (?1, ?2, 'demo', 'skill', 'd', '0.0.0',
                 'skills/demo/SKILL.md', 'h', 1, 0, NULL, '1970-01-01T00:00:00Z')",
        rusqlite::params![catalog, plugin],
    )
    .expect("insert skill row");
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind='skill'",
            rusqlite::params![catalog, plugin],
            |r| r.get(0),
        )
        .expect("skill id");
    let ws_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![workspace],
            |r| r.get(0),
        )
        .expect("ws id");
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
        rusqlite::params![ws_id, skill_id],
    )
    .expect("enrol skill");
}

#[test]
fn real_hooks_merge_idempotence_and_removal_for_claude_code() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard =
        HarnessModulesGuard::install(vec![Box::new(tome::harness::claude_code::CLAUDE_CODE)]);

    let mut fx = Fixture::build("test-workspace", Some("harnesses = [\"claude-code\"]"));

    let url = seed_hooks_source(
        &fx.paths,
        "plugin-a",
        r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/guard.sh --root ${CLAUDE_PROJECT_DIR}" } ] } ] }"#,
    );
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-workspace", "cat-a", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-workspace", "cat-a", "plugin-a");

    // ----- sync 1: hooks merged into settings.local.json -----
    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 1");
    let local = fx.project.join(".claude/settings.local.json");
    assert!(
        local.is_file(),
        "settings.local.json created by the hooks merge"
    );

    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&local).unwrap()).unwrap();
    let cmd = doc["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .expect("command string");
    // PLUGIN_ROOT resolved to an absolute path; PROJECT_DIR left verbatim.
    let plugin_root = fx.paths.cache_dir_for(&url).join("plugin-a");
    assert!(
        cmd.starts_with(&*plugin_root.to_string_lossy()),
        "PLUGIN_ROOT resolved: {cmd}"
    );
    assert!(
        cmd.contains("${CLAUDE_PROJECT_DIR}"),
        "PROJECT_DIR verbatim: {cmd}"
    );

    let hook_changes = outcome
        .added
        .iter()
        .filter(|c| c.subsystem == SyncSubsystem::Hooks)
        .count();
    assert_eq!(hook_changes, 1, "one hooks change recorded on first sync");

    // ----- sync 2: idempotent (no rewrite) -----
    let m1 = mtime(&local);
    std::thread::sleep(Duration::from_millis(1100));
    let outcome2 = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 2");
    assert!(
        outcome2
            .added
            .iter()
            .chain(&outcome2.updated)
            .all(|c| c.subsystem != SyncSubsystem::Hooks),
        "idempotent re-sync must not touch hooks",
    );
    assert_eq!(
        mtime(&local),
        m1,
        "settings.local.json mtime must not advance"
    );

    // ----- sync 3: drop claude-code → owned hook removed, event pruned -----
    fx.workspace = WorkspaceName::parse("test-workspace").unwrap();
    std::fs::write(
        fx.project.join(".tome/config.toml"),
        "workspace = \"test-workspace\"\nharnesses = []\n",
    )
    .unwrap();

    let outcome3 = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync 3");
    let doc3: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&local).unwrap()).unwrap();
    assert!(
        doc3["hooks"]
            .as_object()
            .unwrap()
            .get("PreToolUse")
            .is_none(),
        "the owned hook is removed and its empty event pruned (FR-005/006): {doc3}"
    );
    assert!(
        doc3.as_object().unwrap().contains_key("hooks"),
        "the otherwise-empty hooks object is left in place"
    );
    let removed_hooks = outcome3
        .removed
        .iter()
        .filter(|c| c.subsystem == SyncSubsystem::Hooks)
        .count();
    assert_eq!(removed_hooks, 1, "one hooks change recorded on removal");
}

// ---------------------------------------------------------------------------
// 10b. T2-2: hooks forward-progress — one valid + one malformed plugin.
//      The good plugin's rewritten entry lands in settings.local.json AND the
//      sync returns exit 43 (HookSpecParseError) for the malformed sibling
//      (FR-084). Mirrors `agent_forward_progress_one_corrupt_one_good`.
// ---------------------------------------------------------------------------

#[test]
fn hooks_forward_progress_one_malformed_one_good() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard =
        HarnessModulesGuard::install(vec![Box::new(tome::harness::claude_code::CLAUDE_CODE)]);

    let fx = Fixture::build("test-workspace", Some("harnesses = [\"claude-code\"]"));

    // Good plugin: a well-formed hooks.json.
    let url_a = seed_hooks_source(
        &fx.paths,
        "plugin-a",
        r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/guard.sh" } ] } ] }"#,
    );
    // Malformed plugin: unparsable JSON → HookSpecParseError (exit 43).
    let url_b = seed_hooks_source(&fx.paths, "plugin-b", "{ this is not valid json");

    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    for (cat, url) in [("cat-a", &url_a), ("cat-b", &url_b)] {
        tome::index::workspace_catalogs::insert(&conn, "test-workspace", cat, url, "main")
            .expect("enrol catalog");
    }
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-workspace", "cat-a", "plugin-a");
    insert_enabled_skill_row(&fx.paths, "test-workspace", "cat-b", "plugin-b");

    let err = sync::sync_project(&fx.project, &fx.deps(false))
        .expect_err("a malformed hooks source must surface an error");
    assert_eq!(
        err.exit_code(),
        43,
        "malformed hooks source → HookSpecParseError (exit 43); got {err:?}"
    );

    // Forward progress: the GOOD plugin's entry merged despite the malformed
    // sibling (FR-084).
    let local = fx.project.join(".claude/settings.local.json");
    assert!(
        local.is_file(),
        "settings.local.json created by the good plugin"
    );
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&local).unwrap()).unwrap();
    let cmd = doc["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .expect("good plugin's command string present");
    let plugin_root = fx.paths.cache_dir_for(&url_a).join("plugin-a");
    assert!(
        cmd.starts_with(&*plugin_root.to_string_lossy()),
        "the well-formed plugin's hook merged despite the malformed sibling: {cmd}"
    );
}

// ---------------------------------------------------------------------------
// 11. `--harness <name>` single-harness filter (`SyncDeps.only_harness`):
//     a project whose effective list is `cursor` + `claude-code`, synced with
//     `only_harness = Some("cursor")`, touches ONLY cursor's files and leaves
//     claude-code's `CLAUDE.md` untouched. `None` would reconcile both.
// ---------------------------------------------------------------------------

#[test]
fn sync_with_only_harness_touches_just_that_harness() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Two real harnesses with distinct rules sinks: cursor writes a standalone
    // `.cursor/rules/TOME_SKILLS.md`; claude-code writes a block into
    // `<project>/CLAUDE.md`. Both effective so a full reconcile WOULD write both.
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(tome::harness::cursor::CURSOR),
        Box::new(tome::harness::claude_code::CLAUDE_CODE),
    ]);

    let fx = Fixture::build(
        "test-workspace",
        Some("harnesses = [\"cursor\", \"claude-code\"]"),
    );

    // Plant a Tome-owned agent file in claude-code's native-agent dir BEFORE
    // the sync. Both cursor and claude-code support native agents, so the
    // agents sink runs (cursor is snapshotted ⇒ the fast-exit guard passes).
    // The owned-file naming is `<plugin>__<name>.md`; the agents sink would
    // unlink it as an orphan (its plugin is not in the empty enabled set) IF
    // it touched claude-code's dir. Under `only_harness = Some("cursor")` that
    // dir must be left completely untouched, so this file must survive — this
    // is the regression the agents sink ignored before the filter was honoured.
    let cc_agents_dir = fx.project.join(".claude/agents");
    std::fs::create_dir_all(&cc_agents_dir).expect("create claude-code agent dir");
    let planted_cc_agent = cc_agents_dir.join("plugin-keep__reviewer.md");
    std::fs::write(&planted_cc_agent, "---\nname: reviewer\n---\nbody\n")
        .expect("plant owned claude-code agent file");
    assert!(
        planted_cc_agent.is_file(),
        "precondition: the claude-code owned agent file was planted",
    );

    // Restrict the reconcile to cursor only.
    let mut deps = fx.deps(false);
    deps.only_harness = Some(["cursor".to_string()].into_iter().collect());

    let outcome = sync::sync_project(&fx.project, &deps).expect("sync cursor only");

    // Cursor's standalone rules file was created.
    let cursor_rules = fx.project.join(".cursor/rules/TOME_SKILLS.md");
    assert!(
        cursor_rules.is_file(),
        "cursor's rules file must be written under --harness cursor",
    );
    // Claude-code was left completely untouched: its CLAUDE.md was never created.
    assert!(
        !fx.project.join("CLAUDE.md").exists(),
        "claude-code's CLAUDE.md must NOT be created under --harness cursor",
    );
    // The agents sink honours `only_harness` too: claude-code's planted owned
    // agent file must STILL EXIST — it was never an emit/cleanup target because
    // claude-code was not in the (cursor-only) snapshot set.
    assert!(
        planted_cc_agent.is_file(),
        "claude-code's owned agent file must NOT be removed under --harness cursor \
         (the agents sink must honour only_harness)",
    );
    // No recorded change targets claude-code in the agents subsystem (defence in
    // depth alongside the on-disk survival assertion).
    assert!(
        !outcome
            .removed
            .iter()
            .any(|c| c.subsystem == SyncSubsystem::Agents && c.harness == "claude-code"),
        "no agents-subsystem removal may target claude-code under --harness cursor; got {:?}",
        outcome.removed,
    );

    // Every recorded decision is for cursor only — claude-code never entered
    // the snapshot set, so it produced no decision at all.
    assert!(
        outcome.decisions.iter().all(|d| d.harness == "cursor"),
        "only cursor decisions expected; got {:?}",
        outcome
            .decisions
            .iter()
            .map(|d| d.harness.as_str())
            .collect::<Vec<_>>(),
    );
}

// ---------------------------------------------------------------------------
// 11a. Phase 11 / US6 (T081): `--harness` is a SET. A three-harness effective
//     list synced with `only_harness = Some({cursor, claude-code})` touches
//     BOTH cursor and claude-code and leaves the third (codex) untouched.
// ---------------------------------------------------------------------------

#[test]
fn sync_with_only_harness_set_touches_each_named_harness() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // Three real harnesses with distinct rules sinks: cursor → standalone
    // `.cursor/rules/TOME_SKILLS.md`; claude-code → block in `<project>/CLAUDE.md`;
    // codex → block in `<project>/AGENTS.md`.
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(tome::harness::cursor::CURSOR),
        Box::new(tome::harness::claude_code::CLAUDE_CODE),
        Box::new(tome::harness::codex::CODEX),
    ]);

    let fx = Fixture::build(
        "test-workspace",
        Some("harnesses = [\"cursor\", \"claude-code\", \"codex\"]"),
    );

    // Restrict the reconcile to the {cursor, claude-code} SET.
    let mut deps = fx.deps(false);
    deps.only_harness = Some(
        ["cursor".to_string(), "claude-code".to_string()]
            .into_iter()
            .collect(),
    );

    let outcome = sync::sync_project(&fx.project, &deps).expect("sync the named set");

    // Both named harnesses wrote their rules files.
    assert!(
        fx.project.join(".cursor/rules/TOME_SKILLS.md").is_file(),
        "cursor's rules file must be written",
    );
    assert!(
        fx.project.join("CLAUDE.md").is_file(),
        "claude-code's CLAUDE.md must be written",
    );
    // The UNNAMED third harness (codex) was left untouched: its AGENTS.md was
    // never created, and it produced no decision.
    assert!(
        !fx.project.join("AGENTS.md").exists(),
        "codex's AGENTS.md must NOT be created (not in the --harness set)",
    );
    assert!(
        outcome.decisions.iter().all(|d| d.harness != "codex"),
        "no codex decision expected; got {:?}",
        outcome
            .decisions
            .iter()
            .map(|d| d.harness.as_str())
            .collect::<Vec<_>>(),
    );
    // Both named harnesses DID produce decisions.
    assert!(
        outcome.decisions.iter().any(|d| d.harness == "cursor"),
        "cursor must have a decision",
    );
    assert!(
        outcome.decisions.iter().any(|d| d.harness == "claude-code"),
        "claude-code must have a decision",
    );
}

// ---------------------------------------------------------------------------
// 11a-cmd. Phase 11 / US6 (F1 closeout): the SAME multi-harness filter, but
//     driven THROUGH the real command entry `commands::sync::sync_one_project`,
//     which builds `deps.only_harness` from `args.harness` via
//     `harness_filter_set`. This proves the `args.harness → harness_filter_set →
//     deps.only_harness` wiring end-to-end (the direct `sync_project` test above
//     sets `only_harness` by hand and so never exercises that join).
//
//     `sync_one_project` calls `commands::harness::home_root()`, which reads the
//     process `$HOME`; a `HomeGuard` pins it to this fixture's temp home so
//     harness detection is deterministic AND matches `deps.home_root`.
// ---------------------------------------------------------------------------

#[test]
fn cmd_sync_harness_set_filters_to_named_two_through_command() {
    // HARNESS_OVERRIDE_MUTEX before HOME_MUTEX (via HomeGuard) — the documented
    // lock order when a test needs both.
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(tome::harness::cursor::CURSOR),
        Box::new(tome::harness::claude_code::CLAUDE_CODE),
        Box::new(tome::harness::codex::CODEX),
    ]);

    let fx = Fixture::build(
        "test-workspace",
        Some("harnesses = [\"cursor\", \"claude-code\", \"codex\"]"),
    );
    // Pin $HOME so `home_root()` inside the command resolves to this fixture.
    let _home = HomeGuard::install(fx._home.path());

    // The REAL command entry builds `only_harness` from `args.harness`.
    let args = tome::cli::SyncArgs {
        all: false,
        rules_only: false,
        harness_only: true,
        harness: vec!["cursor".to_string(), "claude-code".to_string()],
    };
    let outcome =
        tome::commands::sync::sync_one_project(&fx.workspace, &fx.project, &args, &fx.paths)
            .expect("sync_one_project through the command");

    // Both named harnesses wrote their rules sinks.
    assert!(
        fx.project.join(".cursor/rules/TOME_SKILLS.md").is_file(),
        "cursor's rules file must be written through the command filter",
    );
    assert!(
        fx.project.join("CLAUDE.md").is_file(),
        "claude-code's CLAUDE.md must be written through the command filter",
    );
    // The UNNAMED third harness (codex) is left untouched: its AGENTS.md was
    // never created — proving `args.harness` reached `deps.only_harness`.
    assert!(
        !fx.project.join("AGENTS.md").exists(),
        "codex's AGENTS.md must NOT be created (not in --harness; through-command filter)",
    );
    // The reported harness_changes account only for the two named harnesses.
    assert!(
        outcome.harness_changes >= 2,
        "expected at least the two named harnesses' changes; got {}",
        outcome.harness_changes,
    );
}

// ---------------------------------------------------------------------------
// 11a-cmd-single. Phase 11 / US6 (F1 closeout): the single-`--harness` variant
//     still works THROUGH the command — `harness: vec!["cursor"]` filters to a
//     one-element set, so only cursor writes and the other two are untouched.
// ---------------------------------------------------------------------------

#[test]
fn cmd_sync_single_harness_filters_through_command() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(tome::harness::cursor::CURSOR),
        Box::new(tome::harness::claude_code::CLAUDE_CODE),
        Box::new(tome::harness::codex::CODEX),
    ]);

    let fx = Fixture::build(
        "test-workspace",
        Some("harnesses = [\"cursor\", \"claude-code\", \"codex\"]"),
    );
    let _home = HomeGuard::install(fx._home.path());

    let args = tome::cli::SyncArgs {
        all: false,
        rules_only: false,
        harness_only: true,
        harness: vec!["cursor".to_string()],
    };
    tome::commands::sync::sync_one_project(&fx.workspace, &fx.project, &args, &fx.paths)
        .expect("single --harness through the command");

    assert!(
        fx.project.join(".cursor/rules/TOME_SKILLS.md").is_file(),
        "cursor's rules file must be written under single --harness cursor",
    );
    assert!(
        !fx.project.join("CLAUDE.md").exists(),
        "claude-code's CLAUDE.md must NOT be created under single --harness cursor",
    );
    assert!(
        !fx.project.join("AGENTS.md").exists(),
        "codex's AGENTS.md must NOT be created under single --harness cursor",
    );
}

// ---------------------------------------------------------------------------
// 11b. I-1 regression: `--harness <X>` on a project where X co-owns a SHARED
//     rules file with another LIVE harness whose body style is `Inline` must
//     preserve the inline LCD body — it must NOT rewrite the shared file to the
//     bare `@`-include form just because the filtered snapshot set sees only X.
//
//     codex (AtInclude) + opencode (Inline) both write `<project>/AGENTS.md`.
//     `tome sync --harness codex` must still write the INLINE body (opencode is
//     a live co-owner), keeping opencode's view intact. A full sync already does
//     this; the bug was that the filtered `--harness` path computed the
//     body-style LCD over the one-element (codex-only) snapshot set.
// ---------------------------------------------------------------------------

#[test]
fn only_harness_preserves_inline_lcd_for_shared_rules_with_live_coowner() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // codex + opencode both target `<project>/AGENTS.md`; codex is `AtInclude`,
    // opencode is `Inline`. Both effective.
    let _guard = HarnessModulesGuard::install(vec![
        Box::new(tome::harness::codex::CODEX),
        Box::new(tome::harness::opencode::OPENCODE),
    ]);

    let fx = Fixture::build(
        "test-workspace",
        Some("harnesses = [\"codex\", \"opencode\"]"),
    );

    // Author a non-empty project RULES.md so the inline body is distinguishable
    // from the bare `@.tome/RULES.md` include directive.
    let rules_marker = fx.project.join(".tome/RULES.md");
    let rules_text = "INLINE RULES BODY MARKER\n";
    std::fs::write(&rules_marker, rules_text).expect("write project RULES.md");

    let agents_md = fx.project.join("AGENTS.md");

    // ----- Sanity: a FULL sync writes the inline body (opencode forces it). -----
    sync::sync_project(&fx.project, &fx.deps(false)).expect("full sync");
    let full_body = std::fs::read_to_string(&agents_md).expect("read AGENTS.md after full sync");
    assert!(
        full_body.contains("INLINE RULES BODY MARKER"),
        "full sync must write the inline LCD body for codex+opencode; got: {full_body}",
    );
    assert!(
        !full_body.contains("@.tome/RULES.md"),
        "full sync must NOT use the bare @-include form; got: {full_body}",
    );

    // ----- The bug: `--harness codex` must NOT downgrade to the @-include. -----
    let mut deps = fx.deps(false);
    deps.only_harness = Some(["codex".to_string()].into_iter().collect());
    sync::sync_project(&fx.project, &deps).expect("sync codex only");

    let after_body = std::fs::read_to_string(&agents_md).expect("read AGENTS.md after --harness");
    assert!(
        after_body.contains("INLINE RULES BODY MARKER"),
        "`--harness codex` must preserve opencode's inline view (the LCD across \
         ALL live co-owners), NOT rewrite the shared AGENTS.md to @-include; got: {after_body}",
    );
    assert!(
        !after_body.contains("@.tome/RULES.md"),
        "`--harness codex` must NOT write the bare @-include directive into the \
         shared file while opencode is a live co-owner; got: {after_body}",
    );
}

#[test]
fn workspace_settings_supply_harness_list_when_marker_omits_key() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    // Project marker omits `harnesses` → resolver falls through.
    let fx = Fixture::build("test-workspace", None);

    // Workspace settings declare the list.
    let ws_dir = fx.paths.workspace_dir(&fx.workspace);
    std::fs::create_dir_all(&ws_dir).unwrap();
    std::fs::write(
        ws_dir.join("settings.toml"),
        "name = \"test-workspace\"\nharnesses = [\"stub\"]\n",
    )
    .unwrap();

    let outcome = sync::sync_project(&fx.project, &fx.deps(false)).expect("sync");

    assert_eq!(
        outcome.added.len(),
        2,
        "rules + mcp added from workspace-supplied list"
    );
    assert!(fx.project.join("STUB_RULES.md").is_file());
    assert!(fx.project.join("stub.mcp.json").is_file());
}

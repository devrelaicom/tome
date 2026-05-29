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
//! `harness_skeleton.rs` convention: a single `OVERRIDE_MUTEX` inside
//! this file, locked at test entry. The mutex is `parking_lot`-style
//! safe across panics — `std::sync::Mutex` poisoning is unwrapped to
//! continue scheduling.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::StubHarness;
use tome::harness::sync::{self, Action, SyncDeps, SyncSubsystem};
use tome::workspace::WorkspaceName;

/// Process-global mutex serialising every test in this file. Held for
/// the entire test body — `HARNESS_MODULES_OVERRIDE` is a single slot
/// and cargo runs `#[test]` cases on multiple threads.
static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

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
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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

    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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
// 9b. T-1: symlink refusal on an agent write → exit 7, target not overwritten.
// ---------------------------------------------------------------------------

#[test]
#[cfg(unix)]
fn agent_write_through_symlink_is_refused_exit_7() {
    use tome::harness::AgentFormat;

    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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
        7,
        "symlink refusal surfaces exit 7; got {err:?}"
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

    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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

    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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

#[test]
fn workspace_settings_supply_harness_list_when_marker_omits_key() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
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

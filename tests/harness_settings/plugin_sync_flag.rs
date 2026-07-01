//! #280 — `tome plugin enable/disable --sync` propagation helper.
//!
//! `--sync` on enable/disable routes through the SAME shared path
//! (`tome::commands::sync::sync_bound_projects` → `sync_all` → `sync_project`)
//! that `tome sync --all` uses, over every project bound to the resolved
//! workspace. These tests exercise that helper directly (the enable/disable
//! `run` fns load the real embedder/index, so the propagation half is tested at
//! the library seam — the same style as `tests/sync_cmd.rs`).
//!
//! The invariant the issue is about: after `--sync`, the harness rules directive
//! and MCP config for a BOUND project ARE written (they were NOT before #280,
//! when enable/disable only propagated RULES.md and never reconciled harness
//! files).

use std::path::{Path, PathBuf};

use crate::common::{
    HarnessModulesGuard, HomeGuard, ToolEnv, paths_for, seed_workspace, stub_embedder_seed,
    stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use time::OffsetDateTime;
use tome::harness::StubHarness;
use tome::index::{self, OpenOptions};
use tome::paths::Paths;
use tome::workspace::WorkspaceName;

fn install_stub() -> HarnessModulesGuard {
    HarnessModulesGuard::install(vec![Box::new(StubHarness::default())])
}

fn open_central(paths: &Paths) -> rusqlite::Connection {
    index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open central DB")
}

/// Create `<root>/.tome/config.toml` (declaring the stub harness) and insert the
/// `workspace_projects` binding row so `sync_all` walks this project.
fn seed_bound_project(paths: &Paths, workspace_name: &str, project_root: &Path) {
    std::fs::create_dir_all(project_root.join(".tome")).expect("create .tome");
    std::fs::write(
        project_root.join(".tome").join("config.toml"),
        format!("workspace = \"{workspace_name}\"\nharnesses = [\"stub\"]\n"),
    )
    .expect("write project config.toml");
    let conn = open_central(paths);
    let workspace_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = ?1",
            rusqlite::params![workspace_name],
            |row| row.get(0),
        )
        .expect("lookup workspace_id");
    let now = OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO workspace_projects (project_path, workspace_id, bound_at)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![
            project_root.to_string_lossy().to_string(),
            workspace_id,
            now
        ],
    )
    .expect("seed workspace_projects");
}

/// Full setup: a temp home, a seeded workspace, and a bound project directory.
struct Fixture {
    home: TempDir,
    paths: Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

fn build(workspace_name: &str) -> Fixture {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).expect("create tome root");
    seed_workspace(&paths, workspace_name);
    let project = env.home_path().join("project");
    std::fs::create_dir_all(&project).expect("create project");
    seed_bound_project(&paths, workspace_name, &project);
    Fixture {
        home: env.home,
        paths,
        project,
        workspace: WorkspaceName::parse(workspace_name).expect("parse workspace"),
    }
}

// ---------------------------------------------------------------------------
// 1. `--sync` reconciles harness files for a bound project.
//    This is the exact gap the issue describes: without it, enable/disable
//    never touch the harness rules directive or MCP config.
// ---------------------------------------------------------------------------

#[test]
fn sync_bound_projects_writes_harness_files_for_bound_project() {
    // HARNESS_OVERRIDE_MUTEX before HOME_MUTEX (via HomeGuard) — documented lock
    // order when a test needs both. `sync_all` calls `home_root()` (reads $HOME).
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let fx = build("ws-sync");
    let _home = HomeGuard::install(fx.home.path());

    // Precondition: no harness files exist before the propagation runs.
    let rules_path = fx.project.join("STUB_RULES.md");
    let mcp_path = fx.project.join("stub.mcp.json");
    assert!(!rules_path.exists(), "precondition: no rules file yet");
    assert!(!mcp_path.exists(), "precondition: no MCP file yet");

    let report = tome::commands::sync::sync_bound_projects(&fx.workspace, &fx.paths).expect("sync");

    // The bound project appears in the report.
    assert_eq!(
        report.projects.len(),
        1,
        "one bound project synced: {report:?}"
    );
    assert_eq!(report.projects[0].project, fx.project);
    // The harness reconcile ran (rules + MCP both created → 2 changes).
    assert!(
        report.projects[0].harness_changes >= 2,
        "harness reconcile must run: {report:?}",
    );

    // The harness rules directive + MCP config were written for the bound
    // project — the behaviour that was missing before #280.
    assert!(
        rules_path.is_file(),
        "STUB_RULES.md must exist after --sync"
    );
    let rules_body = std::fs::read_to_string(&rules_path).unwrap();
    assert!(
        rules_body.contains("<!-- tome:begin -->"),
        "rules file must carry the Tome block: {rules_body}",
    );
    assert!(mcp_path.is_file(), "stub.mcp.json must exist after --sync");
    let mcp: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&mcp_path).unwrap()).unwrap();
    assert!(
        mcp["mcpServers"].get("tome").is_some(),
        "MCP config must carry the tome entry: {mcp}",
    );
}

// ---------------------------------------------------------------------------
// 2. `--sync` fans out to EVERY bound project of the workspace (matching the
//    scope RULES.md propagation already reaches), not just one.
// ---------------------------------------------------------------------------

#[test]
fn sync_bound_projects_fans_out_to_all_bound_projects() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();
    let fx = build("ws-fan");
    let _home = HomeGuard::install(fx.home.path());

    // Bind a SECOND project to the same workspace.
    let project_b = fx.home.path().join("project-b");
    std::fs::create_dir_all(&project_b).expect("create project-b");
    seed_bound_project(&fx.paths, "ws-fan", &project_b);

    let report = tome::commands::sync::sync_bound_projects(&fx.workspace, &fx.paths).expect("sync");

    assert_eq!(
        report.projects.len(),
        2,
        "both bound projects synced: {report:?}"
    );
    // Both projects got their harness files.
    for p in [&fx.project, &project_b] {
        assert!(
            p.join("STUB_RULES.md").is_file(),
            "rules file missing for {}",
            p.display(),
        );
        assert!(
            p.join("stub.mcp.json").is_file(),
            "MCP file missing for {}",
            p.display(),
        );
    }
}

// ---------------------------------------------------------------------------
// 3. No bound projects → an empty report (no error). The absent-`--sync` path
//    is the pre-#280 read-only behaviour and is not exercised here; this
//    confirms the helper is safe when the workspace has no bindings.
// ---------------------------------------------------------------------------

#[test]
fn sync_bound_projects_empty_when_no_bindings() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = install_stub();

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).expect("create tome root");
    seed_workspace(&paths, "ws-empty");
    let _home = HomeGuard::install(env.home_path());

    let ws = WorkspaceName::parse("ws-empty").expect("parse");
    let report = tome::commands::sync::sync_bound_projects(&ws, &paths).expect("sync");
    assert!(
        report.projects.is_empty(),
        "no bindings → empty report: {report:?}"
    );
}

// ---------------------------------------------------------------------------
// 4. Failure ordering (R3): when the follow-up sync FAILS, the returned error
//    carries the underlying `sync_project` exit code. Combined with the code
//    ordering in `enable`/`disable::run` (the state change + success line run
//    BEFORE the `?` on `sync_bound_projects`), this proves the enable/disable
//    is already committed and the sync error surfaces with the right code.
//
//    A symlinked native-agent sink is refused by `sync_project` (exit 45,
//    AgentTranslationFailed) — the same seam `harness_sync_stub` uses. This is
//    the exact propagation path `--sync` takes.
// ---------------------------------------------------------------------------

/// Seed a manifest-less catalog enrolment plus an on-disk source agent so the
/// native-agent sink has something to emit. Returns the catalog URL.
fn seed_agent_source(paths: &Paths, plugin: &str, name: &str, body: &str) -> String {
    let url = format!("https://example.test/{plugin}.git");
    let cache = paths.cache_dir_for(&url);
    let agent_dir = cache.join(plugin).join("agents");
    std::fs::create_dir_all(&agent_dir).expect("create agent source dir");
    std::fs::write(agent_dir.join(format!("{name}.md")), body).expect("write source agent");
    url
}

/// Insert an enabled `agent`-kind row for `(catalog, plugin, name)` in `workspace`.
fn insert_enabled_agent_row(
    paths: &Paths,
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
#[cfg(unix)]
fn sync_bound_projects_failure_surfaces_sync_exit_code() {
    use tome::harness::AgentFormat;

    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    // A native-agent-capable stub so the agents sink runs (and can be made to
    // refuse a symlinked target).
    let _guard = HarnessModulesGuard::install(vec![Box::new(
        StubHarness::default().with_native_agents(AgentFormat::MarkdownYaml),
    )]);
    let fx = build("ws-fail");
    let _home = HomeGuard::install(fx.home.path());

    // Seed an enabled agent so the sink has a file to write.
    let url = seed_agent_source(
        &fx.paths,
        "plugin-a",
        "reviewer",
        "---\nname: reviewer\ndescription: Reviews code\n---\nYou review.\n",
    );
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "ws-fail", "cat-a", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_agent_row(&fx.paths, "ws-fail", "cat-a", "plugin-a", "reviewer");

    // Plant a symlink at the agent target — `sync_project` refuses to follow it.
    let agent_dir = fx.project.join(".stub/agents");
    std::fs::create_dir_all(&agent_dir).expect("create agent dir");
    let decoy = fx.project.join("decoy.md");
    std::fs::write(&decoy, "ORIGINAL DECOY\n").expect("write decoy");
    let target = agent_dir.join("plugin-a__reviewer.md");
    std::os::unix::fs::symlink(&decoy, &target).expect("plant symlink");

    // The `--sync` propagation path surfaces the underlying `sync_project` exit
    // code (45, AgentTranslationFailed) — the caller then reports enable/disable
    // done + sync failed with this code.
    let err = tome::commands::sync::sync_bound_projects(&fx.workspace, &fx.paths)
        .expect_err("symlinked agent sink must make the sync fail");
    assert_eq!(
        err.exit_code(),
        45,
        "the sync failure must carry the sync_project exit code (45); got {err:?}",
    );

    // The symlink's target is untouched (writer safety inherited from sync_project).
    assert_eq!(
        std::fs::read_to_string(&decoy).expect("read decoy"),
        "ORIGINAL DECOY\n",
        "the symlink target must not be overwritten",
    );
}

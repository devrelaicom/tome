//! T051 — per-plugin agent removal through the enable→sync pipeline
//! (Phase 6 / US1, FR-043).
//!
//! Enable two plugins with agents and sync (both emitted), then DISABLE one
//! plugin (drop its `workspace_skills` enrolment) and sync again. Only the
//! disabled plugin's `<plugin>__*` files are removed; the still-enabled
//! plugin's agent files remain untouched.
//!
//! Uses the `StubHarness` with native-agent support — the established
//! chunk-C sync-level fixture (mirrors
//! `tests/harness_sync_stub.rs::native_agents_emit_orphan_removal_and_idempotence`).
//! Removal is a filesystem-glob concern (the `<plugin>__` prefix), so the
//! stub's minimal translation is sufficient; no real-harness rendering is
//! needed.

use std::path::PathBuf;

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::AgentFormat;
use tome::harness::StubHarness;
use tome::harness::sync::{self, SyncDeps, SyncSubsystem};
use tome::workspace::WorkspaceName;

struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    fn build(workspace_name: &str) -> Self {
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
            format!("workspace = \"{workspace_name}\"\nharnesses = [\"stub\"]\n"),
        )
        .expect("write marker config");

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

fn seed_agent_source(paths: &tome::paths::Paths, plugin: &str, name: &str, body: &str) -> String {
    let url = format!("https://example.test/{plugin}.git");
    let cache = paths.cache_dir_for(&url);
    let agent_dir = cache.join(plugin).join("agents");
    std::fs::create_dir_all(&agent_dir).expect("create agent source dir");
    std::fs::write(agent_dir.join(format!("{name}.md")), body).expect("write source agent");
    url
}

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
fn disabling_one_plugin_removes_only_its_agent_files() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(
        StubHarness::default().with_native_agents(AgentFormat::MarkdownYaml),
    )]);

    let fx = Fixture::build("test-workspace");

    // Two plugins, two agents (distinct names so this is purely a removal
    // test, not a clash test).
    let url_a = seed_agent_source(
        &fx.paths,
        "plugin-a",
        "reviewer",
        "---\nname: reviewer\ndescription: A\n---\nYou review.\n",
    );
    let url_b = seed_agent_source(
        &fx.paths,
        "plugin-b",
        "builder",
        "---\nname: builder\ndescription: B\n---\nYou build.\n",
    );
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    for (cat, url) in [("cat-a", &url_a), ("cat-b", &url_b)] {
        tome::index::workspace_catalogs::insert(&conn, "test-workspace", cat, url, "main")
            .expect("enrol catalog");
    }
    drop(conn);
    insert_enabled_agent_row(&fx.paths, "test-workspace", "cat-a", "plugin-a", "reviewer");
    insert_enabled_agent_row(&fx.paths, "test-workspace", "cat-b", "plugin-b", "builder");

    // ----- sync 1: both emitted -----
    sync::sync_project(&fx.project, &fx.deps()).expect("sync 1");
    let agent_dir = fx.project.join(".stub/agents");
    let file_a = agent_dir.join("plugin-a__reviewer.md");
    let file_b = agent_dir.join("plugin-b__builder.md");
    assert!(file_a.is_file(), "plugin-a agent emitted");
    assert!(file_b.is_file(), "plugin-b agent emitted");

    // ----- disable plugin-a, sync 2 -----
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    conn.execute(
        "DELETE FROM workspace_skills WHERE skill_id IN
            (SELECT id FROM skills WHERE plugin = 'plugin-a')",
        [],
    )
    .expect("disable plugin-a agent");
    drop(conn);

    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");

    // Only the disabled plugin's file is removed; the other survives.
    assert!(
        !file_a.exists(),
        "disabled plugin-a agent file removed (FR-043)",
    );
    assert!(
        file_b.is_file(),
        "still-enabled plugin-b agent file must remain untouched (FR-043)",
    );

    // Exactly one agent file removed, and it is plugin-a's.
    let removed: Vec<_> = outcome
        .removed
        .iter()
        .filter(|c| c.subsystem == SyncSubsystem::Agents)
        .collect();
    assert_eq!(removed.len(), 1, "exactly one agent file removed");
    assert!(
        removed[0].path.ends_with("plugin-a__reviewer.md"),
        "the removed file is plugin-a's; got {:?}",
        removed[0].path,
    );
}

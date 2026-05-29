//! T050 — agent name-clash naming through the full enable→sync pipeline
//! (Phase 6 / US1, FR-041 / FR-072).
//!
//! Two enabled plugins ship an agent with the SAME `<name>` (`reviewer`).
//! After `sync_project`:
//!
//! * BOTH agent files exist on disk, namespaced by plugin
//!   (`<pluginA>__reviewer.<ext>` and `<pluginB>__reviewer.<ext>`) — the
//!   filename is ALWAYS namespaced regardless of clash (FR-040);
//! * the DISPLAYED / registered name for each clashing agent is
//!   plugin-prefixed (`<plugin>-reviewer`, FR-041), applied to the clashing
//!   agents only.
//!
//! The displayed name only lands on disk for a real harness whose
//! `translate_agent` writes it into the file (Claude Code writes
//! `name: <displayed>` into the YAML frontmatter), so this drives the real
//! [`tome::harness::claude_code::ClaudeCode`] module rather than the
//! `StubHarness` (whose minimal translation does not surface the displayed
//! name on disk). The sync's clash set is computed once per sync from the
//! workspace scope (FR-072).

mod common;

use std::path::PathBuf;
use std::sync::Mutex;

use common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::claude_code::ClaudeCode;
use tome::harness::sync::{self, SyncDeps, SyncSubsystem};
use tome::workspace::WorkspaceName;

/// Process-global mutex serialising every test in this file — the harness
/// override slot is a single process-global `RwLock`.
static OVERRIDE_MUTEX: Mutex<()> = Mutex::new(());

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
        // Effective list = claude-code (declared in the marker; detection is
        // irrelevant to effective-list resolution).
        std::fs::write(
            marker_dir.join("config.toml"),
            format!("workspace = \"{workspace_name}\"\nharnesses = [\"claude-code\"]\n"),
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
        }
    }
}

/// Seed an on-disk source agent `.md` for `plugin` at the manifest-less
/// fallback path so `resolve_entry_body_path` finds it. Returns the catalog
/// URL.
fn seed_agent_source(paths: &tome::paths::Paths, plugin: &str, name: &str, body: &str) -> String {
    let url = format!("https://example.test/{plugin}.git");
    let cache = paths.cache_dir_for(&url);
    let agent_dir = cache.join(plugin).join("agents");
    std::fs::create_dir_all(&agent_dir).expect("create agent source dir");
    std::fs::write(agent_dir.join(format!("{name}.md")), body).expect("write source agent");
    url
}

/// Insert an enabled `agent`-kind row for `(catalog, plugin, name)` enrolled
/// in `workspace`.
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
fn clashing_agents_namespaced_on_disk_and_prefixed_in_displayed_name() {
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(ClaudeCode)]);

    let fx = Fixture::build("test-workspace");

    // Two DIFFERENT plugins, each shipping an agent named `reviewer` → clash
    // on `<name>` (the clash-set HAVING counts distinct catalog/plugin).
    let url_a = seed_agent_source(
        &fx.paths,
        "plugin-a",
        "reviewer",
        "---\nname: reviewer\ndescription: A reviews\n---\nYou are A.\n",
    );
    let url_b = seed_agent_source(
        &fx.paths,
        "plugin-b",
        "reviewer",
        "---\nname: reviewer\ndescription: B reviews\n---\nYou are B.\n",
    );
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    for (cat, url) in [("cat-a", &url_a), ("cat-b", &url_b)] {
        tome::index::workspace_catalogs::insert(&conn, "test-workspace", cat, url, "main")
            .expect("enrol catalog");
    }
    drop(conn);
    insert_enabled_agent_row(&fx.paths, "test-workspace", "cat-a", "plugin-a", "reviewer");
    insert_enabled_agent_row(&fx.paths, "test-workspace", "cat-b", "plugin-b", "reviewer");

    let outcome = sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    // Both files exist, ALWAYS namespaced by plugin (FR-040), regardless of
    // the clash.
    let agent_dir = fx.project.join(".claude/agents");
    let file_a = agent_dir.join("plugin-a__reviewer.md");
    let file_b = agent_dir.join("plugin-b__reviewer.md");
    assert!(
        file_a.is_file(),
        "plugin-a clashing agent emitted, namespaced"
    );
    assert!(
        file_b.is_file(),
        "plugin-b clashing agent emitted, namespaced"
    );

    // Two agent files added.
    let agent_adds = outcome
        .added
        .iter()
        .filter(|c| c.subsystem == SyncSubsystem::Agents)
        .count();
    assert_eq!(agent_adds, 2, "two clashing agent files added");

    // Displayed name is plugin-PREFIXED (`<plugin>-reviewer`) for the
    // clashing agents (FR-041) — Claude Code writes it into the frontmatter
    // `name:` key.
    let body_a = std::fs::read_to_string(&file_a).unwrap();
    let body_b = std::fs::read_to_string(&file_b).unwrap();
    assert!(
        body_a.contains("name: plugin-a-reviewer"),
        "plugin-a displayed name prefixed on clash (FR-041):\n{body_a}",
    );
    assert!(
        body_b.contains("name: plugin-b-reviewer"),
        "plugin-b displayed name prefixed on clash (FR-041):\n{body_b}",
    );
}

#[test]
fn non_clashing_agent_keeps_clean_displayed_name() {
    // Control: a single plugin's agent (no clash) keeps the clean `<name>` as
    // its displayed name, while the filename stays namespaced.
    let _lock = OVERRIDE_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(ClaudeCode)]);

    let fx = Fixture::build("solo-workspace");
    let url = seed_agent_source(
        &fx.paths,
        "plugin-a",
        "reviewer",
        "---\nname: reviewer\ndescription: A reviews\n---\nYou are A.\n",
    );
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "solo-workspace", "cat-a", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_agent_row(&fx.paths, "solo-workspace", "cat-a", "plugin-a", "reviewer");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let file = fx.project.join(".claude/agents/plugin-a__reviewer.md");
    assert!(
        file.is_file(),
        "filename stays namespaced even without clash"
    );
    let body = std::fs::read_to_string(&file).unwrap();
    assert!(
        body.contains("name: reviewer") && !body.contains("name: plugin-a-reviewer"),
        "no clash → clean displayed name (FR-041):\n{body}",
    );
}

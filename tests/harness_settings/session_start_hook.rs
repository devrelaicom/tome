//! The Tome-owned Claude Code SessionStart hook lands in
//! `settings.local.json` on a sync with a live `RealJson` harness and at least
//! one enabled plugin.
//!
//! Mirrors the `harness_sync_p6_idempotence` scaffold: a single `StubHarness`
//! configured `RealJson` + `with_hook_settings()` drives the hooks sink, one
//! enabled plugin makes the workspace non-empty, and a single `sync_project`
//! exercises the reconciler. The reconciler unconditionally injects the
//! SessionStart entry into `prepared` once a RealJson harness participates, so
//! the entry must appear under `hooks.SessionStart` after the sync.

use std::path::PathBuf;

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use serde_json::Value as JsonValue;
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps};
use tome::harness::{HooksStrategy, StubHarness};
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
        }
    }
}

/// Seed an on-disk plugin `hooks/hooks.json` so the hooks pass has at least one
/// plugin source; returns the catalog URL. (The Tome SessionStart entry is
/// injected regardless, but a real enabled plugin makes the scenario faithful.)
fn seed_hooks_source(paths: &tome::paths::Paths, plugin: &str, body: &str) -> String {
    let url = format!("https://example.test/{plugin}.git");
    let hooks_dir = paths.cache_dir_for(&url).join(plugin).join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks source dir");
    std::fs::write(hooks_dir.join("hooks.json"), body).expect("write source hooks.json");
    url
}

/// Insert an enabled skill row for `(catalog, plugin, name)`; a single enabled
/// row makes the plugin appear in `enabled_plugins_for_workspace`.
fn insert_enabled_skill_row(
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
         VALUES (?1, ?2, ?3, 'skill', 'desc', '0.0.0', ?4, 'h', 1, 0, NULL, '1970-01-01T00:00:00Z')",
        rusqlite::params![catalog, plugin, name, format!("skills/{name}/SKILL.md")],
    )
    .expect("insert skill row");
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind='skill' AND name=?3",
            rusqlite::params![catalog, plugin, name],
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
fn sync_installs_tome_session_start_hook_for_claude_code() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(
        StubHarness::default()
            .with_hooks_strategy(HooksStrategy::RealJson)
            .with_hook_settings(),
    )]);

    let fx = Fixture::build("test-workspace", "\"stub\"");

    let url = seed_hooks_source(
        &fx.paths,
        "plugin-a",
        r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/guard.sh" } ] } ] }"#,
    );

    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-workspace", "cat", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-workspace", "cat", "plugin-a", "skill-a");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let hooks_path = fx.project.join(".stub/settings.local.json");
    assert!(hooks_path.is_file(), "settings.local.json created on sync");

    let doc: JsonValue =
        serde_json::from_str(&std::fs::read_to_string(&hooks_path).expect("read settings"))
            .expect("parse settings");

    let session_start = doc["hooks"]["SessionStart"]
        .as_array()
        .expect("SessionStart event array present");
    let has_session_context = session_start.iter().any(|entry| {
        entry["hooks"][0]["command"]
            .as_str()
            .is_some_and(|c| c.contains("harness session-context"))
    });
    assert!(
        has_session_context,
        "a SessionStart entry must invoke `harness session-context`; got: {doc}"
    );

    // A second sync must be a byte-for-byte no-op (deterministic entry).
    let before = std::fs::read_to_string(&hooks_path).unwrap();
    sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");
    let after = std::fs::read_to_string(&hooks_path).unwrap();
    assert_eq!(
        before, after,
        "re-sync must not change settings.local.json (idempotent)"
    );
}

//! US6.1 — `[hooks] translate_plugin_hooks = false` opt-out gate.
//!
//! Verifies that when the global config carries `translate_plugin_hooks = false`:
//!
//! 1. A fresh sync that WOULD write run-hook entries and a manifest writes nothing
//!    instead (gate prevents initial registration).
//! 2. A live sync followed by toggling the opt-out removes the pre-existing
//!    run-hook entries and deletes the manifest (clean-up guarantee).
//!
//! Both tests drive the REAL `sync_project` path (not hand-built snapshots).

use std::path::{Path, PathBuf};

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps};
use tome::workspace::WorkspaceName;

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

impl Fixture {
    fn build(workspace_name: &str, harnesses_toml: &str) -> Self {
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
            format!("workspace = \"{workspace_name}\"\nharnesses = [{harnesses_toml}]\n"),
        )
        .expect("write marker");
        std::fs::write(marker_dir.join("RULES.md"), "ROUTING DIRECTIVE BODY\n")
            .expect("write rules");

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
        }
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Seed a plugin with a `hooks/hooks.json` so the dispatch reconciler has hooks
/// to register. Returns the catalog URL so the caller can enrol it.
fn seed_hooks_source(paths: &tome::paths::Paths, plugin: &str, body: &str) -> String {
    let url = format!("https://example.test/{plugin}.git");
    let hooks_dir = paths.cache_dir_for(&url).join(plugin).join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks source dir");
    std::fs::write(hooks_dir.join("hooks.json"), body).expect("write source hooks.json");
    url
}

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// With `[hooks] translate_plugin_hooks = false` in the global config, a sync
/// that WOULD otherwise write run-hook entries + a manifest writes nothing.
#[test]
fn opt_out_gate_writes_nothing() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::Cursor)]);

    let fx = Fixture::build("test-ws", "\"cursor\"");

    // Write the opt-out config BEFORE the first sync.
    std::fs::write(
        &fx.paths.global_config_file,
        "[hooks]\ntranslate_plugin_hooks = false\n",
    )
    .expect("write opt-out global config");

    // Seed a plugin with a PreToolUse command hook.
    let url = seed_hooks_source(
        &fx.paths,
        "plugin-a",
        r#"{ "PreToolUse": [ { "hooks": [ { "type": "command", "command": "/opt/guard.sh" } ] } ] }"#,
    );
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat", "plugin-a");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync with opt-out");

    // (a) No manifest must be written.
    let manifest_path = fx.paths.hooks_manifest(&fx.workspace, "cursor");
    assert!(
        !manifest_path.exists(),
        "manifest must NOT be written when translate_plugin_hooks=false"
    );

    // (b) No Tome run-hook entry in the cursor hook file. The file might not
    // even exist (cursor has no session-steering, so the only writer is the
    // dispatch reconciler, which the opt-out gate suppresses).
    let hooks_path = fx.project.join(".cursor/hooks.json");
    if hooks_path.exists() {
        let doc: serde_json::Value =
            serde_json::from_str(&read(&hooks_path)).expect("parse cursor hooks.json");
        let none_or_empty = doc["hooks"].get("preToolUse").is_none()
            || doc["hooks"]["preToolUse"]
                .as_array()
                .is_some_and(|a| a.is_empty());
        assert!(
            none_or_empty,
            "no Tome run-hook entry must be written when opt-out is active:\n{doc}"
        );
    }
}

/// Toggle sequence: sync once with hooks ENABLED (entries + manifest written),
/// then add `translate_plugin_hooks = false` to the global config and sync
/// again. The opt-out pass must REMOVE the run-hook entries and DELETE the
/// manifest (reversible, clean teardown).
#[test]
fn opt_out_gate_cleans_up_existing_entries() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::Cursor)]);

    let fx = Fixture::build("test-ws", "\"cursor\"");

    // Seed a plugin with a PreToolUse command hook.
    let url = seed_hooks_source(
        &fx.paths,
        "plugin-a",
        r#"{ "PreToolUse": [ { "hooks": [ { "type": "command", "command": "/opt/guard.sh" } ] } ] }"#,
    );
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-ws", "cat", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-ws", "cat", "plugin-a");

    // ---- Phase 1: sync with hooks ENABLED (no config file → defaults to true) ----
    sync::sync_project(&fx.project, &fx.deps()).expect("sync 1 (hooks on)");

    let hooks_path = fx.project.join(".cursor/hooks.json");
    let manifest_path = fx.paths.hooks_manifest(&fx.workspace, "cursor");

    assert!(
        hooks_path.is_file(),
        "cursor hook file must be written after hooks-enabled sync"
    );
    assert!(
        manifest_path.is_file(),
        "manifest must be written after hooks-enabled sync"
    );
    {
        let doc: serde_json::Value =
            serde_json::from_str(&read(&hooks_path)).expect("parse cursor hooks.json");
        assert!(
            doc["hooks"]["preToolUse"]
                .as_array()
                .is_some_and(|a| !a.is_empty()),
            "preToolUse run-hook entry must be present after hooks-enabled sync:\n{doc}"
        );
    }

    // ---- Phase 2: write opt-out config + re-sync → cleanup ----
    std::fs::write(
        &fx.paths.global_config_file,
        "[hooks]\ntranslate_plugin_hooks = false\n",
    )
    .expect("write opt-out global config");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 2 (hooks off)");

    // (a) Manifest must be deleted.
    assert!(
        !manifest_path.exists(),
        "manifest must be deleted after opt-out sync"
    );

    // (b) Tome run-hook entries must be stripped from the hook file.
    if hooks_path.exists() {
        let doc: serde_json::Value = serde_json::from_str(&read(&hooks_path))
            .expect("parse cursor hooks.json after opt-out");
        let none_or_empty = doc["hooks"].get("preToolUse").is_none()
            || doc["hooks"]["preToolUse"]
                .as_array()
                .is_some_and(|a| a.is_empty());
        assert!(
            none_or_empty,
            "Tome run-hook entry must be stripped after opt-out sync:\n{doc}"
        );
    }
}

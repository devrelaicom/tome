//! The Tome-owned **Codex** `SessionStart` routing hook lands in
//! `<project>/.codex/hooks.json` on a sync where `codex` is live, is pruned
//! when `codex` goes non-live, and — critically — NEVER carries plugin hooks.
//!
//! Codex routes through `reconcile_tome_session_hooks` (the non-`RealJson`
//! path), NOT the Claude-Code plugin-hooks pass. Its entry is the
//! `codex_session_start_hook` shape (`{"matcher":"startup|resume","hooks":[…]}`)
//! under `{"hooks":{"SessionStart":[…]}}`. These tests use the REAL `codex`
//! module from `SUPPORTED_HARNESSES` (no registry override) so the genuine
//! `tome_session_hook_path` + reconciler run; `codex` is made live purely via
//! the project marker's `harnesses = ["codex"]`.
//!
//! Test #3 (`codex_session_hook_never_contains_plugin_hooks`) is the
//! load-bearing guard: it enables a plugin that ships a VALID `hooks/hooks.json`
//! (a `PreToolUse` entry) in the workspace, then asserts the Codex sink's
//! top-level `hooks` object has EXACTLY the `SessionStart` key — proving plugin
//! hooks are not mapped onto Codex.

use std::path::PathBuf;

use crate::common::{ToolEnv, paths_for, seed_workspace};
use serde_json::Value as JsonValue;
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps};
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
        }
    }
}

/// Seed an on-disk plugin `hooks/hooks.json` (CC-format), mirroring the
/// `session_start_hook.rs` helper. Returns the catalog URL so the caller can
/// enrol it. `plugin_root_dir` resolves a manifest-less catalog clone to
/// `cache_dir_for(url).join(plugin)`, so the hooks tree lands where the hooks
/// reconciler reads it.
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

/// Read + parse `<project>/.codex/hooks.json`.
fn read_codex_hooks(project: &std::path::Path) -> JsonValue {
    let p = project.join(".codex/hooks.json");
    assert!(
        p.is_file(),
        "`.codex/hooks.json` must exist; not found at {p:?}"
    );
    serde_json::from_str(&std::fs::read_to_string(&p).expect("read .codex/hooks.json"))
        .expect("parse .codex/hooks.json")
}

#[test]
fn sync_installs_tome_session_start_hook_for_codex() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let fx = Fixture::build("test-workspace", "\"codex\"");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    let doc = read_codex_hooks(&fx.project);
    let session_start = doc["hooks"]["SessionStart"]
        .as_array()
        .expect("SessionStart event array present");
    assert_eq!(
        session_start.len(),
        1,
        "exactly one SessionStart entry (the Tome one); got: {doc}"
    );
    let entry = &session_start[0];
    assert_eq!(
        entry["matcher"].as_str(),
        Some("startup|resume"),
        "Codex matcher must be `startup|resume`; got: {doc}"
    );
    assert_eq!(
        entry["hooks"][0]["type"].as_str(),
        Some("command"),
        "nested hook type must be `command`; got: {doc}"
    );
    let command = entry["hooks"][0]["command"]
        .as_str()
        .expect("nested hook command present");
    assert!(
        command.contains("harness session-start"),
        "command must invoke `harness session-start`; got: {command}"
    );
    assert!(
        command.contains("--workspace"),
        "command must pin `--workspace`; got: {command}"
    );

    // A second sync must be a byte-for-byte no-op (deterministic entry).
    let hooks_path = fx.project.join(".codex/hooks.json");
    let before = std::fs::read(&hooks_path).unwrap();
    sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");
    let after = std::fs::read(&hooks_path).unwrap();
    assert_eq!(
        before, after,
        "re-sync must not change .codex/hooks.json (idempotent)"
    );
}

/// When `codex` is dropped from the effective harness list,
/// `reconcile_tome_session_hooks` removes the deep-equal Tome entry and
/// `prune_empty_event` removes the now-empty `SessionStart` key.
#[test]
fn sync_removes_tome_session_start_hook_when_codex_non_live() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let fx = Fixture::build("test-workspace", "\"codex\"");

    // ----- sync 1: SessionStart entry lands -----
    sync::sync_project(&fx.project, &fx.deps()).expect("sync 1");

    let doc = read_codex_hooks(&fx.project);
    assert!(
        doc["hooks"]["SessionStart"]
            .as_array()
            .is_some_and(|a| a.iter().any(|e| e["hooks"][0]["command"]
                .as_str()
                .is_some_and(|c| c.contains("harness session-start")))),
        "SessionStart must contain the Tome entry after sync 1; got: {doc}"
    );

    // ----- sync 2: drop codex from the effective list -----
    std::fs::write(
        fx.project.join(".tome/config.toml"),
        "workspace = \"test-workspace\"\nharnesses = []\n",
    )
    .expect("rewrite marker to empty harness list");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 2");

    // The `SessionStart` key must be gone (its only entry — the Tome one — was
    // removed and the empty array pruned).
    let doc2 = read_codex_hooks(&fx.project);
    assert!(
        doc2.get("hooks")
            .and_then(|h| h.get("SessionStart"))
            .is_none(),
        "`SessionStart` key must be pruned when codex goes non-live; got: {doc2}"
    );
}

/// THE LOAD-BEARING GUARD: an enabled plugin that ships a VALID
/// `hooks/hooks.json` (a `PreToolUse` entry) must NOT leak into the Codex sink.
/// Codex carries ONLY Tome's own `SessionStart` routing hook — plugin hooks are
/// never mapped onto Codex. A regression that started mapping plugin hooks to
/// Codex would surface a `PreToolUse` key here and fail this test.
#[test]
fn codex_session_hook_never_contains_plugin_hooks() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());

    let fx = Fixture::build("test-workspace", "\"codex\"");

    // Seed a plugin shipping a VALID CC-format hooks.json with a PreToolUse
    // entry, enrol its catalog, and enable one of its skills in the workspace.
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

    let doc = read_codex_hooks(&fx.project);

    // The Codex sink's top-level `hooks` object must have EXACTLY one key:
    // `SessionStart`. No `PreToolUse` (or any other plugin event) may leak in.
    let hooks_obj = doc["hooks"]
        .as_object()
        .expect("top-level `hooks` object present");
    let keys: Vec<&str> = hooks_obj.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec!["SessionStart"],
        "Codex `hooks` keys must be exactly [\"SessionStart\"] — no plugin event leaked; got: {doc}"
    );

    // And the single SessionStart entry is the Tome one.
    let session_start = doc["hooks"]["SessionStart"]
        .as_array()
        .expect("SessionStart event array present");
    assert_eq!(
        session_start.len(),
        1,
        "exactly one SessionStart entry (the Tome one); got: {doc}"
    );
    assert!(
        session_start[0]["hooks"][0]["command"]
            .as_str()
            .is_some_and(|c| c.contains("harness session-start")),
        "the single SessionStart entry must be the Tome routing hook; got: {doc}"
    );
}

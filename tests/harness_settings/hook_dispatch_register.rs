//! US3.2 — the plugin-hook dispatch reconciler, end-to-end through the REAL
//! `sync_project` path.
//!
//! A single enabled plugin ships a `hooks/hooks.json` with ONE `PreToolUse`
//! command hook. Driving the genuine `sync_project` over a real harness whose
//! `hook_support()` covers `PreToolUse` must:
//!
//! 1. register a Tome `run-hook` dispatcher entry under the harness-NATIVE
//!    event key (cursor's `preToolUse`) — and NO entry for an unused event
//!    (cursor's `stop`), proving the used-event-only filter; and
//! 2. write the resolved per-(workspace, harness) manifest keyed by the CC event
//!    name with the per-plugin matcher carried verbatim.
//!
//! Cursor is the harness under test because it has `hook_support()` but NO
//! `session_steering()` / `tome_session_hook_path`, so `.cursor/hooks.json` is
//! owned SOLELY by the dispatch reconciler — the assertions are not entangled
//! with the session-steering writers (those have their own pins).

use std::path::{Path, PathBuf};

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
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
            dry_run: false,
        }
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Seed a manifest-less catalog enrolment plus an on-disk plugin
/// `hooks/hooks.json`, returning the catalog URL.
fn seed_hooks_source(paths: &tome::paths::Paths, plugin: &str, body: &str) -> String {
    let url = format!("https://example.test/{plugin}.git");
    let hooks_dir = paths.cache_dir_for(&url).join(plugin).join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks source dir");
    std::fs::write(hooks_dir.join("hooks.json"), body).expect("write source hooks.json");
    url
}

/// Insert an enabled `skill`-kind row so the plugin shows up in the workspace's
/// enabled-plugin enumeration (which drives the dispatch reconciler).
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
fn sync_writes_run_hook_entries_and_manifest_for_used_events_only() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::Cursor)]);

    let fx = Fixture::build("test-workspace", "\"cursor\"");

    // ONE plugin, ONE PreToolUse command hook. The command carries no
    // substitution tokens, so it survives the rewrite verbatim (a deterministic
    // manifest handler).
    let url = seed_hooks_source(
        &fx.paths,
        "plugin-a",
        r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "/opt/guard.sh check" } ] } ] }"#,
    );
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-workspace", "cat", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-workspace", "cat", "plugin-a");

    // ----- drive the REAL sync -----
    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    // (a) the cursor hook file carries the run-hook dispatcher entry under the
    // harness-NATIVE PreToolUse key (`preToolUse`), version-stamped.
    let hooks_path = fx.project.join(".cursor/hooks.json");
    assert!(hooks_path.is_file(), "cursor hook file must be written");
    let doc: serde_json::Value = serde_json::from_str(&read(&hooks_path)).unwrap();
    assert_eq!(doc["version"], 1, "cursor hook file is version-stamped");
    let arr = doc["hooks"]["preToolUse"]
        .as_array()
        .expect("preToolUse array present");
    assert_eq!(arr.len(), 1, "exactly one Tome run-hook entry");
    // #337 Phase B: the LAUNCHER prefix is resolved; the byte-stable args suffix
    // is the ownership marker — assert via the launcher-tolerant recogniser.
    {
        let cmd = arr[0]["command"].as_str().expect("command is a string");
        assert!(
            tome::harness::launcher::looks_like_tome_hook_command(
                cmd,
                "harness run-hook --event PreToolUse --harness cursor --workspace test-workspace",
            ),
            "run-hook command must be a recognised tome hook command; got: {cmd}",
        );
    }
    assert_eq!(arr[0]["type"], "command");

    // (b) NO entry for an UNUSED event (cursor's Stop native key is `stop`).
    assert!(
        doc["hooks"].get("stop").is_none(),
        "no run-hook entry for the unused Stop event:\n{doc}",
    );

    // (c) the resolved manifest exists, keyed by the CC event name, with the
    // per-plugin matcher carried verbatim and the (token-free) handler command.
    let manifest_path = fx.paths.hooks_manifest(&fx.workspace, "cursor");
    assert!(manifest_path.is_file(), "manifest must be written");
    let manifest: serde_json::Value = serde_json::from_str(&read(&manifest_path)).unwrap();
    assert_eq!(manifest["harness"], "cursor");
    let events = manifest["events"].as_object().expect("events object");
    assert_eq!(events.len(), 1, "only the used event is in the manifest");
    let entries = manifest["events"]["PreToolUse"]
        .as_array()
        .expect("PreToolUse manifest entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["plugin"], "cat:plugin-a");
    assert_eq!(entries[0]["matcher"], "Bash");
    assert_eq!(entries[0]["handler"]["type"], "command");
    assert_eq!(entries[0]["handler"]["command"], "/opt/guard.sh check");
    // CC seconds → manifest ms: the source had no timeout, so it is absent.
    assert!(entries[0].get("timeout_ms").is_none());
    assert!(
        manifest["events"].get("Stop").is_none(),
        "the unused Stop event must not appear in the manifest",
    );
}

#[test]
fn non_live_dispatch_teardown_strips_run_hook_entries_and_deletes_manifest() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::Cursor)]);

    // ---- Sync 1: cursor is LIVE with one enabled plugin ----
    let fx = Fixture::build("test-workspace", "\"cursor\"");

    let url = seed_hooks_source(
        &fx.paths,
        "plugin-a",
        r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "/opt/guard.sh check" } ] } ] }"#,
    );
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-workspace", "cat", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-workspace", "cat", "plugin-a");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync 1 (LIVE)");

    let hooks_path = fx.project.join(".cursor/hooks.json");
    let manifest_path = fx.paths.hooks_manifest(&fx.workspace, "cursor");

    // Both the hook entry and the manifest must be present after the LIVE sync.
    assert!(
        hooks_path.is_file(),
        "cursor hook file must exist after LIVE sync"
    );
    assert!(
        manifest_path.is_file(),
        "manifest must exist after LIVE sync"
    );
    {
        let doc: serde_json::Value = serde_json::from_str(&read(&hooks_path)).unwrap();
        assert!(
            doc["hooks"]["preToolUse"]
                .as_array()
                .is_some_and(|a| !a.is_empty()),
            "preToolUse run-hook entry must be present after LIVE sync",
        );
    }

    // ---- Transition: remove cursor from the effective harness set ----
    std::fs::write(
        fx.project.join(".tome/config.toml"),
        "workspace = \"test-workspace\"\nharnesses = []\n",
    )
    .expect("rewrite config to non-live");

    // ---- Sync 2: NON-LIVE — reconciler must clean up ----
    sync::sync_project(&fx.project, &fx.deps()).expect("sync 2 (NON-LIVE)");

    // (a) The manifest must be deleted.
    assert!(
        !manifest_path.exists(),
        "manifest must be deleted after NON-LIVE sync",
    );

    // (b) The run-hook entry must be stripped from the hook file. Cursor has no
    // session-steering, so the file is entirely tome-dispatch-owned; the
    // `preToolUse` key must be absent (pruned) after the only entry is removed.
    if hooks_path.exists() {
        let doc: serde_json::Value = serde_json::from_str(&read(&hooks_path)).unwrap();
        assert!(
            doc["hooks"].get("preToolUse").is_none()
                || doc["hooks"]["preToolUse"]
                    .as_array()
                    .is_some_and(|a| a.is_empty()),
            "preToolUse run-hook entry must be stripped after NON-LIVE sync:\n{doc}",
        );
    }
}

// ---------------------------------------------------------------------------
// Fix 4, US6 review: prompt-gate through the REAL sync
// ---------------------------------------------------------------------------

/// The hooks.json source used for the prompt-gate sync tests: a single
/// PreToolUse prompt handler. Written to the plugin's on-disk directory.
const PROMPT_HOOKS_JSON: &str = r#"{
    "PreToolUse": [
        {
            "hooks": [
                { "type": "prompt", "prompt": "Is this tool use safe?" }
            ]
        }
    ]
}"#;

/// Fix 4: without any prompt provider configured (default `Config`), the
/// reconciler's prompt gate (`prompt_enabled = false`) must DROP the
/// `Handler::Prompt` entry from the written manifest — verifying that the
/// gate works end-to-end through the real `sync_project` path.
///
/// This prevents the manifest from carrying an unreachable prompt entry that
/// would silently fail-open on every dispatch until a provider is configured.
#[test]
fn prompt_gate_drops_entry_without_provider() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::Cursor)]);

    let fx = Fixture::build("test-ws-prompt-gate-off", "\"cursor\"");

    // Seed one plugin with a prompt handler in its hooks.json.
    let url = seed_hooks_source(&fx.paths, "plugin-prompt", PROMPT_HOOKS_JSON);
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-ws-prompt-gate-off", "cat", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-ws-prompt-gate-off", "cat", "plugin-prompt");

    // Sync with NO config.toml → Config::default() → prompt_enabled = false.
    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    // The manifest must NOT carry any prompt entry (gate dropped it).
    let manifest_path = fx.paths.hooks_manifest(&fx.workspace, "cursor");
    if manifest_path.is_file() {
        let manifest: serde_json::Value =
            serde_json::from_str(&read(&manifest_path)).expect("parse manifest");
        // Either the events object is missing PreToolUse, or its entries array is
        // empty (no prompt entries survived the gate filter).
        let entries = manifest["events"]["PreToolUse"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for entry in &entries {
            let handler_type = entry["handler"]["type"].as_str().unwrap_or("");
            assert_ne!(
                handler_type, "prompt",
                "prompt entry must be dropped from manifest when no provider is configured:\n{manifest}"
            );
        }
    }
    // (If the manifest file is absent the gate worked: nothing to write → no file.)
}

/// Fix 4: WITH a prompt configuration set (`prompt_model` alone is sufficient
/// to enable the gate), the reconciler must KEEP the `Handler::Prompt` entry in
/// the written manifest so the runtime can execute it.
///
/// The config is written to the tome root's `config.toml` so
/// `load_or_default(deps.paths)` inside `sync_project` picks it up.
#[test]
fn prompt_gate_keeps_entry_with_provider_configured() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::Cursor)]);

    let fx = Fixture::build("test-ws-prompt-gate-on", "\"cursor\"");

    // Write a minimal config.toml that enables the prompt gate (prompt_model
    // alone is sufficient — the gate is `prompt_provider.is_some() ||
    // prompt_model.is_some()`). No need for a real provider in the registry
    // for this sync-path test; the gate logic only inspects these two fields.
    std::fs::write(
        &fx.paths.global_config_file,
        "[hooks]\nprompt_model = \"gpt-4o-mini\"\n",
    )
    .expect("write config.toml");

    // Seed one plugin with a prompt handler.
    let url = seed_hooks_source(&fx.paths, "plugin-prompt", PROMPT_HOOKS_JSON);
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-ws-prompt-gate-on", "cat", &url, "main")
        .expect("enrol catalog");
    drop(conn);
    insert_enabled_skill_row(&fx.paths, "test-ws-prompt-gate-on", "cat", "plugin-prompt");

    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    // The manifest MUST exist and carry the prompt entry (gate kept it).
    let manifest_path = fx.paths.hooks_manifest(&fx.workspace, "cursor");
    assert!(
        manifest_path.is_file(),
        "manifest must be written when prompt provider is configured"
    );
    let manifest: serde_json::Value =
        serde_json::from_str(&read(&manifest_path)).expect("parse manifest");
    let entries = manifest["events"]["PreToolUse"]
        .as_array()
        .expect("PreToolUse entries must be present");
    let has_prompt = entries
        .iter()
        .any(|e| e["handler"]["type"].as_str() == Some("prompt"));
    assert!(
        has_prompt,
        "manifest must carry the prompt entry when prompt_model is configured:\n{manifest}"
    );
}

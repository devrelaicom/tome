//! US3.3 + US7 byte-stable registration + manifest pins for `cursor`.
//!
//! Drives the REAL `sync_project` over the `cursor` module with ONE enabled
//! plugin shipping a `PreToolUse` Bash command hook (a token-free command, so
//! the rewritten manifest handler is deterministic), then pins:
//!
//! 1. the exact on-disk `run-hook` dispatcher entry — registered under the
//!    harness-NATIVE event key (`preToolUse`) under the `hooks` container;
//! 2. the exact on-disk session-steering entry — registered under the
//!    harness-NATIVE event key (`sessionStart`, camelCase) under `hooks`; and
//! 3. the exact resolved manifest JSON — keyed by the CC event name, with the
//!    per-plugin matcher carried verbatim and `timeout_ms` baked from CC seconds.
//!
//! The literals were captured from the implementation output once and pinned
//! (byte-stable). The Tome `run-hook` and `sessionStart` entries are SEPARATE
//! additive leaves; they compose and neither clobbers the other.

use std::path::{Path, PathBuf};

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps};
use tome::workspace::WorkspaceName;

/// The exact `run-hook` event-array under the native event key (pretty-printed).
const HOOK_ARRAY: &str = r##"[
  {
    "type": "command",
    "command": "tome harness run-hook --event PreToolUse --harness cursor --workspace test-workspace"
  }
]"##;

/// The exact session-steering entry array under `sessionStart` (camelCase, US7).
/// Cursor's native event key is camelCase — distinct from the PascalCase used by
/// Devin / Copilot / Gemini which all share a CC-compatible PascalCase key.
const SESSION_START_ARRAY: &str = r##"[
  {
    "type": "command",
    "command": "tome harness session-start --workspace test-workspace --harness cursor"
  }
]"##;

/// The exact resolved manifest bytes (pretty-printed + trailing newline).
const MANIFEST: &str = r##"{
  "harness": "cursor",
  "raw_event_passthrough": false,
  "events": {
    "PreToolUse": [
      {
        "plugin": "cat:plugin-a",
        "matcher": "Bash",
        "handler": {
          "type": "command",
          "command": "/opt/guard.sh check"
        }
      }
    ]
  }
}
"##;

struct Fixture {
    _home: TempDir,
    paths: tome::paths::Paths,
    project: PathBuf,
    workspace: WorkspaceName,
}

fn build() -> Fixture {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).expect("create tome root");
    seed_workspace(&paths, "test-workspace");
    let workspace = WorkspaceName::parse("test-workspace").expect("parse workspace");

    let project = env.home_path().join("project");
    std::fs::create_dir_all(&project).expect("create project");
    let marker_dir = project.join(".tome");
    std::fs::create_dir_all(&marker_dir).expect("create marker dir");
    std::fs::write(
        marker_dir.join("config.toml"),
        "workspace = \"test-workspace\"\nharnesses = [\"cursor\"]\n",
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

/// Seed a plugin shipping a `PreToolUse` Bash command hook + enrol/enable it.
fn seed_plugin(fx: &Fixture) {
    let url = String::from("https://example.test/plugin-a.git");
    let hooks_dir = fx.paths.cache_dir_for(&url).join("plugin-a").join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create hooks source dir");
    std::fs::write(
        hooks_dir.join("hooks.json"),
        r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [ { "type": "command", "command": "/opt/guard.sh check" } ] } ] }"#,
    )
    .expect("write source hooks.json");

    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-workspace", "cat", &url, "main")
        .expect("enrol catalog");
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES ('cat','plugin-a','demo','skill','d','0.0.0','skills/demo/SKILL.md','h',1,0,NULL,'1970-01-01T00:00:00Z')",
        [],
    )
    .expect("insert skill row");
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog='cat' AND plugin='plugin-a'",
            [],
            |r| r.get(0),
        )
        .expect("skill id");
    let ws_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name='test-workspace'",
            [],
            |r| r.get(0),
        )
        .expect("ws id");
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
        rusqlite::params![ws_id, skill_id],
    )
    .expect("enrol skill");
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn cursor_run_hook_registration_and_manifest_pins() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::Cursor)]);

    let fx = build();
    seed_plugin(&fx);
    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    // ----- (a) the on-disk run-hook registration entry -----
    let hook_path = fx.project.join(".cursor/hooks.json");
    let mut doc: serde_json::Value = serde_json::from_str(&read(&hook_path)).unwrap();
    // #337 Phase B: canonicalise BOTH the run-hook and session-start launcher
    // prefixes to bare `tome` so the structural byte-pins stay deterministic.
    crate::common::canonicalize_tome_hook_command_leaves(
        &mut doc,
        &[
            "harness run-hook --event PreToolUse --harness cursor --workspace test-workspace",
            "harness session-start --workspace test-workspace --harness cursor",
        ],
    );
    assert_eq!(doc["version"], 1, "the hook file is version-stamped");
    let arr = &doc["hooks"]["preToolUse"];
    assert_eq!(
        serde_json::to_string_pretty(arr).unwrap(),
        HOOK_ARRAY,
        "run-hook entry bytes drifted for cursor",
    );

    // Non-leak: the plugin's verbatim command must NOT appear in the hook file.
    let raw = read(&hook_path);
    assert!(
        !raw.contains("/opt/guard.sh check"),
        "plugin command must not leak into hook file:\n{raw}",
    );

    // US7: Cursor now delivers session steering via `.cursor/hooks.json` (the
    // keep-both pattern — rules file + hook, same as Devin/Gemini/Copilot).
    // The key is camelCase `sessionStart` (Cursor's native wire), NOT PascalCase.
    let session_arr = &doc["hooks"]["sessionStart"];
    assert_eq!(
        serde_json::to_string_pretty(session_arr).unwrap(),
        SESSION_START_ARRAY,
        "session-steering entry bytes drifted for cursor",
    );
    // Verify no stray PascalCase `SessionStart` key (wrong for Cursor).
    assert!(
        doc["hooks"].get("SessionStart").is_none(),
        "cursor hook file must NOT use PascalCase SessionStart (use camelCase sessionStart)",
    );

    // ----- (b) the resolved manifest -----
    let manifest_path = fx.paths.hooks_manifest(&fx.workspace, "cursor");
    // `plugin_root` (Fix 1, US8 review) is a temp-dir-derived path and cannot be
    // byte-pinned. Extract and verify it separately, then compare the remainder
    // against the byte-stable MANIFEST constant.
    let manifest_text = read(&manifest_path);
    let mut manifest_val: serde_json::Value =
        serde_json::from_str(&manifest_text).expect("manifest is valid JSON");
    let got_root = manifest_val["events"]["PreToolUse"][0]
        .as_object_mut()
        .expect("entry is an object")
        .shift_remove("plugin_root")
        .and_then(|v| v.as_str().map(|s| s.to_owned()))
        .expect("plugin_root must be present in the manifest entry");
    let expected_root = fx
        .paths
        .cache_dir_for("https://example.test/plugin-a.git")
        .join("plugin-a")
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        got_root, expected_root,
        "plugin_root must be the plugin install root, not the catalog data dir (Fix 1)"
    );
    let got_rest = serde_json::to_string_pretty(&manifest_val).unwrap() + "\n";
    assert_eq!(got_rest, MANIFEST, "manifest bytes drifted for cursor");
}

impl Fixture {
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

// =====================================================================
// US7 coexistence: a plugin's `SessionStart` run-hook dispatch entry and
// the session-steering entry both land under `hooks.sessionStart`, and
// selective removal leaves only the surviving entry intact.
// =====================================================================

/// Seed a plugin shipping a `SessionStart` command hook + enrol/enable it.
/// Returns `(workspace_id, skill_id)` so the caller can unenroll later.
fn seed_session_start_plugin(fx: &Fixture) -> (i64, i64) {
    let url = String::from("https://example.test/plugin-session.git");
    let hooks_dir = fx
        .paths
        .cache_dir_for(&url)
        .join("plugin-session")
        .join("hooks");
    std::fs::create_dir_all(&hooks_dir).expect("create session plugin hooks dir");
    std::fs::write(
        hooks_dir.join("hooks.json"),
        r#"{ "SessionStart": [ { "matcher": "", "hooks": [ { "type": "command", "command": "/opt/trigger.sh start" } ] } ] }"#,
    )
    .expect("write session plugin hooks.json");

    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw");
    tome::index::workspace_catalogs::insert(&conn, "test-workspace", "cat", &url, "main")
        .expect("enrol session catalog");
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES ('cat','plugin-session','demo','skill','d','0.0.0','skills/demo/SKILL.md','hs',1,0,NULL,'1970-01-01T00:00:00Z')",
        [],
    )
    .expect("insert session skill row");
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog='cat' AND plugin='plugin-session'",
            [],
            |r| r.get(0),
        )
        .expect("session skill id");
    let ws_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name='test-workspace'",
            [],
            |r| r.get(0),
        )
        .expect("ws id for session plugin");
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
        rusqlite::params![ws_id, skill_id],
    )
    .expect("enrol session skill");
    (ws_id, skill_id)
}

/// Remove the plugin from the workspace (selective removal: the session-steering
/// entry must survive after this).
fn unenroll_session_start_plugin(fx: &Fixture, ws_id: i64, skill_id: i64) {
    let conn = rusqlite::Connection::open(&fx.paths.index_db).expect("open rw for unenroll");
    conn.execute(
        "DELETE FROM workspace_skills WHERE workspace_id=?1 AND skill_id=?2",
        rusqlite::params![ws_id, skill_id],
    )
    .expect("unenrol session skill");
}

/// The byte-stable run-hook dispatch ARGS SUFFIX for SessionStart under cursor
/// (#337 Phase B — the launcher prefix is resolved, the suffix is the marker).
const SESSION_START_RUN_HOOK_SUFFIX: &str =
    "harness run-hook --event SessionStart --harness cursor --workspace test-workspace";

/// The byte-stable session-steering ARGS SUFFIX for cursor (US7, #337 Phase B).
const SESSION_STEERING_SUFFIX: &str =
    "harness session-start --workspace test-workspace --harness cursor";

/// `true` iff `entry`'s `command` is a recognised tome hook command for the
/// given args suffix (launcher-tolerant per #337 Phase B).
fn entry_cmd_matches(entry: &serde_json::Value, suffix: &str) -> bool {
    entry["command"]
        .as_str()
        .is_some_and(|c| tome::harness::launcher::looks_like_tome_hook_command(c, suffix))
}

/// US7 coexistence: when a plugin ships a `SessionStart` hook, BOTH the plugin
/// run-hook dispatch entry AND the session-steering entry land under
/// `hooks.sessionStart`. After unenrolling the plugin, ONLY the session-steering
/// entry survives (selective removal — neither reconciler clobbers the other).
#[test]
fn cursor_session_start_coexistence_and_selective_removal() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard = HarnessModulesGuard::install(vec![Box::new(tome::harness::cursor::Cursor)]);

    let fx = build();
    let (ws_id, skill_id) = seed_session_start_plugin(&fx);
    sync::sync_project(&fx.project, &fx.deps()).expect("initial sync");

    // ----- Step 1: both entries coexist under hooks.sessionStart -----
    let hook_path = fx.project.join(".cursor/hooks.json");
    let doc: serde_json::Value = serde_json::from_str(&read(&hook_path)).unwrap();
    assert_eq!(doc["version"], 1, "hook file must be version-stamped");

    let arr = doc["hooks"]["sessionStart"]
        .as_array()
        .expect("hooks.sessionStart must be an array");
    assert_eq!(
        arr.len(),
        2,
        "both the session-steering entry AND the run-hook dispatch entry must be present; \
         got: {}",
        serde_json::to_string_pretty(&doc["hooks"]["sessionStart"]).unwrap(),
    );

    // (a) session-steering entry: runs `tome harness session-start …`
    let has_steering = arr
        .iter()
        .any(|e| entry_cmd_matches(e, SESSION_STEERING_SUFFIX));
    assert!(
        has_steering,
        "session-steering entry (session-start command) must be present; \
         sessionStart array: {}",
        serde_json::to_string_pretty(&doc["hooks"]["sessionStart"]).unwrap(),
    );

    // (b) run-hook dispatch entry: runs `tome harness run-hook --event SessionStart …`
    let has_run_hook = arr
        .iter()
        .any(|e| entry_cmd_matches(e, SESSION_START_RUN_HOOK_SUFFIX));
    assert!(
        has_run_hook,
        "run-hook dispatch entry (run-hook command) must be present; \
         sessionStart array: {}",
        serde_json::to_string_pretty(&doc["hooks"]["sessionStart"]).unwrap(),
    );

    // No stale PascalCase key.
    assert!(
        doc["hooks"].get("SessionStart").is_none(),
        "cursor hook file must NOT use PascalCase SessionStart (camelCase only)",
    );

    // ----- Step 2: unenrol the plugin → selective removal -----
    unenroll_session_start_plugin(&fx, ws_id, skill_id);
    sync::sync_project(&fx.project, &fx.deps()).expect("post-unenrol sync");

    let doc2: serde_json::Value = serde_json::from_str(&read(&hook_path)).unwrap();
    let arr2 = doc2["hooks"]["sessionStart"]
        .as_array()
        .expect("hooks.sessionStart must still be an array after removal");
    assert_eq!(
        arr2.len(),
        1,
        "after plugin unenrol only the session-steering entry should remain; \
         got: {}",
        serde_json::to_string_pretty(&doc2["hooks"]["sessionStart"]).unwrap(),
    );

    // The surviving entry is the session-steering one.
    assert!(
        entry_cmd_matches(&arr2[0], SESSION_STEERING_SUFFIX),
        "surviving entry must be the session-steering command; got: {}",
        serde_json::to_string_pretty(&arr2[0]).unwrap(),
    );

    // The run-hook dispatch entry has been removed.
    let still_has_run_hook = arr2
        .iter()
        .any(|e| entry_cmd_matches(e, SESSION_START_RUN_HOOK_SUFFIX));
    assert!(
        !still_has_run_hook,
        "run-hook dispatch entry must be removed after plugin unenrol",
    );
}

//! US3.3 byte-stable registration + manifest pins for `copilot-cli`.
//!
//! Drives the REAL `sync_project` over the `copilot-cli` module with ONE enabled
//! plugin shipping a `PreToolUse` Bash command hook (a token-free command, so
//! the rewritten manifest handler is deterministic), then pins:
//!
//! 1. the exact on-disk `run-hook` dispatcher entry — registered under the
//!    harness-NATIVE event key (`PreToolUse`) under the `hooks` container; and
//! 2. the exact resolved manifest JSON — keyed by the CC event name, with the
//!    per-plugin matcher carried verbatim and `timeout_ms` baked from CC seconds.
//!
//! The literals were captured from the implementation output once and pinned
//! (byte-stable). The Tome `run-hook` entry is a SEPARATE additive leaf from any
//! session-steering entry on the same file, so it never clobbers it.

use std::path::{Path, PathBuf};

use crate::common::{HarnessModulesGuard, ToolEnv, paths_for, seed_workspace};
use tempfile::TempDir;
use tome::harness::sync::{self, SyncDeps};
use tome::workspace::WorkspaceName;

/// The exact `run-hook` event-array under the native event key (pretty-printed).
const HOOK_ARRAY: &str = r##"[
  {
    "type": "command",
    "command": "tome harness run-hook --event PreToolUse --harness copilot-cli --workspace test-workspace"
  }
]"##;

/// The exact resolved manifest bytes (pretty-printed + trailing newline).
const MANIFEST: &str = r##"{
  "harness": "copilot-cli",
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
        "workspace = \"test-workspace\"\nharnesses = [\"copilot-cli\"]\n",
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
fn copilot_cli_run_hook_registration_and_manifest_pins() {
    let _lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _guard =
        HarnessModulesGuard::install(vec![Box::new(tome::harness::copilot_cli::CopilotCli)]);

    let fx = build();
    seed_plugin(&fx);
    sync::sync_project(&fx.project, &fx.deps()).expect("sync");

    // ----- (a) the on-disk run-hook registration entry -----
    let hook_path = fx.project.join(".github/hooks/tome.json");
    let mut doc: serde_json::Value = serde_json::from_str(&read(&hook_path)).unwrap();
    // #337 Phase B: canonicalise the resolved launcher prefix to bare `tome`.
    crate::common::canonicalize_tome_hook_command_leaves(
        &mut doc,
        &["harness run-hook --event PreToolUse --harness copilot-cli --workspace test-workspace"],
    );
    assert_eq!(doc["version"], 1, "the hook file is version-stamped");
    let arr = &doc["hooks"]["PreToolUse"];
    assert_eq!(
        serde_json::to_string_pretty(arr).unwrap(),
        HOOK_ARRAY,
        "run-hook entry bytes drifted for copilot-cli",
    );

    // Non-leak: the plugin's verbatim command must NOT appear in the hook file.
    let raw = read(&hook_path);
    assert!(
        !raw.contains("/opt/guard.sh check"),
        "plugin command must not leak into hook file:\n{raw}",
    );

    // Composition: the session-steering `SessionStart` entry coexists (additive).
    assert!(
        doc["hooks"]["SessionStart"]
            .as_array()
            .is_some_and(|a| !a.is_empty()),
        "the session-steering entry must coexist with the run-hook entry",
    );

    // ----- (b) the resolved manifest -----
    let manifest_path = fx.paths.hooks_manifest(&fx.workspace, "copilot-cli");
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
    assert_eq!(got_rest, MANIFEST, "manifest bytes drifted for copilot-cli");
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

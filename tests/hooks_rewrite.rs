//! T069 — the two-variable hooks rewrite (Phase 6 / US2, FR-003).
//!
//! Exercises `harness::hooks::read_rewritten_entries` library-API style:
//! `${CLAUDE_PLUGIN_ROOT}` / `${CLAUDE_PLUGIN_DATA}` resolve to absolute
//! paths; `${CLAUDE_PROJECT_DIR}` / `${CLAUDE_SESSION_ID}` are left verbatim;
//! only string VALUES are rewritten (keys untouched).
//!
//! Contract: `contracts/hooks-integration.md` § "Path-variable rewriting".

use std::path::Path;

use serde_json::Value as JsonValue;
use tempfile::TempDir;
use tome::harness::hooks::{self, RewrittenHooks};

/// Write `<plugin_root>/hooks/hooks.json` with `body` and return the plugin
/// root path.
fn seed_hooks(plugin_root: &Path, body: &str) {
    let dir = plugin_root.join("hooks");
    std::fs::create_dir_all(&dir).expect("create hooks dir");
    std::fs::write(dir.join("hooks.json"), body).expect("write hooks.json");
}

/// Pull the single command string out of the first entry under `event`.
fn first_command<'a>(hooks: &'a RewrittenHooks, event: &str) -> &'a str {
    let (_, entries) = hooks
        .events
        .iter()
        .find(|(e, _)| e == event)
        .expect("event present");
    entries[0]["hooks"][0]["command"]
        .as_str()
        .expect("command string")
}

#[test]
fn resolves_plugin_root_and_data_leaves_others_verbatim() {
    let tmp = TempDir::new().unwrap();
    let plugin_root = tmp.path().join("install/midnight-expert");
    let plugin_data = tmp.path().join("data/cat/midnight-expert");

    seed_hooks(
        &plugin_root,
        r#"{
          "PreToolUse": [
            {
              "matcher": "Bash",
              "hooks": [
                { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/scripts/guard.sh --root ${CLAUDE_PROJECT_DIR} --data ${CLAUDE_PLUGIN_DATA} --sess ${CLAUDE_SESSION_ID}" }
              ]
            }
          ]
        }"#,
    );

    let rewritten = hooks::read_rewritten_entries(&plugin_root, &plugin_data)
        .expect("read ok")
        .expect("hooks present");

    let cmd = first_command(&rewritten, "PreToolUse");
    let root = plugin_root.to_string_lossy();
    let data = plugin_data.to_string_lossy();

    assert!(
        cmd.contains(&format!("{root}/scripts/guard.sh")),
        "PLUGIN_ROOT resolved to an absolute path: {cmd}"
    );
    assert!(
        cmd.contains(&format!("--data {data}")),
        "PLUGIN_DATA resolved to an absolute path: {cmd}"
    );
    // The other two tokens survive verbatim — Claude Code resolves them at
    // runtime.
    assert!(
        cmd.contains("--root ${CLAUDE_PROJECT_DIR}"),
        "PROJECT_DIR left verbatim: {cmd}"
    );
    assert!(
        cmd.contains("--sess ${CLAUDE_SESSION_ID}"),
        "SESSION_ID left verbatim: {cmd}"
    );
    // Neither rewritten token survives.
    assert!(!cmd.contains("${CLAUDE_PLUGIN_ROOT}"));
    assert!(!cmd.contains("${CLAUDE_PLUGIN_DATA}"));
}

#[test]
fn only_string_values_rewritten_keys_untouched() {
    let tmp = TempDir::new().unwrap();
    let plugin_root = tmp.path().join("install/p");
    let plugin_data = tmp.path().join("data/p");

    // A key that spells a token must NOT be rewritten; a non-string scalar
    // value (number) must survive unchanged.
    seed_hooks(
        &plugin_root,
        r#"{
          "PreToolUse": [
            {
              "${CLAUDE_PLUGIN_ROOT}": "literal-key",
              "timeout": 30,
              "hooks": [ { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/x" } ]
            }
          ]
        }"#,
    );

    let rewritten = hooks::read_rewritten_entries(&plugin_root, &plugin_data)
        .expect("read ok")
        .expect("hooks present");
    let (_, entries) = &rewritten.events[0];
    let entry = &entries[0];

    // The token-looking KEY is preserved verbatim.
    assert!(
        entry
            .as_object()
            .unwrap()
            .contains_key("${CLAUDE_PLUGIN_ROOT}"),
        "key must stay verbatim: {entry}"
    );
    // The numeric scalar is untouched.
    assert_eq!(entry["timeout"], JsonValue::from(30));
    // The string VALUE is rewritten.
    let cmd = entry["hooks"][0]["command"].as_str().unwrap();
    assert!(cmd.starts_with(&*plugin_root.to_string_lossy()));
}

#[test]
fn absent_hooks_file_is_none() {
    let tmp = TempDir::new().unwrap();
    let plugin_root = tmp.path().join("install/empty-plugin");
    std::fs::create_dir_all(&plugin_root).unwrap();
    let plugin_data = tmp.path().join("data/empty-plugin");

    let result = hooks::read_rewritten_entries(&plugin_root, &plugin_data).expect("read ok");
    assert!(result.is_none(), "a plugin with no hooks.json yields None");
}

// ---------------------------------------------------------------------------
// T2-6: a symlinked hook SOURCE (`hooks/hooks.json`) is refused → exit 7,
//       mirroring the settings-write symlink refusal.
// ---------------------------------------------------------------------------

#[test]
#[cfg(unix)]
fn symlinked_hook_source_is_refused_exit_7() {
    let tmp = TempDir::new().unwrap();
    let plugin_root = tmp.path().join("install/linked");
    let plugin_data = tmp.path().join("data/linked");

    // Plant a real hooks.json elsewhere, then symlink the source path to it.
    let decoy = tmp.path().join("decoy-hooks.json");
    std::fs::write(
        &decoy,
        r#"{ "PreToolUse": [ { "matcher": "Bash", "hooks": [] } ] }"#,
    )
    .unwrap();
    let hooks_dir = plugin_root.join("hooks");
    std::fs::create_dir_all(&hooks_dir).unwrap();
    std::os::unix::fs::symlink(&decoy, hooks_dir.join("hooks.json")).expect("plant symlink");

    let err = hooks::read_rewritten_entries(&plugin_root, &plugin_data)
        .expect_err("a symlinked source must be refused");
    assert_eq!(
        err.exit_code(),
        7,
        "symlinked source refusal → exit 7; got {err:?}"
    );
}

#[test]
fn malformed_hooks_file_is_exit_43() {
    let tmp = TempDir::new().unwrap();
    let plugin_root = tmp.path().join("install/broken");
    let plugin_data = tmp.path().join("data/broken");
    seed_hooks(&plugin_root, "{ this is not valid json");

    let err = hooks::read_rewritten_entries(&plugin_root, &plugin_data)
        .expect_err("malformed hooks.json must fail");
    assert_eq!(
        err.exit_code(),
        43,
        "malformed hooks → exit 43; got {err:?}"
    );
    match &err {
        tome::error::TomeError::HookSpecParseError { path } => {
            assert!(
                path.ends_with("hooks/hooks.json"),
                "error names the source file: {path:?}"
            );
        }
        other => panic!("expected HookSpecParseError, got {other:?}"),
    }
}

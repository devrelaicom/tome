//! T070 — structural-match merge / removal / pruning of real hooks into
//! `.claude/settings.local.json` (Phase 6 / US2, FR-002/004/005/006).
//!
//! Exercises `harness::hooks::{merge_into_settings, remove_from_settings}`
//! library-API style: add, idempotent re-add, user-authored dedup,
//! user-edit preservation on removal, create-if-absent (`settings.json`
//! never touched), prune-empty-event, and mtime-capture idempotence.
//!
//! Contract: `contracts/hooks-integration.md` § "Merge semantics" /
//! "Removal semantics" / "Post-removal pruning".

use std::path::Path;
use std::time::{Duration, SystemTime};

use serde_json::{Value as JsonValue, json};
use tempfile::TempDir;
use tome::harness::hooks::{self, RewrittenHooks};

/// Build a single-event `RewrittenHooks` carrying one entry under `event`.
fn one(event: &str, entry: JsonValue) -> RewrittenHooks {
    RewrittenHooks {
        events: vec![(event.to_string(), vec![entry])],
    }
}

fn entry(cmd: &str) -> JsonValue {
    json!({ "matcher": "Bash", "hooks": [ { "type": "command", "command": cmd } ] })
}

fn read(path: &Path) -> JsonValue {
    serde_json::from_str(&std::fs::read_to_string(path).expect("read settings")).expect("parse")
}

fn mtime(path: &Path) -> SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap()
}

#[test]
fn create_if_absent_never_touches_committed_settings() {
    let tmp = TempDir::new().unwrap();
    let claude = tmp.path().join(".claude");
    let local = claude.join("settings.local.json");
    let committed = claude.join("settings.json");

    let hooks = one("PreToolUse", entry("/abs/g.sh"));
    let changed = hooks::merge_into_settings(&local, &hooks).expect("merge");
    assert!(changed, "first merge creates the file");

    assert!(local.is_file(), "settings.local.json created");
    assert!(
        !committed.exists(),
        "the committed settings.json must NEVER be written (SC-004)"
    );

    let doc = read(&local);
    assert_eq!(
        doc["hooks"]["PreToolUse"].as_array().unwrap().len(),
        1,
        "the rewritten entry is present under its event"
    );
}

#[test]
fn idempotent_re_add_is_deep_equal_skip_no_rewrite() {
    let tmp = TempDir::new().unwrap();
    let local = tmp.path().join(".claude/settings.local.json");
    let hooks = one("PreToolUse", entry("/abs/g.sh"));

    assert!(hooks::merge_into_settings(&local, &hooks).expect("merge 1"));
    let mtime_1 = mtime(&local);

    std::thread::sleep(Duration::from_millis(1100));

    // Second merge: deep-equal entry already present → skip, no rewrite.
    let changed = hooks::merge_into_settings(&local, &hooks).expect("merge 2");
    assert!(!changed, "deep-equal re-add must be a no-op");
    assert_eq!(mtime(&local), mtime_1, "mtime must not advance on no-op");
    assert_eq!(
        read(&local)["hooks"]["PreToolUse"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "no duplicate appended"
    );
}

#[test]
fn user_authored_identical_entry_not_duplicated() {
    let tmp = TempDir::new().unwrap();
    let claude = tmp.path().join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    let local = claude.join("settings.local.json");

    // The user hand-authored an entry that is deep-equal to Tome's.
    let pre = json!({ "hooks": { "PreToolUse": [ entry("/abs/g.sh") ] } });
    std::fs::write(&local, serde_json::to_string_pretty(&pre).unwrap()).unwrap();

    let changed =
        hooks::merge_into_settings(&local, &one("PreToolUse", entry("/abs/g.sh"))).expect("merge");
    assert!(
        !changed,
        "a deep-equal hand-authored entry counts as present"
    );
    assert_eq!(
        read(&local)["hooks"]["PreToolUse"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "the user's entry is not duplicated (FR-004, SC-005)"
    );
}

#[test]
fn user_edited_entry_preserved_on_removal() {
    let tmp = TempDir::new().unwrap();
    let local = tmp.path().join(".claude/settings.local.json");

    // Tome merged an entry, then the user hand-edited it (added a flag).
    hooks::merge_into_settings(&local, &one("PreToolUse", entry("/abs/g.sh"))).expect("merge");
    let edited = json!({ "hooks": { "PreToolUse": [ entry("/abs/g.sh --user-flag") ] } });
    std::fs::write(&local, serde_json::to_string_pretty(&edited).unwrap()).unwrap();

    // Disable: Tome re-derives its original entry; the edited one no longer
    // matches → left in place (NFR-003, SC-005).
    let changed = hooks::remove_from_settings(&local, &one("PreToolUse", entry("/abs/g.sh")))
        .expect("remove");
    assert!(
        !changed,
        "a non-matching (user-edited) entry is not removed"
    );
    let arr = read(&local)["hooks"]["PreToolUse"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(arr.len(), 1, "the user-edited entry survives");
    assert!(
        arr[0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("--user-flag"),
        "the user's edit is preserved verbatim"
    );
}

#[test]
fn removal_prunes_empty_event_but_keeps_hooks_object() {
    let tmp = TempDir::new().unwrap();
    let local = tmp.path().join(".claude/settings.local.json");

    let hooks = one("PreToolUse", entry("/abs/g.sh"));
    hooks::merge_into_settings(&local, &hooks).expect("merge");

    let changed = hooks::remove_from_settings(&local, &hooks).expect("remove");
    assert!(changed, "the owned entry is removed");

    let doc = read(&local);
    // The now-empty event array is pruned (FR-006) …
    assert!(
        doc["hooks"]
            .as_object()
            .unwrap()
            .get("PreToolUse")
            .is_none(),
        "empty event array pruned: {doc}"
    );
    // … but the otherwise-empty `hooks` object and the file are left in place.
    assert!(
        doc.as_object().unwrap().contains_key("hooks"),
        "the hooks object is left in place even when empty (FR-006)"
    );
    assert!(local.is_file(), "the settings file itself is not deleted");
}

#[test]
fn removal_against_absent_file_is_noop() {
    let tmp = TempDir::new().unwrap();
    let local = tmp.path().join(".claude/settings.local.json");
    let changed = hooks::remove_from_settings(&local, &one("PreToolUse", entry("/abs/g.sh")))
        .expect("remove on absent");
    assert!(!changed, "removing from an absent file changes nothing");
    assert!(!local.exists(), "no file is created on removal");
}

#[test]
fn merge_preserves_unrelated_user_keys_and_appends() {
    let tmp = TempDir::new().unwrap();
    let claude = tmp.path().join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    let local = claude.join("settings.local.json");

    // Pre-existing user content: an unrelated top-level key + a different
    // hook the user authored under the same event.
    let pre = json!({
        "model": "opus",
        "hooks": { "PreToolUse": [ entry("/users/own-hook.sh") ] }
    });
    std::fs::write(&local, serde_json::to_string_pretty(&pre).unwrap()).unwrap();

    let changed =
        hooks::merge_into_settings(&local, &one("PreToolUse", entry("/abs/g.sh"))).expect("merge");
    assert!(changed, "a distinct entry is appended");

    let doc = read(&local);
    assert_eq!(doc["model"], "opus", "unrelated user keys are preserved");
    assert_eq!(
        doc["hooks"]["PreToolUse"].as_array().unwrap().len(),
        2,
        "Tome's entry appended alongside the user's"
    );
}

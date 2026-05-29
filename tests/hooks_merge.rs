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

// ---------------------------------------------------------------------------
// T2-3: two plugins' rewritten entries merge into one file in a single pass;
//       a re-merge of the same set is a mtime no-op.
// ---------------------------------------------------------------------------

#[test]
fn multi_plugin_merge_in_one_pass_then_idempotent() {
    let tmp = TempDir::new().unwrap();
    let local = tmp.path().join(".claude/settings.local.json");

    // Plugin A contributes two PreToolUse entries; plugin B one Stop entry.
    let plugin_a = RewrittenHooks {
        events: vec![(
            "PreToolUse".to_string(),
            vec![entry("/a/one.sh"), entry("/a/two.sh")],
        )],
    };
    let plugin_b = one("Stop", entry("/b/stop.sh"));

    assert!(hooks::merge_into_settings(&local, &plugin_a).expect("merge a"));
    assert!(hooks::merge_into_settings(&local, &plugin_b).expect("merge b"));

    let doc = read(&local);
    assert_eq!(
        doc["hooks"]["PreToolUse"].as_array().unwrap().len(),
        2,
        "both of plugin A's entries land under PreToolUse: {doc}"
    );
    assert_eq!(
        doc["hooks"]["Stop"].as_array().unwrap().len(),
        1,
        "plugin B's entry lands under its own event: {doc}"
    );

    // Re-merge the same set: deep-equal everywhere → no rewrite, mtime stable.
    let m1 = mtime(&local);
    std::thread::sleep(Duration::from_millis(1100));
    assert!(!hooks::merge_into_settings(&local, &plugin_a).expect("re-merge a"));
    assert!(!hooks::merge_into_settings(&local, &plugin_b).expect("re-merge b"));
    assert_eq!(mtime(&local), m1, "re-merge of the same set is a no-op");
}

// ---------------------------------------------------------------------------
// T2-5: multi-event partial prune — Tome's entry under event A, a user entry
//       under event B; removing event A prunes it while event B + its user
//       entry survive.
// ---------------------------------------------------------------------------

#[test]
fn multi_event_removal_prunes_only_the_target_event() {
    let tmp = TempDir::new().unwrap();
    let claude = tmp.path().join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    let local = claude.join("settings.local.json");

    // Seed Tome's entry under event A (PreToolUse) and a USER-authored entry
    // under event B (Stop) that Tome does not own.
    let pre = json!({
        "hooks": {
            "PreToolUse": [ entry("/abs/g.sh") ],
            "Stop": [ entry("/users/stop-hook.sh") ]
        }
    });
    std::fs::write(&local, serde_json::to_string_pretty(&pre).unwrap()).unwrap();

    // Remove only the event-A entry.
    let changed = hooks::remove_from_settings(&local, &one("PreToolUse", entry("/abs/g.sh")))
        .expect("remove");
    assert!(changed, "the event-A entry is removed");

    let doc = read(&local);
    assert!(
        doc["hooks"]
            .as_object()
            .unwrap()
            .get("PreToolUse")
            .is_none(),
        "event A is pruned once empty: {doc}"
    );
    assert_eq!(
        doc["hooks"]["Stop"].as_array().unwrap().len(),
        1,
        "event B survives untouched: {doc}"
    );
    assert!(
        doc["hooks"]["Stop"][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .contains("stop-hook.sh"),
        "the user's event-B entry is preserved verbatim: {doc}"
    );
}

// ---------------------------------------------------------------------------
// T2-4: a malformed / wrong-type existing settings.local.json → exit 44, and
//       the original file is left byte-for-byte intact.
// ---------------------------------------------------------------------------

#[test]
fn merge_into_wrong_type_settings_is_exit_44_original_intact() {
    let tmp = TempDir::new().unwrap();
    let claude = tmp.path().join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    let local = claude.join("settings.local.json");

    // Top-level value is a JSON array, not an object → load_settings rejects it.
    let original = "[not an object]";
    std::fs::write(&local, original).unwrap();

    let err = hooks::merge_into_settings(&local, &one("PreToolUse", entry("/abs/g.sh")))
        .expect_err("wrong-type settings must fail");
    assert_eq!(err.exit_code(), 44, "wrong-type → exit 44; got {err:?}");
    match &err {
        tome::error::TomeError::HookSettingsWriteFailed { path, .. } => {
            assert!(
                path.ends_with("settings.local.json"),
                "error names the settings file: {path:?}"
            );
        }
        other => panic!("expected HookSettingsWriteFailed, got {other:?}"),
    }

    // The original file is left byte-for-byte intact.
    assert_eq!(
        std::fs::read_to_string(&local).unwrap(),
        original,
        "the malformed original must not be rewritten"
    );
}

#[test]
fn merge_into_wrong_type_hooks_value_is_exit_44_original_intact() {
    let tmp = TempDir::new().unwrap();
    let claude = tmp.path().join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    let local = claude.join("settings.local.json");

    // `hooks` is an array, not an object → ensure_hooks_object rejects it.
    let original = "{\"hooks\": []}";
    std::fs::write(&local, original).unwrap();

    let err = hooks::merge_into_settings(&local, &one("PreToolUse", entry("/abs/g.sh")))
        .expect_err("wrong-type hooks value must fail");
    assert_eq!(
        err.exit_code(),
        44,
        "wrong-type hooks → exit 44; got {err:?}"
    );

    assert_eq!(
        std::fs::read_to_string(&local).unwrap(),
        original,
        "the original must be left byte-for-byte intact"
    );
}

// ---------------------------------------------------------------------------
// T2-1: symlink refusal on the settings.local.json write → exit 44, for BOTH
//       the merge and remove paths. settings.local.json is a dedicated Phase 6
//       sink, so a symlinked target surfaces HookSettingsWriteFailed (exit 44),
//       not the generic Io (exit 7) used for the hooks.json SOURCE read. The
//       decoy the symlink points at is left intact and the target stays a
//       symlink (not replaced).
// ---------------------------------------------------------------------------

#[cfg(unix)]
fn assert_symlink_refused(op: impl FnOnce(&Path) -> Result<bool, tome::error::TomeError>) {
    let tmp = TempDir::new().unwrap();
    let claude = tmp.path().join(".claude");
    std::fs::create_dir_all(&claude).unwrap();

    let decoy = tmp.path().join("decoy.json");
    std::fs::write(&decoy, "ORIGINAL DECOY CONTENT\n").unwrap();

    let local = claude.join("settings.local.json");
    std::os::unix::fs::symlink(&decoy, &local).expect("plant symlink");

    let err = op(&local).expect_err("symlink target must be refused");
    assert_eq!(
        err.exit_code(),
        44,
        "symlink refusal → exit 44; got {err:?}"
    );
    assert!(
        matches!(&err, tome::error::TomeError::HookSettingsWriteFailed { .. }),
        "the settings.local.json sink uses HookSettingsWriteFailed; got {err:?}"
    );

    // The decoy is untouched and the target is still a symlink.
    assert_eq!(
        std::fs::read_to_string(&decoy).unwrap(),
        "ORIGINAL DECOY CONTENT\n",
        "the symlink target must NOT be overwritten"
    );
    let meta = std::fs::symlink_metadata(&local).unwrap();
    assert!(
        meta.file_type().is_symlink(),
        "the settings target must remain the planted symlink"
    );
}

#[test]
#[cfg(unix)]
fn merge_through_symlinked_settings_is_refused_exit_44() {
    assert_symlink_refused(|local| {
        hooks::merge_into_settings(local, &one("PreToolUse", entry("/abs/g.sh")))
    });
}

#[test]
#[cfg(unix)]
fn remove_through_symlinked_settings_is_refused_exit_44() {
    assert_symlink_refused(|local| {
        hooks::remove_from_settings(local, &one("PreToolUse", entry("/abs/g.sh")))
    });
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

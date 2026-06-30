//! US9 — CC `if` arg-aware predicate dispatch filter.
//!
//! A manifest entry with an `if` permission-rule predicate (e.g.
//! `Bash(git push *)`) fires ONLY when the synthesised `tool_input` field
//! matches the glob pattern. Drives [`dispatch_core`] with matching and
//! non-matching tool_input to verify the evaluator is applied in the filter.

use tome::commands::harness::run_hook;
use tome::harness::hooks_ir::HookManifest;

fn manifest(json: &str) -> HookManifest {
    serde_json::from_str(json).expect("parse manifest JSON")
}

/// A hook with `"if": "Bash(git push *)"` fires on a matching Bash command
/// (deny) but is skipped on a non-matching command (fail-open allow).
#[test]
fn if_pred_bash_glob_fires_on_match_only() {
    let m = manifest(
        r#"{
            "harness": "cursor",
            "events": {
                "PreToolUse": [
                    { "plugin": "cat:guard",
                      "matcher": "Bash",
                      "if": "Bash(git push *)",
                      "handler": { "type": "command", "command": "printf 'push-detected' >&2; exit 2" } }
                ]
            }
        }"#,
    );

    // Matching tool_input: command starts with "git push " → hook fires → deny.
    let matching = r#"{"tool_name":"Bash","tool_input":{"command":"git push origin main"}}"#;
    let out = run_hook::dispatch_core("cursor", "PreToolUse", matching, Some(&m));
    assert_eq!(out.exit_code, 0, "Cursor never exits 2 (blocks via JSON)");
    assert!(
        out.stdout.contains("\"permission\":\"deny\""),
        "matching if_pred must fire the hook and deny; got: {}",
        out.stdout,
    );
    assert!(
        out.stdout.contains("push-detected"),
        "deny reason must contain the hook's stderr; got: {}",
        out.stdout,
    );

    // Non-matching tool_input: command is "git pull …" → hook skipped → allow.
    let non_matching = r#"{"tool_name":"Bash","tool_input":{"command":"git pull origin main"}}"#;
    let out = run_hook::dispatch_core("cursor", "PreToolUse", non_matching, Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.is_empty(),
        "non-matching if_pred must skip the hook and allow (empty); got: {}",
        out.stdout,
    );
}

/// A hook with no `if` predicate fires regardless of the tool_input content
/// (baseline / regression: existing unconditional behaviour is preserved).
#[test]
fn no_if_pred_fires_unconditionally() {
    let m = manifest(
        r#"{
            "harness": "cursor",
            "events": {
                "PreToolUse": [
                    { "plugin": "cat:guard",
                      "matcher": "Bash",
                      "handler": { "type": "command", "command": "printf 'blocked' >&2; exit 2" } }
                ]
            }
        }"#,
    );
    // No if predicate: any Bash invocation fires the hook.
    let out = run_hook::dispatch_core(
        "cursor",
        "PreToolUse",
        r#"{"tool_name":"Bash","tool_input":{"command":"anything here"}}"#,
        Some(&m),
    );
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.contains("\"permission\":\"deny\""),
        "hook without if_pred must fire unconditionally; got: {}",
        out.stdout,
    );
}

/// A malformed `if` predicate (fail-open): the hook does NOT fire, and the
/// dispatcher falls through to an empty allow at exit 0.
#[test]
fn malformed_if_pred_fails_open() {
    let m = manifest(
        r#"{
            "harness": "cursor",
            "events": {
                "PreToolUse": [
                    { "plugin": "cat:guard",
                      "if": "((((malformed",
                      "handler": { "type": "command", "command": "exit 2" } }
                ]
            }
        }"#,
    );
    let out = run_hook::dispatch_core(
        "cursor",
        "PreToolUse",
        r#"{"tool_name":"Bash","tool_input":{"command":"anything"}}"#,
        Some(&m),
    );
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.is_empty(),
        "malformed if_pred must fail open (no deny, empty allow); got: {}",
        out.stdout,
    );
}

/// A hook with `"if": "Edit(/etc/*)"` fires only when tool_input.file_path
/// matches, verifying the Read/Edit/Write → file_path field mapping.
#[test]
fn if_pred_edit_file_path_glob() {
    let m = manifest(
        r#"{
            "harness": "cursor",
            "events": {
                "PreToolUse": [
                    { "plugin": "cat:guard",
                      "matcher": "Edit",
                      "if": "Edit(/etc/*)",
                      "handler": { "type": "command", "command": "printf 'etc-write' >&2; exit 2" } }
                ]
            }
        }"#,
    );

    // Matching file_path → hook fires.
    let matching = r#"{"tool_name":"Edit","tool_input":{"file_path":"/etc/hosts"}}"#;
    let out = run_hook::dispatch_core("cursor", "PreToolUse", matching, Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.contains("\"permission\":\"deny\""),
        "Edit /etc/hosts must match Edit(/etc/*) and deny; got: {}",
        out.stdout,
    );

    // Non-matching file_path → hook skipped.
    let non_matching = r#"{"tool_name":"Edit","tool_input":{"file_path":"/home/user/foo.rs"}}"#;
    let out = run_hook::dispatch_core("cursor", "PreToolUse", non_matching, Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.is_empty(),
        "Edit /home/user/foo.rs must not match Edit(/etc/*); got: {}",
        out.stdout,
    );
}

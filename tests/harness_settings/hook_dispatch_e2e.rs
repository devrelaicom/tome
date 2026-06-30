//! US4.6 — end-to-end dispatcher coverage + exhaustive fail-open.
//!
//! Drives the pure core [`dispatch_core`] with real command hooks (the same
//! pipeline `tome harness run-hook` runs), asserting the EXACT emitted wire
//! bytes + exit code for allow / deny / context-inject, then exhaustively
//! verifies that every Tome-side fault degrades to a fail-open allow + exit 0.

use tome::commands::harness::run_hook;
use tome::harness::hooks_ir::HookManifest;

/// One command hook (optional matcher) for the given harness + event.
fn manifest(harness: &str, event: &str, matcher: Option<&str>, command: &str) -> HookManifest {
    let command_json = serde_json::to_string(command).expect("escape command");
    let matcher_field = match matcher {
        Some(m) => format!(
            r#""matcher": {}, "#,
            serde_json::to_string(m).expect("escape matcher")
        ),
        None => String::new(),
    };
    let json = format!(
        r#"{{
            "harness": "{harness}",
            "events": {{
                "{event}": [
                    {{ "plugin": "cat:g", {matcher_field}"handler": {{ "type": "command", "command": {command_json} }} }}
                ]
            }}
        }}"#
    );
    serde_json::from_str(&json).expect("parse manifest JSON")
}

/// Full pipeline on Devin (ClaudeStyle), pinning EXACT wire bytes for allow,
/// deny, and context-inject.
#[test]
fn e2e_devin_allow_deny_context_exact_bytes() {
    // allow — empty no-op, exit 0.
    let m = manifest("devin", "PreToolUse", None, "exit 0");
    let out = run_hook::dispatch_core("devin", "PreToolUse", r#"{"tool_name":"exec"}"#, Some(&m));
    assert_eq!(out.exit_code, 0);
    assert_eq!(out.stdout, "");

    // deny — `exec` rewrites to CC `Bash`, matches the `Bash` matcher, runs the
    // exit-2 hook → top-level block JSON + exit 2 with provenance-prefixed reason.
    let m = manifest(
        "devin",
        "PreToolUse",
        Some("Bash"),
        "printf 'blocked' >&2; exit 2",
    );
    let out = run_hook::dispatch_core("devin", "PreToolUse", r#"{"tool_name":"exec"}"#, Some(&m));
    assert_eq!(out.exit_code, 2);
    assert_eq!(
        out.stdout,
        r#"{"decision":"block","reason":"[cat:g] blocked"}"#
    );

    // context-inject — a hook emitting hookSpecificOutput.additionalContext is
    // relayed nested with the hookEventName, at exit 0.
    let cmd = r#"printf '{"hookSpecificOutput":{"additionalContext":"note"}}'"#;
    let m = manifest("devin", "UserPromptSubmit", None, cmd);
    let out = run_hook::dispatch_core("devin", "UserPromptSubmit", "{}", Some(&m));
    assert_eq!(out.exit_code, 0);
    assert_eq!(
        out.stdout,
        r#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"note"}}"#
    );
}

/// Full pipeline on Cursor (CursorSnake): a deny is relayed as snake_case JSON
/// at exit 0 (Cursor blocks via JSON, never exit-2).
#[test]
fn e2e_cursor_deny_exact_bytes() {
    let m = manifest("cursor", "PreToolUse", None, "printf 'denied' >&2; exit 2");
    let out = run_hook::dispatch_core("cursor", "PreToolUse", r#"{"tool_name":"Read"}"#, Some(&m));
    assert_eq!(out.exit_code, 0);
    assert_eq!(
        out.stdout,
        r#"{"permission":"deny","agent_message":"[cat:g] denied"}"#
    );
}

/// FAIL-OPEN: a missing manifest (`None`) is an allow + exit 0 on every harness.
#[test]
fn fail_open_missing_manifest_every_harness() {
    for h in ["devin", "gemini", "codex", "cursor", "copilot-cli"] {
        let out = run_hook::dispatch_core(h, "PreToolUse", r#"{"tool_name":"Bash"}"#, None);
        assert_eq!(out.exit_code, 0, "{h} missing-manifest exit");
        assert!(out.stdout.is_empty(), "{h} missing-manifest stdout");
    }
}

/// FAIL-OPEN: an unparsable stdin must not fault or block — the allow hook still
/// yields an empty allow at exit 0.
#[test]
fn fail_open_unparsable_stdin() {
    let m = manifest("cursor", "PreToolUse", None, "exit 0");
    let out = run_hook::dispatch_core("cursor", "PreToolUse", "this is not json {{{", Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.is_empty());
}

/// FAIL-OPEN: a command that crashes (signal-killed, or a non-2 non-0 exit)
/// degrades to a non-blocking allow + exit 0 — NEVER a block.
#[test]
fn fail_open_command_crash() {
    // Signal-killed: status.code() is None → treated as exit 0 → allow.
    let m = manifest("cursor", "PreToolUse", None, "kill -9 $$");
    let out = run_hook::dispatch_core("cursor", "PreToolUse", "{}", Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.is_empty(),
        "signal crash must allow, got: {}",
        out.stdout
    );

    // A non-2 non-0 exit (e.g. 3) is also a non-blocking allow.
    let m = manifest("cursor", "PreToolUse", None, "exit 3");
    let out = run_hook::dispatch_core("cursor", "PreToolUse", "{}", Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.is_empty(),
        "exit 3 must allow, got: {}",
        out.stdout
    );
}

/// FAIL-OPEN: a hook that hangs is killed at its `timeout_ms` and degrades to a
/// non-blocking allow + exit 0 — a Tome timeout is NEVER a block.
#[test]
fn fail_open_handler_timeout() {
    let json = r#"{
        "harness": "cursor",
        "events": {
            "PreToolUse": [
                { "plugin": "cat:g", "timeout_ms": 50,
                  "handler": { "type": "command", "command": "sleep 5" } }
            ]
        }
    }"#;
    let m: HookManifest = serde_json::from_str(json).expect("parse manifest");
    let out = run_hook::dispatch_core("cursor", "PreToolUse", "{}", Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.is_empty(),
        "timeout must allow, got: {}",
        out.stdout
    );
}

/// FAIL-OPEN: an `http` handler (US5, not yet executed) in the manifest is a
/// non-blocking allow placeholder — the dispatch loop never crashes on it.
#[test]
fn fail_open_unsupported_handler_kind() {
    let json = r#"{
        "harness": "devin",
        "events": {
            "PreToolUse": [
                { "plugin": "cat:g",
                  "handler": { "type": "prompt", "prompt": "is this safe?" } }
            ]
        }
    }"#;
    let m: HookManifest = serde_json::from_str(json).expect("parse manifest");
    let out = run_hook::dispatch_core("devin", "PreToolUse", r#"{"tool_name":"exec"}"#, Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.is_empty(),
        "prompt handler must be a no-op allow, got: {}",
        out.stdout
    );
}

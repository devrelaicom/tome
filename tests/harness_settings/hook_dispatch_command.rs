//! US4.3 — command-handler execution through the full dispatch pipeline.
//!
//! Drives the pure core [`dispatch_core`] with a JSON-deserialized manifest (the
//! same shape US3 writes to disk), exercising matcher filter → command exec →
//! decision parse → merge → wire emit end-to-end. Cursor is the wire under test
//! (snake_case, exit 0 always) so the block path is unambiguous.

use tome::commands::harness::run_hook;
use tome::harness::hooks_ir::HookManifest;

fn manifest(json: &str) -> HookManifest {
    serde_json::from_str(json).expect("parse manifest JSON")
}

/// A `PreToolUse` command hook that exits 2 with a stderr reason denies through
/// the pipeline, emitted as Cursor's snake_case `permission:"deny"` at exit 0.
#[test]
fn cursor_command_exit2_denies_through_pipeline() {
    let m = manifest(
        r#"{
            "harness": "cursor",
            "events": {
                "PreToolUse": [
                    { "plugin": "cat:guard", "matcher": "Bash",
                      "handler": { "type": "command", "command": "printf 'nope' >&2; exit 2" } }
                ]
            }
        }"#,
    );
    let out = run_hook::dispatch_core("cursor", "PreToolUse", r#"{"tool_name":"Bash"}"#, Some(&m));
    // Cursor blocks via JSON, NEVER exit-2 from Tome.
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.contains("\"permission\":\"deny\""),
        "expected a cursor deny, got: {}",
        out.stdout,
    );
    assert!(
        out.stdout.contains("\"agent_message\""),
        "expected the reason on Cursor's agent_message channel, got: {}",
        out.stdout,
    );
    assert!(
        out.stdout.contains("nope"),
        "expected the stderr reason text, got: {}",
        out.stdout,
    );
}

/// When the incoming CC tool name does not match the entry's matcher, the hook
/// never runs and the dispatcher fails open (empty allow, exit 0).
#[test]
fn matcher_miss_runs_nothing_and_allows() {
    let m = manifest(
        r#"{
            "harness": "cursor",
            "events": {
                "PreToolUse": [
                    { "plugin": "cat:guard", "matcher": "Edit",
                      "handler": { "type": "command", "command": "exit 2" } }
                ]
            }
        }"#,
    );
    // tool_name Bash ≠ the `Edit` matcher → the deny command is never spawned.
    let out = run_hook::dispatch_core("cursor", "PreToolUse", r#"{"tool_name":"Bash"}"#, Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.is_empty(),
        "expected empty allow, got: {}",
        out.stdout
    );
}

/// An event present in the manifest but with an `http` handler (US5) is a
/// non-blocking allow placeholder — the dispatch loop must not crash on it.
#[test]
fn http_handler_is_non_blocking_placeholder() {
    let m = manifest(
        r#"{
            "harness": "cursor",
            "events": {
                "PreToolUse": [
                    { "plugin": "cat:webhook",
                      "handler": { "type": "http", "url": "https://example.test/hook" } }
                ]
            }
        }"#,
    );
    let out = run_hook::dispatch_core("cursor", "PreToolUse", r#"{"tool_name":"Bash"}"#, Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.is_empty(),
        "http handler must be a no-op allow, got: {}",
        out.stdout
    );
}

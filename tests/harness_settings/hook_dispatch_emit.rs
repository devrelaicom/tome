//! US4.5 — per-harness decision emit, driven end-to-end through
//! [`dispatch_core`] with a real command hook per wire.
//!
//! Each harness name resolves (via the registry) to its decision wire:
//! devin/gemini → ClaudeStyle, codex → Codex, cursor → CursorSnake, copilot-cli
//! → CopilotFlat. The assertions pin the exact emitted wire shape + exit code.

use tome::commands::harness::run_hook;
use tome::harness::hooks_ir::HookManifest;

/// Build a one-command-hook manifest (no matcher → fires on every tool) for the
/// given harness + event. `command` is JSON-escaped into the handler.
fn manifest_one_command(harness: &str, event: &str, command: &str) -> HookManifest {
    let command_json = serde_json::to_string(command).expect("escape command");
    let json = format!(
        r#"{{
            "harness": "{harness}",
            "events": {{
                "{event}": [
                    {{ "plugin": "cat:g", "handler": {{ "type": "command", "command": {command_json} }} }}
                ]
            }}
        }}"#
    );
    serde_json::from_str(&json).expect("parse manifest JSON")
}

/// Devin (ClaudeStyle): a deny emits `{"decision":"block",…}` AND exits 2 —
/// Devin blocks on exit-2.
#[test]
fn devin_claude_style_deny_exits_2() {
    let m = manifest_one_command("devin", "PreToolUse", "printf 'no' >&2; exit 2");
    let out = run_hook::dispatch_core("devin", "PreToolUse", "{}", Some(&m));
    assert_eq!(out.exit_code, 2);
    assert!(
        out.stdout.contains("\"decision\":\"block\""),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("[cat:g] no"), "{}", out.stdout);
}

/// Codex: the SAME block JSON, but at exit 0 (Codex's exit-2 semantics are
/// unverified, so Tome blocks via JSON only).
#[test]
fn codex_block_via_json_exits_0() {
    let m = manifest_one_command("codex", "PreToolUse", "printf 'no' >&2; exit 2");
    let out = run_hook::dispatch_core("codex", "PreToolUse", "{}", Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.contains("\"decision\":\"block\""),
        "{}",
        out.stdout
    );
}

/// Cursor (CursorSnake): deny is `{permission:"deny", agent_message}` at exit 0.
#[test]
fn cursor_deny_snake_case_exit_0() {
    let m = manifest_one_command("cursor", "PreToolUse", "printf 'no' >&2; exit 2");
    let out = run_hook::dispatch_core("cursor", "PreToolUse", "{}", Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.contains("\"permission\":\"deny\""),
        "{}",
        out.stdout
    );
    assert!(out.stdout.contains("\"agent_message\""), "{}", out.stdout);
}

/// Copilot CLI (CopilotFlat): deny is `{permissionDecision:"deny",
/// permissionDecisionReason}` at exit 0 (Copilot's exit-2 is only a warning).
#[test]
fn copilot_deny_flat_exit_0() {
    let m = manifest_one_command("copilot-cli", "PreToolUse", "printf 'no' >&2; exit 2");
    let out = run_hook::dispatch_core("copilot-cli", "PreToolUse", "{}", Some(&m));
    assert_eq!(out.exit_code, 0);
    assert!(
        out.stdout.contains("\"permissionDecision\":\"deny\""),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout.contains("\"permissionDecisionReason\""),
        "{}",
        out.stdout
    );
}

/// Gemini (ClaudeStyle): a hook that injects `additionalContext` nests it under
/// `hookSpecificOutput` with the `hookEventName`, at exit 0.
#[test]
fn gemini_additional_context_nested_exit_0() {
    let cmd = r#"printf '{"hookSpecificOutput":{"additionalContext":"extra"}}'"#;
    let m = manifest_one_command("gemini", "UserPromptSubmit", cmd);
    let out = run_hook::dispatch_core("gemini", "UserPromptSubmit", "{}", Some(&m));
    assert_eq!(out.exit_code, 0);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).expect("valid JSON");
    assert_eq!(v["hookSpecificOutput"]["additionalContext"], "extra");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "UserPromptSubmit");
}

/// Copilot (CopilotFlat): the SAME injecting hook emits a FLAT top-level
/// `additionalContext` (NOT nested under `hookSpecificOutput`).
#[test]
fn copilot_additional_context_is_flat() {
    let cmd = r#"printf '{"hookSpecificOutput":{"additionalContext":"extra"}}'"#;
    let m = manifest_one_command("copilot-cli", "PostToolUse", cmd);
    let out = run_hook::dispatch_core("copilot-cli", "PostToolUse", "{}", Some(&m));
    assert_eq!(out.exit_code, 0);
    let v: serde_json::Value = serde_json::from_str(&out.stdout).expect("valid JSON");
    assert_eq!(v["additionalContext"], "extra");
    assert!(
        v.get("hookSpecificOutput").is_none(),
        "Copilot additionalContext must be flat, got: {}",
        out.stdout
    );
}

/// A plain allow (`exit 0`, no stdout) is the empty no-op at exit 0 on every
/// in-scope harness.
#[test]
fn allow_is_empty_no_op_per_harness() {
    for h in ["devin", "gemini", "codex", "cursor", "copilot-cli"] {
        let m = manifest_one_command(h, "PreToolUse", "exit 0");
        let out = run_hook::dispatch_core(h, "PreToolUse", "{}", Some(&m));
        assert_eq!(out.exit_code, 0, "{h} allow exit");
        assert!(
            out.stdout.is_empty(),
            "{h} allow must be empty, got: {}",
            out.stdout
        );
    }
}

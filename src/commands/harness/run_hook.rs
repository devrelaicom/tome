//! `tome harness run-hook` — the runtime plugin-hook dispatcher (the hot path).
//!
//! Reads the per-(workspace, harness) dispatch manifest US3 wrote, translates
//! the harness's native hook event JSON (on stdin) into Claude-Code-shaped
//! stdin, filters the manifest entries by CC matcher, runs the matching COMMAND
//! handlers, merges their decisions (most-restrictive-wins), and emits the
//! harness's native wire decision + exit code.
//!
//! ## THE invariant — fail-open totality
//!
//! Any Tome-side fault — a missing / unreadable / malformed manifest, an
//! unparsable stdin, a handler that errors or times out, or a CAUGHT PANIC at
//! the boundary — degrades to the harness's allow/no-op at **exit 0**. Tome
//! NEVER blocks the agent because of its OWN fault. A plugin hook that
//! LEGITIMATELY denies still blocks (its decision is faithfully relayed as the
//! harness wire shape). The mechanisms:
//!
//! * [`dispatch_core`] wraps [`dispatch_inner`] in
//!   [`std::panic::catch_unwind`] → [`fail_open_output`] on panic.
//! * [`compute`] reads the manifest via `read_manifest(path).ok()` — ANY
//!   read/parse error becomes `None`, and `None` → `fail_open_output`.
//! * an unknown `--harness` (no `wire_for`) → `fail_open_output`.
//! * [`fail_open_output`] = empty stdout + exit 0 (a valid allow/no-op on ALL
//!   five wires; the empty no-op is preferred over an explicit allow envelope).
//! * [`run`] ALWAYS returns `Ok(())`. It relays stdout, then — for a NON-ZERO
//!   wire exit code (a legitimate plugin BLOCK) — calls [`std::process::exit`]
//!   with that code. The run-hook process exit code IS the harness wire code
//!   (0/2/…), NEVER a [`TomeError`] code; the closed error set is sync-time only.
//!
//! Sync only — the dispatcher is a synchronous subprocess (no async leaks here;
//! `tests/harness_settings/sync_boundary.rs` enforces it).

use std::io::{Read, Write};

use serde_json::Value;

use crate::cli::HarnessRunHookArgs;
use crate::error::TomeError;
use crate::harness::HookWire;
use crate::harness::hooks_ir::{HookManifest, cc_tool_name, matcher_matches};
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::{ResolvedScope, WorkspaceName};

/// The pure result of a dispatch: the bytes to write to stdout, plus the
/// process exit code (the HARNESS wire code — 0 for allow/no-op, 2 for a
/// ClaudeStyle block, etc.).
pub struct DispatchOutput {
    pub stdout: String,
    pub exit_code: i32,
}

/// `tome harness run-hook` — relay a harness hook event through the dispatcher.
///
/// ALWAYS returns `Ok(())`: every Tome-side fault degrades to a fail-open allow
/// inside [`compute`]/[`dispatch_core`]. The only non-`Ok`-shaped exit is the
/// HARNESS wire code for a legitimate plugin block, applied via
/// [`std::process::exit`] (overriding Tome's normal `Result`→exit mapping).
pub fn run(
    args: HarnessRunHookArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    _mode: Mode,
) -> Result<(), TomeError> {
    // Read the harness's native event JSON from stdin. Best-effort: an empty or
    // unreadable stdin degrades to a fail-open allow downstream (never errors).
    let mut stdin = String::new();
    let _ = std::io::stdin().lock().read_to_string(&mut stdin);

    // US10: `--explain` prints what WOULD fire and runs nothing. Stubbed here —
    // a no-op that emits nothing and never blocks (returns Ok → exit 0).
    if args.explain {
        return explain(&args, scope, paths);
    }

    let out = compute(&args, scope, paths, &stdin);

    // Relay the harness wire bytes, then — for a LEGITIMATE plugin block — exit
    // with the harness wire code. This OVERRIDES Tome's normal Result→exit map:
    // the run-hook process exit code IS the harness wire code (0/2/…), never a
    // TomeError code. A Tome-side fault never reaches here as a non-zero code
    // (`compute` degrades every fault to a fail-open allow at exit 0).
    let _ = std::io::stdout().lock().write_all(out.stdout.as_bytes());
    if out.exit_code != 0 {
        std::process::exit(out.exit_code);
    }
    Ok(())
}

/// Resolve the workspace + manifest path and run the pure dispatcher. A bad
/// `--workspace` (or any other resolution fault) degrades to a fail-open allow.
fn compute(
    args: &HarnessRunHookArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    stdin: &str,
) -> DispatchOutput {
    let workspace: WorkspaceName = match args.workspace.as_deref() {
        Some(raw) => match WorkspaceName::parse(raw) {
            Ok(w) => w,
            // FAIL-OPEN: a malformed `--workspace` must not block the agent.
            Err(_) => return fail_open_output(&args.harness),
        },
        None => scope.scope.name().clone(),
    };

    let manifest_path = paths.hooks_manifest(&workspace, &args.harness);
    // `.ok()` swallows EVERY read/parse fault (missing / oversize / non-UTF-8 /
    // malformed JSON / symlink refusal) → None → fail-open in `dispatch_inner`.
    let manifest = crate::harness::hooks_ir::read_manifest(&manifest_path).ok();

    dispatch_core(&args.harness, &args.event, stdin, manifest.as_ref())
}

/// Pure, total dispatch. NEVER returns an error — any Tome fault becomes a
/// fail-open allow. `manifest: None` (missing/unreadable/malformed) → fail-open.
/// A panic anywhere in [`dispatch_inner`] is caught and turned into a fail-open
/// allow (effective under unwind builds; see the `panic = "abort"` caveat in
/// the crate's release profile — there the panic aborts instead).
pub fn dispatch_core(
    harness: &str,
    event_cc: &str,
    stdin: &str,
    manifest: Option<&HookManifest>,
) -> DispatchOutput {
    let wire = match wire_for(harness) {
        Some(w) => w,
        // Unknown harness (no hook_support) → fail-open allow.
        None => return fail_open_output(harness),
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch_inner(harness, wire, event_cc, stdin, manifest)
    }));
    result.unwrap_or_else(|_| fail_open_output(harness))
}

/// Resolve the harness's decision-protocol wire from the registry. `None` for
/// an unknown harness or one without `hook_support()` (→ fail-open caller-side).
fn wire_for(harness: &str) -> Option<HookWire> {
    crate::harness::lookup(harness)
        .and_then(|m| m.hook_support())
        .map(|s| s.wire)
}

/// The fail-open allow/no-op for any wire: empty stdout + exit 0. Empty stdout
/// is a valid allow on ALL five harnesses; the no-op (vs. an explicit allow
/// envelope) avoids injecting noise into the agent's context.
fn fail_open_output(_harness: &str) -> DispatchOutput {
    DispatchOutput {
        stdout: String::new(),
        exit_code: 0,
    }
}

/// US10 `--explain` stub: print what WOULD fire, run nothing. For now a no-op
/// that emits nothing and returns `Ok` (exit 0) — never blocks.
fn explain(
    _args: &HarnessRunHookArgs,
    _scope: &ResolvedScope,
    _paths: &Paths,
) -> Result<(), TomeError> {
    Ok(())
}

/// The pipeline core (stdin-translate → filter → exec → merge → emit), built up
/// across US4.2–US4.5. A `None` manifest (missing/unreadable/malformed) is the
/// canonical fail-open path; an event absent from the manifest has no hooks and
/// is likewise a fail-open allow. US4.2 lands the stdin-translate + matcher
/// filter; US4.3–4.5 land exec → merge → emit (until then a match is a
/// non-blocking allow).
fn dispatch_inner(
    harness: &str,
    wire: HookWire,
    event_cc: &str,
    stdin: &str,
    manifest: Option<&HookManifest>,
) -> DispatchOutput {
    // No manifest (missing / unreadable / malformed) → fail-open allow.
    let Some(manifest) = manifest else {
        return fail_open_output(harness);
    };

    // Translate the harness's native event JSON into CC-shaped stdin. An
    // unparsable stdin degrades to `Value::Null` (→ an empty CC object after
    // backfill), never an error.
    let raw: Value = serde_json::from_str(stdin).unwrap_or(Value::Null);
    let cc_value = harness_event_to_cc(wire, event_cc, harness, &raw);
    // The CC tool name the plugin matchers (CC vocabulary) filter against.
    let cc_tool = cc_value
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // The manifest entries registered for this CC event. Absent → no hooks →
    // fail-open allow.
    let Some(entries) = manifest.events.get(event_cc) else {
        return fail_open_output(harness);
    };

    let has_match = entries
        .iter()
        .any(|e| matcher_matches(e.matcher.as_deref(), cc_tool));
    if !has_match {
        return fail_open_output(harness);
    }
    // US4.3 execs the matching handlers and US4.4/4.5 merge + emit; until then a
    // match is a non-blocking allow.
    fail_open_output(harness)
}

/// Translate a harness's native hook event JSON into a normalized CC-shaped
/// stdin object (US4.2). Backfills `hook_event_name`/`session_id`/`cwd`/
/// `permission_mode` (and event-specific `source`/`tool_response`), applies the
/// per-wire field remaps (e.g. Cursor `conversation_id` → `session_id`), and
/// rewrites the native tool name → CC canonical via
/// [`cc_tool_name`]`(harness, native).unwrap_or(native)` so the plugin script
/// sees `tool_name:"Bash"` regardless of the harness vocabulary. `tool_input`
/// passes through as-is (full per-tool input-schema normalization is a
/// documented v1 limitation; the namespaced `tome` block is US8).
fn harness_event_to_cc(wire: HookWire, event_cc: &str, harness: &str, raw: &Value) -> Value {
    let mut obj = raw.as_object().cloned().unwrap_or_default();

    // Per-wire native-key remaps (harness key → CC key) BEFORE the universal
    // backfill, so a remapped value wins over an empty default.
    match wire {
        HookWire::CursorSnake => {
            // Cursor: `conversation_id` → `session_id` (C9); `workspace_roots[0]`
            // → `cwd`. No generic tool_name/tool_input on the base event; v1
            // translates what is present and backfills the rest.
            if !obj.contains_key("session_id") {
                let conversation_id = obj.get("conversation_id").cloned();
                if let Some(cid) = conversation_id {
                    obj.insert("session_id".to_string(), cid);
                }
            }
            if !obj.contains_key("cwd") {
                let first_root = obj
                    .get("workspace_roots")
                    .and_then(|v| v.as_array())
                    .and_then(|a| a.first())
                    .cloned();
                if let Some(root) = first_root {
                    obj.insert("cwd".to_string(), root);
                }
            }
        }
        // Codex / Copilot (PascalCase) stdin is already CC-shaped; ClaudeStyle
        // (Devin/Gemini) is CC-ish. Nothing to remap beyond the universal
        // tool-name rewrite + backfill below.
        HookWire::Codex | HookWire::CopilotFlat | HookWire::ClaudeStyle => {}
    }

    // Universal: rewrite the native tool name → CC canonical. An unmapped native
    // name falls back to itself so a matcher referencing it directly still hits.
    if let Some(native) = obj.get("tool_name").and_then(|v| v.as_str()) {
        let cc = cc_tool_name(harness, native).unwrap_or(native).to_string();
        obj.insert("tool_name".to_string(), Value::String(cc));
    }

    // Universal backfills (only when absent).
    obj.entry("hook_event_name")
        .or_insert_with(|| Value::String(event_cc.to_string()));
    obj.entry("session_id")
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("cwd")
        .or_insert_with(|| Value::String(String::new()));
    obj.entry("permission_mode")
        .or_insert_with(|| Value::String("default".to_string()));
    if event_cc == "SessionStart" {
        obj.entry("source")
            .or_insert_with(|| Value::String("startup".to_string()));
    }
    if event_cc == "PostToolUse" {
        obj.entry("tool_response").or_insert(Value::Null);
    }

    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (a) Cursor's `conversation_id` maps to CC `session_id`; (b) Gemini's
    /// `run_shell_command` native tool name rewrites to `tool_name:"Bash"`; (c) a
    /// missing `cwd` is backfilled (here from Cursor's `workspace_roots[0]`).
    #[test]
    fn harness_event_to_cc_remaps_and_backfills() {
        // (a) Cursor conversation_id → session_id; workspace_roots[0] → cwd.
        let raw = serde_json::json!({
            "conversation_id": "conv-123",
            "workspace_roots": ["/repo/root"],
            "tool_name": "Read",
        });
        let cc = harness_event_to_cc(HookWire::CursorSnake, "PreToolUse", "cursor", &raw);
        assert_eq!(cc["session_id"], "conv-123");
        assert_eq!(cc["cwd"], "/repo/root");
        assert_eq!(cc["hook_event_name"], "PreToolUse");
        assert_eq!(cc["permission_mode"], "default");

        // (b) Gemini run_shell_command → Bash.
        let raw = serde_json::json!({ "tool_name": "run_shell_command" });
        let cc = harness_event_to_cc(HookWire::ClaudeStyle, "PreToolUse", "gemini", &raw);
        assert_eq!(cc["tool_name"], "Bash");

        // (c) Missing cwd is backfilled to an empty string when the harness has
        // no source for it (Devin, U2).
        let raw = serde_json::json!({ "tool_name": "exec" });
        let cc = harness_event_to_cc(HookWire::ClaudeStyle, "PreToolUse", "devin", &raw);
        assert_eq!(cc["cwd"], "");
        assert_eq!(cc["session_id"], "");
        assert_eq!(cc["tool_name"], "Bash");
    }

    /// An unmapped native tool name falls back to itself (so a matcher that
    /// references the native token directly still matches), and a wholly empty
    /// raw object still produces the universal CC backfills.
    #[test]
    fn harness_event_to_cc_unmapped_tool_and_empty_raw() {
        let raw = serde_json::json!({ "tool_name": "totally_custom" });
        let cc = harness_event_to_cc(HookWire::ClaudeStyle, "PreToolUse", "gemini", &raw);
        assert_eq!(cc["tool_name"], "totally_custom");

        let cc = harness_event_to_cc(HookWire::Codex, "SessionStart", "codex", &Value::Null);
        assert_eq!(cc["hook_event_name"], "SessionStart");
        assert_eq!(cc["source"], "startup");
        assert_eq!(cc["session_id"], "");
    }
}

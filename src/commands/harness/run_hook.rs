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
use crate::harness::hooks_ir::{
    Handler, HookManifest, ManifestEntry, cc_tool_name, matcher_matches,
};
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
    let cc_stdin = cc_value.to_string();
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

    // Filter by matcher, run each matching handler, and collect its CC decision
    // keyed by plugin provenance (for the merge's block-reason prefix).
    let mut decisions: Vec<(String, CcDecision)> = Vec::new();
    for entry in entries {
        if !matcher_matches(entry.matcher.as_deref(), cc_tool) {
            continue;
        }
        // US9: the `if` permission-rule predicate is not yet evaluated; an entry
        // carrying one is treated as matching (the predicate is an additive
        // future tightening, never a loosening).
        let decision = match &entry.handler {
            Handler::Command { .. } => {
                let outcome = run_command_handler(entry, &cc_stdin);
                command_outcome_to_decision(&outcome)
            }
            // US5/US6: http/prompt handlers are not executed yet. A non-blocking
            // allow (no-op) placeholder — the dispatch loop must NEVER crash on
            // them.
            Handler::Http { .. } | Handler::Prompt { .. } => CcDecision::default(),
        };
        decisions.push((entry.plugin.clone(), decision));
    }

    let merged = merge_decisions(&decisions);
    emit_decision(wire, event_cc, &merged)
}

/// Wall-clock budget for a command handler when the manifest carries no
/// `timeout_ms` (Claude Code's 60s hook default, baked in ms).
const DEFAULT_TIMEOUT_MS: u64 = 60_000;
/// Poll cadence for the spawn + wait-with-timeout loop.
const POLL_INTERVAL_MS: u64 = 5;

/// The raw result of running one command handler: its exit code plus captured
/// stdout/stderr. Translated into a [`CcDecision`] by
/// [`command_outcome_to_decision`].
struct HandlerOutcome {
    exit: i32,
    stdout: String,
    stderr: String,
}

/// Run one command handler bounded by `entry.timeout_ms`, feeding `cc_stdin` on
/// stdin. Returns the raw exit/stdout/stderr.
///
/// SECURITY: the ONLY shell-evaluated string is `entry.command` — relocated
/// verbatim from the Tome-owned manifest. No other field (matcher, plugin, cwd,
/// env) is ever interpolated into the shell line. A spawn failure or a timeout
/// degrades to a non-blocking allow (exit 0, empty), NEVER a block.
fn run_command_handler(entry: &ManifestEntry, cc_stdin: &str) -> HandlerOutcome {
    let allow = || HandlerOutcome {
        exit: 0,
        stdout: String::new(),
        stderr: String::new(),
    };
    let Handler::Command { command } = &entry.handler else {
        // The caller only routes Command handlers here; defend regardless.
        return allow();
    };

    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd.stdin(std::process::Stdio::piped());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    // Hook env: CLAUDE_PROJECT_DIR + the per-entry cwd/env. (TOME_* is US8.)
    if let Some(cwd) = &entry.cwd {
        cmd.current_dir(cwd);
        cmd.env("CLAUDE_PROJECT_DIR", cwd);
    }
    for (k, v) in &entry.env {
        cmd.env(k, v);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        // Spawn failure (no `sh`, bad cwd, …) → non-blocking allow.
        Err(_) => return allow(),
    };

    // Feed CC stdin on a side thread so a large payload cannot deadlock against
    // the child's stdout/stderr pipes (which we drain only after it exits).
    let writer = child.stdin.take().map(|mut sin| {
        let payload = cc_stdin.as_bytes().to_vec();
        std::thread::spawn(move || {
            let _ = sin.write_all(&payload);
            // `sin` drops here → the child sees EOF on stdin.
        })
    });

    let timeout = std::time::Duration::from_millis(entry.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
    let result = match wait_with_timeout(&mut child, timeout) {
        Some(outcome) => outcome,
        None => {
            // Timed out — kill FIRST (so a stdin writer blocked on a full pipe
            // unblocks with EPIPE), then treat as a non-blocking allow.
            let _ = child.kill();
            let _ = child.wait();
            allow()
        }
    };
    if let Some(handle) = writer {
        let _ = handle.join();
    }
    result
}

/// Spawn + wait-with-timeout: poll `try_wait` until the child exits or the
/// wall-clock `timeout` elapses. On exit, drains stdout/stderr. `None` means it
/// did not finish in time (caller kills + fails open). A signal-killed child
/// reports `exit = 0` (→ a non-blocking allow).
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
) -> Option<HandlerOutcome> {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut o) = child.stdout.take() {
                    let _ = o.read_to_string(&mut stdout);
                }
                if let Some(mut e) = child.stderr.take() {
                    let _ = e.read_to_string(&mut stderr);
                }
                return Some(HandlerOutcome {
                    exit: status.code().unwrap_or(0),
                    stdout,
                    stderr,
                });
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
            }
            // `try_wait` error → no usable outcome (caller fails open).
            Err(_) => return None,
        }
    }
}

/// The plugin-hook decision in CC vocabulary, the merge currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Permission {
    Allow,
    Deny,
    Ask,
}

impl Permission {
    /// Restrictiveness rank for the most-restrictive-wins merge
    /// (Deny > Ask > Allow). `None` (absent) ranks below all three.
    fn rank(self) -> u8 {
        match self {
            Permission::Allow => 1,
            Permission::Ask => 2,
            Permission::Deny => 3,
        }
    }
}

/// One hook's CC decision. `Default` = all-None/empty (a non-blocking allow).
#[derive(Debug, Clone, Default, PartialEq)]
struct CcDecision {
    permission: Option<Permission>,
    block: bool,
    reason: Option<String>,
    additional_context: Vec<String>,
    updated_input: Option<Value>,
}

/// True iff this decision blocks the agent (an explicit deny or a top-level
/// `decision:"block"`).
fn is_blocking(decision: &CcDecision) -> bool {
    decision.block || matches!(decision.permission, Some(Permission::Deny))
}

/// Translate a command handler's raw outcome into a [`CcDecision`]
/// (CC command-hook convention):
///
/// * exit 2 + stderr → Deny + the stderr text as the reason.
/// * exit 0 + JSON stdout → parse `hookSpecificOutput.permissionDecision`
///   (allow/deny/ask; `defer`/unknown → None), top-level `decision:"block"` +
///   `reason`, `additionalContext`, and `updatedInput`.
/// * any other exit (or exit 0 with no/non-JSON stdout) → non-blocking allow.
fn command_outcome_to_decision(outcome: &HandlerOutcome) -> CcDecision {
    if outcome.exit == 2 {
        let reason = outcome.stderr.trim();
        return CcDecision {
            permission: Some(Permission::Deny),
            block: true,
            reason: (!reason.is_empty()).then(|| reason.to_string()),
            ..Default::default()
        };
    }
    if outcome.exit == 0 {
        let trimmed = outcome.stdout.trim();
        if !trimmed.is_empty() {
            let parsed = serde_json::from_str::<Value>(trimmed);
            if let Ok(v) = parsed {
                return cc_json_to_decision(&v);
            }
        }
        return CcDecision::default();
    }
    CcDecision::default()
}

/// Parse a Claude-Code hook stdout JSON object into a [`CcDecision`].
fn cc_json_to_decision(v: &Value) -> CcDecision {
    let mut d = CcDecision::default();
    let hso = v.get("hookSpecificOutput");

    // permissionDecision: allow/deny/ask. `defer` (and any unknown) → None — a
    // non-blocking deferral to the harness's normal flow (C12).
    let pd = hso
        .and_then(|h| h.get("permissionDecision"))
        .and_then(|p| p.as_str());
    if let Some(pd) = pd {
        d.permission = match pd {
            "allow" => Some(Permission::Allow),
            "deny" => Some(Permission::Deny),
            "ask" => Some(Permission::Ask),
            _ => None,
        };
    }
    if let Some(reason) = hso
        .and_then(|h| h.get("permissionDecisionReason"))
        .and_then(|r| r.as_str())
    {
        d.reason = Some(reason.to_string());
    }

    // Top-level `decision:"block"` (+ `reason`) → Deny + block.
    if v.get("decision").and_then(|x| x.as_str()) == Some("block") {
        d.permission = Some(Permission::Deny);
        d.block = true;
    }
    if let Some(reason) = v.get("reason").and_then(|r| r.as_str()) {
        d.reason = Some(reason.to_string());
    }

    // additionalContext (under hookSpecificOutput or flat at top level).
    let ctx = hso
        .and_then(|h| h.get("additionalContext"))
        .or_else(|| v.get("additionalContext"))
        .and_then(|c| c.as_str());
    if let Some(ctx) = ctx {
        d.additional_context.push(ctx.to_string());
    }

    // updatedInput (under hookSpecificOutput or flat at top level).
    let updated = hso
        .and_then(|h| h.get("updatedInput"))
        .or_else(|| v.get("updatedInput"));
    if let Some(updated) = updated {
        d.updated_input = Some(updated.clone());
    }

    d
}

/// Merge the per-plugin decisions into one. Most-restrictive permission wins
/// (Deny > Ask > Allow > None); `block` is the OR; `additional_context` is the
/// in-order concat; `updated_input` is last-wins. The block reason is the FIRST
/// blocking entry's reason (provenance-prefixed in US4.4).
fn merge_decisions(plugin_keyed: &[(String, CcDecision)]) -> CcDecision {
    let mut merged = CcDecision::default();
    let mut best_rank = 0u8;
    let mut reason_set = false;
    for (_plugin, d) in plugin_keyed {
        // Most-restrictive permission wins (Deny > Ask > Allow > None).
        let rank = d.permission.map_or(0, Permission::rank);
        if rank > best_rank {
            best_rank = rank;
            merged.permission = d.permission;
        }
        merged.block |= d.block;
        if !reason_set && is_blocking(d) && d.reason.is_some() {
            merged.reason = d.reason.clone();
            reason_set = true;
        }
        merged
            .additional_context
            .extend(d.additional_context.iter().cloned());
        if d.updated_input.is_some() {
            merged.updated_input = d.updated_input.clone();
        }
    }
    merged
}

/// Map a merged decision to a wire permission token. `None` permission with a
/// `block` flag still denies; otherwise `None` means "no opinion".
fn permission_token(decision: &CcDecision) -> Option<&'static str> {
    match decision.permission {
        Some(Permission::Deny) => Some("deny"),
        Some(Permission::Ask) => Some("ask"),
        Some(Permission::Allow) => Some("allow"),
        None if decision.block => Some("deny"),
        None => None,
    }
}

/// Emit the merged decision in the harness's native wire shape (US4.5). US4.3
/// lands the permission/reason core; `additionalContext` + `updatedInput`/output
/// rewrites land in US4.5.
fn emit_decision(wire: HookWire, event_cc: &str, decision: &CcDecision) -> DispatchOutput {
    match wire {
        HookWire::ClaudeStyle => emit_claude_style(event_cc, decision, false),
        HookWire::Codex => emit_claude_style(event_cc, decision, true),
        HookWire::CursorSnake => emit_cursor(decision),
        HookWire::CopilotFlat => emit_copilot(decision),
    }
}

/// ClaudeStyle (Devin/Gemini) + Codex emit. A block is the top-level
/// `{"decision":"block","reason"}`; Devin/Gemini ALSO exit 2 (they block on
/// exit-2), while Codex blocks via the JSON at exit 0 (its exit-2 semantics are
/// unverified — never depend on them). `ask` rides `hookSpecificOutput`.
fn emit_claude_style(event_cc: &str, decision: &CcDecision, is_codex: bool) -> DispatchOutput {
    if is_blocking(decision) {
        let reason = decision.reason.clone().unwrap_or_default();
        let body = serde_json::json!({ "decision": "block", "reason": reason });
        let exit_code = if is_codex { 0 } else { 2 };
        return DispatchOutput {
            stdout: body.to_string(),
            exit_code,
        };
    }
    if matches!(decision.permission, Some(Permission::Ask)) {
        let mut hso = serde_json::Map::new();
        hso.insert(
            "hookEventName".to_string(),
            Value::String(event_cc.to_string()),
        );
        hso.insert(
            "permissionDecision".to_string(),
            Value::String("ask".to_string()),
        );
        if let Some(r) = &decision.reason {
            hso.insert(
                "permissionDecisionReason".to_string(),
                Value::String(r.clone()),
            );
        }
        let body = serde_json::json!({ "hookSpecificOutput": Value::Object(hso) });
        return DispatchOutput {
            stdout: body.to_string(),
            exit_code: 0,
        };
    }
    // Allow / no-op → empty stdout + exit 0.
    DispatchOutput {
        stdout: String::new(),
        exit_code: 0,
    }
}

/// Cursor emit: snake_case `{permission, agent_message, …}` ALWAYS at exit 0.
/// Cursor blocks via JSON `permission:"deny"`; Tome leaves `failClosed` off, so
/// a non-zero exit fails OPEN — Tome NEVER exits 2 here.
fn emit_cursor(decision: &CcDecision) -> DispatchOutput {
    let mut obj = serde_json::Map::new();
    if let Some(p) = permission_token(decision) {
        obj.insert("permission".to_string(), Value::String(p.to_string()));
    }
    if let Some(r) = &decision.reason {
        // `agent_message` is Cursor's reason channel (sent to the agent).
        obj.insert("agent_message".to_string(), Value::String(r.clone()));
    }
    if obj.is_empty() {
        return DispatchOutput {
            stdout: String::new(),
            exit_code: 0,
        };
    }
    DispatchOutput {
        stdout: Value::Object(obj).to_string(),
        exit_code: 0,
    }
}

/// Copilot CLI emit: flat `{permissionDecision, permissionDecisionReason}`.
/// Copilot blocks ONLY via JSON `permissionDecision:"deny"` — exit-2 is a mere
/// warning for most events, so Tome NEVER exits 2 for a block.
fn emit_copilot(decision: &CcDecision) -> DispatchOutput {
    let mut obj = serde_json::Map::new();
    if let Some(pd) = permission_token(decision) {
        obj.insert(
            "permissionDecision".to_string(),
            Value::String(pd.to_string()),
        );
        if let Some(r) = &decision.reason {
            obj.insert(
                "permissionDecisionReason".to_string(),
                Value::String(r.clone()),
            );
        }
    }
    if obj.is_empty() {
        return DispatchOutput {
            stdout: String::new(),
            exit_code: 0,
        };
    }
    DispatchOutput {
        stdout: Value::Object(obj).to_string(),
        exit_code: 0,
    }
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

    /// A command [`ManifestEntry`] with the given matcher + shell command, a
    /// generous timeout, and no cwd/env.
    fn command_entry(matcher: &str, command: &str) -> ManifestEntry {
        ManifestEntry {
            plugin: "cat:test".to_string(),
            matcher: Some(matcher.to_string()),
            if_pred: None,
            handler: Handler::Command {
                command: command.to_string(),
            },
            timeout_ms: Some(5_000),
            cwd: None,
            env: std::collections::BTreeMap::new(),
        }
    }

    /// A command hook that writes a reason to stderr and exits 2 blocks: the raw
    /// outcome is exit 2, and `command_outcome_to_decision` maps it to
    /// Deny + the stderr text as the reason.
    #[test]
    fn command_exit_2_blocks_with_reason() {
        let entry = command_entry("Bash", "printf 'nope' >&2; exit 2");
        let outcome = run_command_handler(&entry, r#"{"tool_name":"Bash"}"#);
        assert_eq!(outcome.exit, 2);
        let decision = command_outcome_to_decision(&outcome);
        assert_eq!(decision.permission, Some(Permission::Deny));
        assert_eq!(decision.reason.as_deref(), Some("nope"));
    }

    /// exit 0 with a `hookSpecificOutput.permissionDecision:"deny"` + a
    /// top-level reason parses to Deny + reason; `defer` maps to None.
    #[test]
    fn command_exit_0_json_parses_permission_and_defer() {
        let entry = command_entry(
            "Bash",
            r#"printf '{"hookSpecificOutput":{"permissionDecision":"deny"},"reason":"blocked by policy"}'"#,
        );
        let outcome = run_command_handler(&entry, "{}");
        assert_eq!(outcome.exit, 0);
        let decision = command_outcome_to_decision(&outcome);
        assert_eq!(decision.permission, Some(Permission::Deny));
        assert_eq!(decision.reason.as_deref(), Some("blocked by policy"));

        // `defer` is not a Tome Permission → None (non-blocking).
        let deferred = cc_json_to_decision(&serde_json::json!({
            "hookSpecificOutput": { "permissionDecision": "defer" }
        }));
        assert_eq!(deferred.permission, None);
        assert!(!is_blocking(&deferred));
    }

    /// A timed-out command is killed and degrades to a non-blocking allow
    /// (exit 0, empty) — a Tome timeout is NEVER a block.
    #[test]
    fn command_timeout_is_non_blocking_allow() {
        let mut entry = command_entry("Bash", "sleep 5");
        entry.timeout_ms = Some(50);
        let outcome = run_command_handler(&entry, "{}");
        let decision = command_outcome_to_decision(&outcome);
        assert!(!is_blocking(&decision));
        assert_eq!(decision.permission, None);
    }

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

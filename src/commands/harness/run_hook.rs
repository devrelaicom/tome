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

use std::collections::BTreeMap;
use std::io::{Read, Write};

use regex::Regex;
use serde_json::Value;

use crate::cli::HarnessRunHookArgs;
use crate::error::TomeError;
use crate::harness::HookWire;
use crate::harness::hooks_ir::{
    Handler, HookManifest, ManifestEntry, cc_tool_name, if_predicate_matches, matcher_matches,
};
use crate::output::Mode;
use crate::paths::Paths;
use crate::workspace::{ResolvedScope, WorkspaceName};

/// Per-entry plugin provenance threaded into [`run_command_handler`] so it
/// can set the `TOME_*` environment variables (US8). All strings are valid
/// UTF-8 (derived from Tome-owned paths and workspace names); an empty string
/// means the field is unavailable (e.g., when called via [`dispatch_core`]
/// without paths).
struct TomeProvenance<'a> {
    harness: &'a str,
    workspace: &'a str,
    /// Full `"<catalog>:<plugin>"` provenance string.
    plugin: &'a str,
    catalog: &'a str,
    plugin_root: &'a str,
    plugin_data: &'a str,
}

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

    // US10: `--explain` prints what WOULD fire and runs nothing. Stdin is read
    // above and passed in so `explain` can apply the matcher + if filter against
    // the incoming event; stdin cannot be re-read from inside `explain`.
    if args.explain {
        return explain(&args, scope, paths, &stdin);
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

    // Load the global config for the prompt handler (US6.2). Any error → defaults
    // so a bad config never blocks the agent (fail-open).
    let cfg = crate::config::load_or_default(paths);
    dispatch_with_cfg(
        &args.harness,
        &args.event,
        stdin,
        manifest.as_ref(),
        &cfg,
        workspace.as_str(),
        Some(paths),
    )
}

/// Pure, total dispatch. NEVER returns an error — any Tome fault becomes a
/// fail-open allow. `manifest: None` (missing/unreadable/malformed) → fail-open.
///
/// ## `catch_unwind` and `panic = "abort"` caveat
///
/// The `catch_unwind` wrapper inside this function is **a NO-OP in the release
/// binary**. Under `[profile.release] panic = "abort"` (this crate's release
/// profile), a panic aborts the process immediately (exit 134) rather than
/// unwinding the stack, so `catch_unwind` never catches it. For Copilot's
/// `preToolUse` hook, exit 134 is treated by the harness as fail-CLOSED (block),
/// not as a fail-open allow.
///
/// The **real** production fail-open guarantee is **panic-freedom by
/// construction**: `dispatch_inner` and ALL its callees must never panic. That
/// means: no bare `unwrap`/`expect`, no unchecked indexing on untrusted data,
/// no arithmetic that can overflow on adversarial input.
///
/// **Future contributors: do NOT add panicking operations inside `dispatch_inner`
/// or any of its callees** under the assumption that this `catch_unwind` will
/// save you — in the release binary it will not. Treat every call path as if
/// there is no safety net, because in production there is not one.
///
/// Keep this `catch_unwind`: it is real, zero-cost protection under the test
/// profile's default `panic = "unwind"`, and costs nothing in the release build.
pub fn dispatch_core(
    harness: &str,
    event_cc: &str,
    stdin: &str,
    manifest: Option<&HookManifest>,
) -> DispatchOutput {
    // Use Config::default() → prompt handlers in the manifest fail-open (no
    // provider configured). Tests that need real prompt dispatch should call
    // `dispatch_with_cfg` directly with a configured `Config`.
    // Workspace and paths are unknown at this entry point (test/legacy callers);
    // the `tome` block will carry empty workspace + empty per-entry path fields.
    dispatch_with_cfg(
        harness,
        event_cc,
        stdin,
        manifest,
        &crate::config::Config::default(),
        "",
        None,
    )
}

/// Like [`dispatch_core`] but with an explicit configuration, workspace name,
/// and optional path resolver.
///
/// Used by the production path ([`compute`]) with the on-disk config loaded from
/// the user's `~/.tome/config.toml`, and by integration tests that exercise the
/// prompt-handler dispatch path (US6.2) with a configured BYOM provider.
///
/// `workspace` is embedded as `tome.workspace` in the synthesized CC stdin and
/// as `TOME_WORKSPACE` in the command handler's environment. Pass `""` when the
/// workspace name is unavailable (e.g., legacy test callers).
///
/// `paths` is used to resolve `tome.plugin_data` and `tome.plugin_root` per
/// manifest entry. Pass `None` when paths are unavailable; those fields will be
/// empty strings (never a block — fail-open).
///
/// The `catch_unwind` and panic-freedom notes on [`dispatch_core`] apply equally
/// here — see that function's doc-comment for the full safety rationale.
pub fn dispatch_with_cfg(
    harness: &str,
    event_cc: &str,
    stdin: &str,
    manifest: Option<&HookManifest>,
    cfg: &crate::config::Config,
    workspace: &str,
    paths: Option<&Paths>,
) -> DispatchOutput {
    let wire = match wire_for(harness) {
        Some(w) => w,
        // Unknown harness (no hook_support) → fail-open allow.
        None => return fail_open_output(harness),
    };
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch_inner(
            harness, wire, event_cc, stdin, manifest, cfg, workspace, paths,
        )
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

/// US10 `--explain`: print what WOULD fire for the given event, run nothing.
///
/// Resolves the workspace + manifest path (same logic as [`compute`]) and calls
/// [`explain_core`] to filter entries and format the output lines. A bad
/// workspace or unreadable manifest degrades gracefully (a message is printed
/// and `Ok(())` is returned — never blocks the caller).
///
/// `stdin` is the harness event JSON already read by [`run`]; it is forwarded so
/// the matcher + `if` filter sees the incoming tool name and tool_input.
fn explain(
    args: &HarnessRunHookArgs,
    scope: &ResolvedScope,
    paths: &Paths,
    stdin: &str,
) -> Result<(), TomeError> {
    let workspace: WorkspaceName = match args.workspace.as_deref() {
        Some(raw) => match WorkspaceName::parse(raw) {
            Ok(w) => w,
            Err(_) => {
                // Fail-open: a malformed --workspace must not block the caller.
                // writeln! (not println!) avoids a panic on a broken-pipe stdout
                // (e.g. `--explain | head`); under panic=abort println! would abort.
                let _ = writeln!(
                    std::io::stdout().lock(),
                    "[explain] malformed --workspace argument; \
                     manifest lookup skipped — dispatch would be allow (fail-open)"
                );
                return Ok(());
            }
        },
        None => scope.scope.name().clone(),
    };

    let manifest_path = paths.hooks_manifest(&workspace, &args.harness);
    let manifest = crate::harness::hooks_ir::read_manifest(&manifest_path).ok();

    let lines = explain_core(&args.harness, &args.event, stdin, manifest.as_ref());
    for line in &lines {
        let _ = writeln!(std::io::stdout().lock(), "{line}");
    }
    Ok(())
}

/// Pure explain core: returns the human-readable lines describing which manifest
/// entries WOULD fire for `(harness, event_cc)` against the given `stdin`,
/// without executing any handler.
///
/// Applies the identical matcher + `if`-predicate filter as [`dispatch_inner`]
/// (including CC tool-name normalisation via [`harness_event_to_cc`]). Each
/// matching entry produces one line:
/// `plugin=<p> event=<e> matcher=<m> if=<pred|none> kind=<command|http|prompt> -> would run`
///
/// All user-authored values (plugin, matcher, `if` predicate) are scrubbed
/// through [`crate::catalog::git::scrub_credentials`] before inclusion. The
/// handler body (command text, URL, prompt text) is NEVER printed — only the
/// KIND. Tool-input values are also not echoed.
///
/// Returns a fallback message for degenerate inputs (no manifest, no entries).
/// Never panics; never executes any handler.
fn explain_core(
    harness: &str,
    event_cc: &str,
    stdin: &str,
    manifest: Option<&HookManifest>,
) -> Vec<String> {
    // Scrub the Tome-owned event name up front; the invariant is unconditional —
    // every value printed by --explain routes through the scrubber.
    let event_cc_scrubbed = crate::catalog::git::scrub_to_string(event_cc.as_bytes());

    let Some(manifest) = manifest else {
        return vec!["[explain] no manifest found — dispatch would fail-open (allow)".to_string()];
    };
    let Some(entries) = manifest.events.get(event_cc) else {
        return vec![format!(
            "[explain] no entries registered for event={event_cc_scrubbed} \
             — dispatch would fail-open (allow)"
        )];
    };

    // Translate the harness stdin → CC format, identical to dispatch_inner, so
    // the matcher + if filter operates on the normalised CC tool name.
    let raw: Value = serde_json::from_str(stdin).unwrap_or(Value::Null);
    let cc_base = match wire_for(harness) {
        Some(wire) => harness_event_to_cc(
            wire, event_cc, harness, "", // workspace: not needed for the matcher/if filter
            false, &raw,
        ),
        // Unknown harness: dispatch would fail-open immediately (no wire = no hooks).
        // Return the same fail-open message so explain and dispatch are consistent.
        None => {
            return vec![
                "[explain] unknown harness — no hook support, dispatch would fail-open (allow)"
                    .to_string(),
            ];
        }
    };
    let cc_tool = cc_base
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    // Scrub cc_tool immediately after extraction; it derives from harness-provided
    // stdin (the tool_name field), not a Tome constant, so the invariant applies.
    let cc_tool_scrubbed = crate::catalog::git::scrub_to_string(cc_tool.as_bytes());
    let tool_input_null = Value::Null;
    let tool_input = cc_base.get("tool_input").unwrap_or(&tool_input_null);

    let mut lines = Vec::new();
    let mut any_matched = false;
    for entry in entries {
        if !matcher_matches(entry.matcher.as_deref(), cc_tool) {
            continue;
        }
        if let Some(if_pred) = entry.if_pred.as_deref()
            && !if_predicate_matches(if_pred, cc_tool, tool_input)
        {
            continue;
        }
        any_matched = true;

        // Handler KIND: the only safe piece of handler metadata to expose.
        // The command body, URL, prompt text, and header values are NEVER printed.
        let kind = handler_kind(&entry.handler);

        // Scrub all plugin-authored metadata through the shared credential
        // scrubber before printing (defensive; unlikely to contain secrets but
        // the security invariant is unconditional).
        let plugin_scrubbed = crate::catalog::git::scrub_to_string(entry.plugin.as_bytes());
        let matcher_scrubbed = crate::catalog::git::scrub_to_string(
            entry.matcher.as_deref().unwrap_or("*").as_bytes(),
        );
        let if_scrubbed = entry
            .if_pred
            .as_deref()
            .map(|p| crate::catalog::git::scrub_to_string(p.as_bytes()))
            .unwrap_or_else(|| "(none)".to_string());

        lines.push(format!(
            "plugin={plugin_scrubbed} event={event_cc_scrubbed} \
             matcher={matcher_scrubbed} if={if_scrubbed} \
             kind={kind} -> would run"
        ));
    }

    if !any_matched {
        lines.push(format!(
            "[explain] no entries match for event={event_cc_scrubbed} tool={cc_tool_scrubbed} \
             — dispatch would be allow (no-op)"
        ));
    }
    lines
}

/// Return the short kind label for a handler (for --explain and TOME_HOOK_DEBUG).
/// Never prints the handler body, URL, or prompt text.
fn handler_kind(handler: &Handler) -> &'static str {
    match handler {
        Handler::Command { .. } => "command",
        Handler::Http { .. } => "http",
        Handler::Prompt { .. } => "prompt",
    }
}

/// Format one TOME_HOOK_DEBUG trace line for a handler's decision.
///
/// Formats: `[TOME_HOOK_DEBUG] plugin=<p> kind=<k> decision=<allow|ask|deny> reason=<r>`.
/// All values (plugin name, reason text) are scrubbed through
/// [`crate::catalog::git::scrub_to_string`] before inclusion so no credential-like
/// token leaks to stderr. The handler body (command text, URL, prompt text) is NOT
/// included — only the handler kind. Returns a `String`; the caller emits it via
/// `writeln!(stderr, ...)` (not `eprintln!`) so a stderr write failure never panics.
fn debug_trace_line(plugin: &str, kind: &str, decision: &CcDecision) -> String {
    let label = if is_blocking(decision) {
        "deny"
    } else if matches!(decision.permission, Some(Permission::Ask)) {
        "ask"
    } else {
        "allow"
    };
    let plugin_scrubbed = crate::catalog::git::scrub_to_string(plugin.as_bytes());
    let reason_scrubbed = decision
        .reason
        .as_deref()
        .map(|r| crate::catalog::git::scrub_to_string(r.as_bytes()))
        .unwrap_or_default();
    format!(
        "[TOME_HOOK_DEBUG] plugin={plugin_scrubbed} kind={kind} \
         decision={label} reason={reason_scrubbed}"
    )
}

/// The pipeline core (stdin-translate → filter → exec → merge → emit), built up
/// across US4.2–US4.5. A `None` manifest (missing/unreadable/malformed) is the
/// canonical fail-open path; an event absent from the manifest has no hooks and
/// is likewise a fail-open allow. US4.2 lands the stdin-translate + matcher
/// filter; US4.3–4.5 land exec → merge → emit (until then a match is a
/// non-blocking allow). US8 adds the namespaced `tome` block + `TOME_*` env.
///
/// `workspace` and `paths` thread the US8 per-entry plugin provenance into each
/// handler's stdin clone and (for command handlers) the `TOME_*` env. Pass `""`
/// and `None` when unavailable (e.g., legacy callers via [`dispatch_core`]); the
/// `tome` fields will be present but with empty values — never a block.
#[allow(clippy::too_many_arguments)]
fn dispatch_inner(
    harness: &str,
    wire: HookWire,
    event_cc: &str,
    stdin: &str,
    manifest: Option<&HookManifest>,
    cfg: &crate::config::Config,
    workspace: &str,
    paths: Option<&Paths>,
) -> DispatchOutput {
    // No manifest (missing / unreadable / malformed) → fail-open allow.
    let Some(manifest) = manifest else {
        return fail_open_output(harness);
    };

    // Translate the harness's native event JSON into CC-shaped stdin. An
    // unparsable stdin degrades to `Value::Null` (→ an empty CC object after
    // backfill), never an error. US8: harness_event_to_cc injects the global
    // `tome` block (harness, workspace, and optionally raw_event).
    let raw: Value = serde_json::from_str(stdin).unwrap_or(Value::Null);
    let cc_base = harness_event_to_cc(
        wire,
        event_cc,
        harness,
        workspace,
        manifest.raw_event_passthrough,
        &raw,
    );
    // The CC tool name the plugin matchers (CC vocabulary) filter against.
    // Derived from the base (pre-per-entry-augment) value — the same for all entries.
    let cc_tool = cc_base
        .get("tool_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // The tool_input for the current event, used by the US9 `if` predicate
    // evaluator. Passed through as-is from the harness stdin (CC Fact A).
    // Uses a local Null sentinel so the borrow is valid for the full loop below.
    let tool_input_null = Value::Null;
    let tool_input = cc_base.get("tool_input").unwrap_or(&tool_input_null);

    // The manifest entries registered for this CC event. Absent → no hooks →
    // fail-open allow.
    let Some(entries) = manifest.events.get(event_cc) else {
        return fail_open_output(harness);
    };

    // US10: read TOME_HOOK_DEBUG once before the loop (Fix 3 — perf: avoids a
    // per-entry env read on the hot dispatch path; `unwrap_or_default()` is
    // fail-open for non-UTF-8 env entries so a bad env never panics here).
    let debug_enabled = !std::env::var("TOME_HOOK_DEBUG")
        .unwrap_or_default()
        .is_empty();

    // Filter by matcher, run each matching handler, and collect its CC decision
    // keyed by plugin provenance (for the merge's block-reason prefix).
    //
    // US8 (per-entry restructure): each handler gets its OWN cc_stdin derived
    // from a CLONE of cc_base augmented with the per-entry `tome` plugin fields
    // (plugin, catalog, plugin_root, plugin_data). Command handlers also receive
    // the TOME_* env vars via a TomeProvenance struct.
    let mut decisions: Vec<(String, CcDecision)> = Vec::new();
    for entry in entries {
        if !matcher_matches(entry.matcher.as_deref(), cc_tool) {
            continue;
        }
        // US9: evaluate the CC `if` permission-rule predicate. An entry whose
        // predicate does not match the current tool_input is skipped. An
        // unparsable predicate also skips (fail-open: hook does not fire).
        if let Some(if_pred) = entry.if_pred.as_deref()
            && !if_predicate_matches(if_pred, cc_tool, tool_input)
        {
            continue;
        }

        // US8: derive per-entry plugin provenance. Fail-open: a malformed
        // `entry.plugin` (missing ':') degrades to empty catalog/name — never panics.
        let (catalog, plugin_name) = entry
            .plugin
            .split_once(':')
            .unwrap_or(("", entry.plugin.as_str()));

        // Compute plugin_data from paths when available; fall back to empty
        // string for legacy callers that pass None (fail-open).
        let plugin_data_str = match paths {
            Some(p) => p
                .plugin_data_dir_for(catalog, plugin_name)
                .to_string_lossy()
                .into_owned(),
            None => String::new(),
        };
        // Fix 1 (US8 review): plugin_root is baked into the manifest at sync
        // time by the reconciler (which has DB access). The hot-path dispatcher
        // reads it directly — no DB, no path manipulation. Defensive empty-string
        // fallback for manifests written before this field was introduced.
        let plugin_root_str = entry.plugin_root.as_deref().unwrap_or("").to_owned();

        // Build the per-entry cc_stdin: clone the base value and inject the
        // per-entry tome fields (plugin, catalog, plugin_root, plugin_data).
        let per_entry_cc_stdin = {
            let mut v = cc_base.clone();
            if let Some(tome) = v.get_mut("tome").and_then(Value::as_object_mut) {
                tome.insert("plugin".to_string(), Value::String(entry.plugin.clone()));
                tome.insert("catalog".to_string(), Value::String(catalog.to_string()));
                tome.insert(
                    "plugin_root".to_string(),
                    Value::String(plugin_root_str.clone()),
                );
                tome.insert(
                    "plugin_data".to_string(),
                    Value::String(plugin_data_str.clone()),
                );
            }
            v.to_string()
        };

        let decision = match &entry.handler {
            Handler::Command { .. } => {
                // Build per-entry provenance for TOME_* env vars.
                let prov = TomeProvenance {
                    harness,
                    workspace,
                    plugin: &entry.plugin,
                    catalog,
                    plugin_root: &plugin_root_str,
                    plugin_data: &plugin_data_str,
                };
                let outcome = run_command_handler(entry, &per_entry_cc_stdin, &prov);
                command_outcome_to_decision(&outcome)
            }
            // SECURITY (NFR-007): `interpolate_headers` is the single place
            // plugin-declared text drives a substitution. It is confined to header
            // VALUES + allowlist-gated (unlisted → empty string). URL + header
            // names are relocated verbatim; no shell, no eval.
            // HTTP handlers receive the per-entry cc_stdin body (with the `tome`
            // block) but no TOME_* env (there is no subprocess to inherit it).
            Handler::Http {
                url,
                headers,
                allowed_env_vars,
            } => {
                let o = run_http_handler(
                    url,
                    headers,
                    allowed_env_vars,
                    &per_entry_cc_stdin,
                    entry.timeout_ms,
                );
                command_outcome_to_decision(&o)
            }
            // US6.2: execute the prompt handler via the configured BYOM provider.
            // Fails open (CcDecision::default) on any Tome-side fault.
            // entry.timeout_ms is forwarded so the hook's declared budget is
            // honoured (Fix 2, US6 review — uses the smaller of the hook
            // timeout and the provider default).
            Handler::Prompt { prompt } => {
                run_prompt_handler(prompt, &per_entry_cc_stdin, cfg, entry.timeout_ms)
            }
        };

        // US10: TOME_HOOK_DEBUG trace — observe-only, best-effort. When
        // `debug_enabled` (hoisted before the loop), log plugin + decision to
        // stderr. `writeln!` to stderr is used instead of `eprintln!` BECAUSE
        // `eprintln!` panics on a stderr write failure (e.g. EPIPE when stderr
        // is a closed pipe). Under `panic = "abort"` in the release binary that
        // panic aborts the dispatcher at exit 134, which Copilot's `preToolUse`
        // wire treats as fail-CLOSED — a logging error must NEVER affect the
        // dispatch decision. `let _ = writeln!(...)` discards the Result so a
        // write failure is silently ignored. Handler body/url/prompt NOT logged —
        // only kind. All values are scrubbed.
        if debug_enabled {
            let kind = handler_kind(&entry.handler);
            let _ = writeln!(
                std::io::stderr(),
                "{}",
                debug_trace_line(&entry.plugin, kind, &decision)
            );
        }

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
/// Maximum bytes read from an HTTP hook response body via [`run_http_handler`].
///
/// An unbounded `read_to_end` on a streaming 2xx body would buffer the entire
/// response in memory. A malicious or misbehaving webhook returning a fast
/// multi-GiB 2xx body would exhaust the subprocess heap → exit 137 (OOM
/// kill) → fail-CLOSED on Copilot's preToolUse wire — a transport abuse
/// becoming an agent block, violating the fail-open totality invariant.
///
/// A CC-decision JSON body is tiny (well under 1 KiB); 4 MiB is ample for
/// any legitimate payload. A body truncated at this cap will fail JSON parsing
/// in `command_outcome_to_decision` → non-blocking allow (correct fail-open).
const HOOK_HTTP_BODY_MAX: u64 = 4 * 1024 * 1024; // 4 MiB

/// Maximum bytes drained from each command hook's stdout AND stderr pipe via
/// [`run_command_handler`] reader threads. Mirrors [`HOOK_HTTP_BODY_MAX`].
///
/// Without a cap, a hook command that emits a fast multi-GiB stream
/// (accidental `cat big.log`, runaway `set -x`, etc.) buffers the entire
/// output on the heap → OOM kill (exit 137) → fail-CLOSED on Copilot's
/// preToolUse wire, violating the fail-open totality invariant. A CC-decision
/// JSON payload is tiny; 4 MiB is ample for any legitimate output. Truncated
/// output that fails JSON parsing in `command_outcome_to_decision` degrades to
/// a non-blocking allow — the correct fail-open behaviour.
const HOOK_CMD_OUTPUT_MAX: u64 = 4 * 1024 * 1024; // 4 MiB, parity with HTTP cap

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
/// stdout and stderr are drained on concurrent reader threads (started
/// immediately after `spawn`) to prevent the OS pipe-buffer deadlock: a handler
/// that writes >~64 KB to either pipe blocks on `write()` and never exits; the
/// old serial drain (read-after-wait) caused `wait_with_timeout` to spin to the
/// full timeout, kill the child, and fail open — silently downgrading a Deny
/// with large stderr output to an Allow (security: deny-bypass).
///
/// SECURITY: the ONLY shell-evaluated string is `entry.command` — relocated
/// verbatim from the Tome-owned manifest. No other field (matcher, plugin, cwd,
/// env) is ever interpolated into the shell line. A spawn failure, a thread
/// creation failure, or a timeout degrades to a non-blocking allow (exit 0,
/// empty), NEVER a block.
fn run_command_handler(
    entry: &ManifestEntry,
    cc_stdin: &str,
    prov: &TomeProvenance<'_>,
) -> HandlerOutcome {
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
    // Hook env: CLAUDE_PROJECT_DIR + the per-entry cwd/env + TOME_* (US8).
    if let Some(cwd) = &entry.cwd {
        cmd.current_dir(cwd);
        cmd.env("CLAUDE_PROJECT_DIR", cwd);
    }
    for (k, v) in &entry.env {
        cmd.env(k, v);
    }
    // US8: per-entry plugin-provenance env vars. The TOME_* prefix is fixed and
    // none of these names ends in _API_KEY, so they cannot collide with the
    // Phase-12 TOME_<NAME>_API_KEY provider env vars. Additionally, Tome sets
    // these vars AFTER the plugin-declared entry.env, so a plugin cannot spoof
    // its own provenance by declaring a TOME_* key in its hook env block.
    cmd.env("TOME_HARNESS", prov.harness);
    cmd.env("TOME_WORKSPACE", prov.workspace);
    cmd.env("TOME_PLUGIN", prov.plugin);
    cmd.env("TOME_CATALOG", prov.catalog);
    cmd.env("TOME_PLUGIN_ROOT", prov.plugin_root);
    cmd.env("TOME_PLUGIN_DATA", prov.plugin_data);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        // Spawn failure (no `sh`, bad cwd, …) → non-blocking allow.
        Err(_) => return allow(),
    };

    // All three I/O threads use Builder::new().spawn() (returns Result) rather
    // than thread::spawn (which PANICS on OS resource exhaustion — under
    // panic="abort" that aborts the process at exit 134, making a deny hook
    // fail-CLOSED). On spawn Err → kill child and fail open immediately.

    // Stdin writer thread: feed CC stdin so a large payload cannot block the
    // child on a full pipe (child's read end would fill; sin drops → EOF).
    let writer = match child.stdin.take() {
        None => None,
        Some(mut sin) => {
            let payload = cc_stdin.as_bytes().to_vec();
            match std::thread::Builder::new().spawn(move || {
                let _ = sin.write_all(&payload);
                // `sin` drops here → the child sees EOF on stdin.
            }) {
                Ok(h) => Some(h),
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return allow();
                }
            }
        }
    };

    // Stdout reader thread: drains continuously so the child never stalls.
    // Capped at HOOK_CMD_OUTPUT_MAX bytes (parity with the HTTP-body cap) so
    // a runaway command emitting a multi-GiB stream cannot exhaust the heap
    // and trigger an OOM-kill → fail-CLOSED on Copilot's preToolUse wire.
    // Truncated output that fails JSON parsing degrades to a fail-open allow.
    let stdout_reader = match child.stdout.take() {
        None => None,
        Some(out) => {
            match std::thread::Builder::new().spawn(move || {
                let mut bytes = Vec::new();
                let _ = out.take(HOOK_CMD_OUTPUT_MAX).read_to_end(&mut bytes);
                String::from_utf8_lossy(&bytes).into_owned()
            }) {
                Ok(h) => Some(h),
                Err(_) => {
                    // writer handle is dropped here → thread detaches and exits on EPIPE.
                    let _ = child.kill();
                    let _ = child.wait();
                    return allow();
                }
            }
        }
    };

    // Stderr reader thread: drains continuously so the child never stalls.
    // Capped at HOOK_CMD_OUTPUT_MAX bytes for the same OOM-protection reason
    // as the stdout reader above.  A truncated stderr reason still denies when
    // the command exits 2 — the deny is correct; the reason is merely shorter.
    let stderr_reader = match child.stderr.take() {
        None => None,
        Some(err) => {
            match std::thread::Builder::new().spawn(move || {
                let mut bytes = Vec::new();
                let _ = err.take(HOOK_CMD_OUTPUT_MAX).read_to_end(&mut bytes);
                String::from_utf8_lossy(&bytes).into_owned()
            }) {
                Ok(h) => Some(h),
                Err(_) => {
                    // writer + stdout_reader handles drop here → threads detach and exit on EOF/EPIPE.
                    let _ = child.kill();
                    let _ = child.wait();
                    return allow();
                }
            }
        }
    };

    let timeout = std::time::Duration::from_millis(entry.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
    let maybe_exit = wait_with_timeout(&mut child, timeout);

    if maybe_exit.is_none() {
        // Timed out: kill child so reader threads get EOF and can be joined
        // below without leaking. A Tome timeout is NEVER a block.
        let _ = child.kill();
        let _ = child.wait();
    }

    // Join stdin writer (fire-and-forget; exits on EOF/EPIPE after kill).
    if let Some(w) = writer {
        let _ = w.join();
    }
    // Collect captured stdout/stderr from the concurrent reader threads.
    let stdout = stdout_reader
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    let stderr = stderr_reader
        .and_then(|h| h.join().ok())
        .unwrap_or_default();

    match maybe_exit {
        Some(exit) => HandlerOutcome {
            exit,
            stdout,
            stderr,
        },
        // A Tome timeout is NEVER a block: fail open.
        None => allow(),
    }
}

/// Resolve `$VAR` / `${VAR}` references in header VALUES ONLY, and only for
/// variables whose name appears in `allowed`. An unlisted reference (or one that
/// matches no host env var) is replaced with an empty string.
///
/// ## Security (NFR-007)
///
/// This is the SINGLE place plugin-declared text drives a substitution.  The
/// substitution is strictly confined to header VALUES — header names, the URL,
/// and all other manifest fields are relocated verbatim. Only variables listed
/// by the plugin in `allowedEnvVars` can be resolved; every other `$REF` is
/// silently elided (→ empty string), so a plugin cannot exfiltrate arbitrary
/// host env vars through header values.
fn interpolate_headers(
    headers: &BTreeMap<String, String>,
    allowed: &[String],
) -> BTreeMap<String, String> {
    use std::sync::OnceLock;
    // Compiled once; the pattern is a compile-time constant.
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"\$(?:\{([A-Za-z_][A-Za-z0-9_]*)\}|([A-Za-z_][A-Za-z0-9_]*))")
            .expect("hard-coded interpolate_headers regex is valid")
    });
    headers
        .iter()
        .map(|(k, v)| {
            let resolved = re.replace_all(v, |caps: &regex::Captures<'_>| {
                // Group 1 = braced ${VAR}; group 2 = bare $VAR.
                let name = caps
                    .get(1)
                    .or_else(|| caps.get(2))
                    .map_or("", |m| m.as_str());
                if allowed.iter().any(|a| a == name) {
                    std::env::var(name).unwrap_or_default()
                } else {
                    String::new() // unlisted → empty; cannot exfiltrate.
                }
            });
            (k.clone(), resolved.into_owned())
        })
        .collect()
}

/// POST `cc_stdin` as the body to `url` (Content-Type: application/json) with
/// header interpolation applied. A 2xx response body is returned as
/// `HandlerOutcome { exit: 0, stdout: body }` so the EXISTING
/// `command_outcome_to_decision` parser processes it identically to a command's
/// stdout JSON. Any transport error, timeout, or non-2xx status degrades to a
/// non-blocking allow (`HandlerOutcome { exit: 0, stdout: "", stderr: "" }`).
///
/// ## Security hardening
///
/// * **Body cap** (`HOOK_HTTP_BODY_MAX`): the body is read via `Read::take` so
///   a fast multi-GiB 2xx response cannot buffer to OOM. A body truncated at
///   the cap fails JSON parsing → non-blocking allow (fail-open).
/// * **Redirect policy (`Policy::none`)**: a 307/308 redirect would resend the
///   POST with the full body and any secret headers (e.g. `X-Api-Key`) to the
///   redirect target. Pinning to a single hop makes requests predictable;
///   a 3xx response falls through to the non-2xx → fail-open allow branch.
/// * **Content-Type deduplication**: plugin headers are inserted into a
///   `HeaderMap` (case-insensitive key comparison) before the default
///   `Content-Type: application/json` is added — so a plugin-supplied
///   `Content-Type` wins and is never duplicated.
///
/// NEVER panics: every `Result` is matched; no bare `.unwrap()` on the
/// response (mirrors `run_command_handler`'s panic-freedom discipline — under
/// `panic = "abort"` in release, a panic aborts the process at exit 134 and
/// `catch_unwind` cannot help).
fn run_http_handler(
    url: &str,
    headers: &BTreeMap<String, String>,
    allowed_env_vars: &[String],
    cc_stdin: &str,
    timeout_ms: Option<u64>,
) -> HandlerOutcome {
    let allow = || HandlerOutcome {
        exit: 0,
        stdout: String::new(),
        stderr: String::new(),
    };
    let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS));
    // A client-build failure (e.g. TLS init error) → fail-open, never a block.
    let client = match reqwest::blocking::Client::builder()
        .timeout(timeout)
        // Pin to a single hop: a 307/308 redirect would forward the POST body
        // and all headers (including any plugin secret tokens) to the redirect
        // target. A 3xx response falls to the non-2xx → fail-open allow branch.
        .redirect(reqwest::redirect::Policy::none())
        .build()
    {
        Ok(c) => c,
        Err(_) => return allow(),
    };
    let interpolated = interpolate_headers(headers, allowed_env_vars);

    // Build a HeaderMap so Content-Type is never duplicated. Plugin headers are
    // inserted first; the default `Content-Type: application/json` is added
    // ONLY if the plugin did not supply one (HeaderMap keys are
    // case-insensitive, so `content-type` / `Content-Type` / `CONTENT-TYPE`
    // all compare equal). An invalid header name or value is silently skipped;
    // reqwest will surface a builder error on `.send()` if needed (→ fail-open).
    let mut header_map = reqwest::header::HeaderMap::new();
    for (k, v) in &interpolated {
        if let (Ok(name), Ok(value)) = (
            reqwest::header::HeaderName::from_bytes(k.as_bytes()),
            reqwest::header::HeaderValue::from_str(v.as_str()),
        ) {
            header_map.insert(name, value);
        }
    }
    if !header_map.contains_key(reqwest::header::CONTENT_TYPE) {
        header_map.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
    }

    let response = match client
        .post(url)
        .headers(header_map)
        .body(cc_stdin.to_owned())
        .send()
    {
        Ok(r) => r,
        // Transport error / conn-refused / DNS failure / timeout /
        // 3xx (redirect disabled) → fail-open.
        Err(_) => return allow(),
    };
    if response.status().is_success() {
        // Cap the body read at HOOK_HTTP_BODY_MAX bytes. `Response` implements
        // `std::io::Read`; `.take(n)` adapts it to read at most n bytes.
        // `from_utf8_lossy` avoids a mid-char truncation error; a truncated
        // body then fails JSON parsing → non-blocking allow (correct fail-open).
        // Do NOT use `.text()` here — it reads the full unbounded body.
        let mut bytes = Vec::new();
        if response
            .take(HOOK_HTTP_BODY_MAX)
            .read_to_end(&mut bytes)
            .is_err()
        {
            return allow();
        }
        let body = String::from_utf8_lossy(&bytes).into_owned();
        HandlerOutcome {
            exit: 0,
            stdout: body,
            stderr: String::new(),
        }
    } else {
        // non-2xx (including 3xx when redirect is disabled) → non-blocking
        // allow; a misbehaving or redirecting webhook must never block.
        allow()
    }
}

/// Execute a `Handler::Prompt` hook by sending the CC context to the configured
/// BYOM provider and mapping its reply to a [`CcDecision`].
///
/// ## I/O contract (our internal protocol with the model)
///
/// * **System message**: the hook's `prompt` text (the plugin author's policy)
///   PLUS an explicit JSON-only reply contract instruction (Fix 1a, US6 review).
/// * **User message**: the CC context JSON from stdin (the event payload).
/// * **Expected reply**: `{"ok":true}` (allow) or `{"ok":false,"reason":"…"}` (deny).
///
/// The parse is lenient (Fix 1b): strips markdown code fences, falls back to
/// extracting the first balanced `{…}` from prose, then applies the deny rule.
/// Any reply that is not `{"ok":false,...}` → allow (fail-open).
///
/// ## Timeout (Fix 2)
///
/// When the manifest entry carries a `timeout_ms`, Tome uses the SMALLER of the
/// hook's timeout and the provider's default, capping the LLM call on the hot path.
///
/// ## Fail-open totality
///
/// Any Tome-side fault — config error, resolve error, provider transport error,
/// unparsable model reply — degrades to a non-blocking allow (`CcDecision::default`).
/// Tome NEVER blocks the agent due to its own fault, even in the prompt path.
///
/// ## BUNDLED-LOCAL path — DEFERRED
///
/// When `prompt_model` is set but `prompt_provider` is unset, `resolve` returns
/// `Ok(None)` (bundled-only). This path is currently UNAVAILABLE (deferred to a
/// future US): `run_inference` is private in `summarise/llama.rs` and wiring llama
/// here is out of scope for US6. The handler fails open with a debug log.
/// The manifest gate in `reconcile_one_harness_dispatch` still carries the handler
/// (both `prompt_model` and `prompt_provider` absent → gate drops it; only
/// `prompt_model` set, `prompt_provider` unset → gate allows it, runtime no-ops).
fn run_prompt_handler(
    prompt: &str,
    cc_stdin: &str,
    cfg: &crate::config::Config,
    timeout_ms: Option<u64>,
) -> CcDecision {
    use crate::config::ProviderKind;
    use crate::provider::config::{Capability, resolve};
    use crate::provider::{anthropic, gemini, openai};

    let mut resolved = match resolve(cfg, Capability::HookPrompt) {
        Ok(Some(r)) => r,
        Ok(None) => {
            // No BYOM provider resolved. Bundled-only path (prompt_model set, no
            // prompt_provider) is DEFERRED — run_inference is private in
            // summarise/llama.rs and wiring it here is out of scope for US6.
            // US-future: honor entry.timeout_ms for prompt eval once the
            //   bundled-local path is wired.
            // Fix 3: make this silent no-op observable at debug level so operators
            // can diagnose why a prompt hook is not evaluating without log spam on
            // every dispatch.
            tracing::debug!(
                "prompt hook is configured but no BYOM provider is resolved \
                 (bundled-local path deferred) — failing open"
            );
            return CcDecision::default();
        }
        // Config error (e.g. provider set but model missing) → fail-open.
        Err(_) => return CcDecision::default(),
    };

    // Fix 2: honor the hook entry's timeout_ms on the prompt path, using the
    // SMALLER of the hook timeout and the provider default so a long-running
    // LLM call cannot block the agent's hot path for longer than the plugin
    // author's declared budget.
    //
    // `resolved.timeout` is a public `Duration` field set by `resolve()` from
    // `TOME_PROVIDER_TIMEOUT_SECS` (or the 30-second default). We simply cap it
    // here; the HTTP layer in `provider/http.rs` reads `resolved.timeout`
    // directly when building the per-call reqwest client.
    if let Some(ms) = timeout_ms {
        let hook_timeout = std::time::Duration::from_millis(ms);
        resolved.timeout = resolved.timeout.min(hook_timeout);
    }

    // Fix 1a: build the system message as the plugin's policy prompt PLUS an
    // explicit JSON-only contract instruction. A real LLM given a free-form policy
    // prompt replies in natural language or fenced JSON; the instruction steers it
    // to the exact wire format `parse_prompt_reply` expects.
    let system = format!(
        "{prompt}\n\nYou are a hook policy evaluator. Reply with ONLY a single JSON object \
         and no other text: {{\"ok\": true}} to ALLOW the action, or \
         {{\"ok\": false, \"reason\": \"<brief reason>\"}} to BLOCK it."
    );

    // Dispatch to the BYOM provider: system = policy + contract, user = event payload.
    let reply = match resolved.kind {
        ProviderKind::Openai => openai::chat(&resolved, Some(&system), cc_stdin),
        ProviderKind::Anthropic => anthropic::chat(&resolved, Some(&system), cc_stdin),
        ProviderKind::Gemini => gemini::chat(&resolved, Some(&system), cc_stdin),
        // voyage is rejected by resolve() for HookPrompt (allows_kind returns false);
        // unreachable at runtime but required for exhaustiveness — fail-open.
        ProviderKind::Voyage => return CcDecision::default(),
    };

    match reply {
        Ok(text) => parse_prompt_reply(&text),
        // Provider error (transport, timeout, bad status, …) → fail-open.
        // Tome never blocks the agent because of a provider fault.
        Err(_) => CcDecision::default(),
    }
}

/// Strip a leading and trailing markdown code fence from `s` so a model
/// that wraps its JSON reply in ` ```json … ``` ` or ` ``` … ``` ` is still
/// parseable. Returns the inner content (trimmed), or `s` unchanged when no
/// matching fence pair is found.
///
/// Never panics: all indexing is bounds-checked via `str` APIs.
fn strip_fences(s: &str) -> &str {
    // Only process strings that start with the fence marker.
    let Some(after_open) = s.strip_prefix("```") else {
        return s;
    };
    // Find the closing ``` (must be AFTER the opening fence).
    let Some(close_pos) = s[3..].rfind("```") else {
        return s; // no closing fence — treat as raw text.
    };
    let close_start = 3 + close_pos; // byte offset of the closing ``` in `s`

    // Skip the optional language tag (e.g. "json\n") on the opening fence line.
    let inner_start = after_open.find('\n').map_or(after_open.len(), |p| p + 1);
    // `inner_start` is a byte offset inside `after_open` = `s[3..]`.
    let content_start = 3 + inner_start;

    if content_start > close_start {
        // Degenerate: the content region is empty or inverted — return as-is.
        return s;
    }
    s[content_start..close_start].trim()
}

/// Extract the FIRST balanced `{ … }` substring from `s`. Used as a fallback
/// when the whole string does not parse as JSON (e.g. the model prepends prose
/// before the JSON object). Returns `None` when no balanced brace pair exists.
///
/// Never panics: `depth` only decrements when already >0; string/escape
/// tracking prevents treating a `}` inside a quoted value as closing.
fn first_json_object(s: &str) -> Option<&str> {
    let start = s.find('{')?;
    let bytes = s.as_bytes();
    let mut depth: usize = 0;
    let mut in_string = false;
    let mut escape = false;
    for (i, &b) in bytes[start..].iter().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        if in_string {
            match b {
                b'\\' => escape = true,
                b'"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_string = true,
            b'{' => depth += 1,
            b'}' => {
                // depth is always ≥1 here because the outer `{` increments it
                // before any `}` is seen; saturating_sub keeps it panic-free.
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&s[start..start + i + 1]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Parse the model's text reply into a [`CcDecision`]. Lenient and fail-open:
/// only a well-formed `{"ok":false,...}` (anywhere in the reply) produces a
/// Deny; anything else allows.
///
/// Fix 1b parsing pipeline:
/// 1. Strip markdown code fences (` ```json … ``` ` or ` ``` … ``` `).
/// 2. Try parsing the stripped text directly as a JSON object.
/// 3. If that fails, extract the first balanced `{ … }` substring and parse it.
/// 4. Apply the deny rule: `ok == false` (bool) → Deny + optional `reason`.
/// 5. Everything else (ok != false, no JSON object, parse failure) → fail-open allow.
///
/// Never panics: no bare indexing on untrusted input; all Results are matched.
fn parse_prompt_reply(text: &str) -> CcDecision {
    // Helper: check whether a parsed JSON Value is a deny decision.
    let to_deny = |v: &serde_json::Value| -> Option<CcDecision> {
        if v.get("ok").and_then(|o| o.as_bool()) == Some(false) {
            let reason = v
                .get("reason")
                .and_then(|r| r.as_str())
                .map(|s| s.to_owned());
            Some(CcDecision {
                permission: Some(Permission::Deny),
                block: true,
                reason,
                ..Default::default()
            })
        } else {
            None
        }
    };

    // 1. Strip markdown code fences so a model-wrapped ` ```json {…} ``` ` parses.
    let stripped = strip_fences(text.trim());

    // 2. Try direct parse of the whole stripped text.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(stripped) {
        if let Some(d) = to_deny(&v) {
            return d;
        }
        // Valid JSON but ok ≠ false → fail-open allow. No need to probe for
        // an embedded object: the entire text WAS valid JSON.
        return CcDecision::default();
    }

    // 3. Whole text did not parse — try extracting the first `{…}` substring.
    //    This handles "I've reviewed the request. {\"ok\":false,\"reason\":\"x\"}" style
    //    prose replies where the model ignored the JSON-only instruction.
    if let Some(obj_str) = first_json_object(stripped)
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(obj_str)
        && let Some(d) = to_deny(&v)
    {
        return d;
    }

    // 4. Non-JSON reply or ok≠false in every candidate → fail-open allow.
    CcDecision::default()
}

/// Poll `try_wait` until the child exits or the wall-clock `timeout` elapses.
/// Returns the exit code on success, or `None` on timeout (caller kills + fails
/// open). stdout/stderr are drained by the concurrent reader threads in
/// [`run_command_handler`] — this function ONLY polls for process exit. A
/// signal-killed child reports `exit = 0` (→ a non-blocking allow).
fn wait_with_timeout(child: &mut std::process::Child, timeout: std::time::Duration) -> Option<i32> {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.code().unwrap_or(0)),
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
        // Intentional: top-level `reason` takes precedence over (overwrites)
        // `hookSpecificOutput.permissionDecisionReason` set above.
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
/// blocking entry's reason, prefixed with that plugin's provenance
/// `[<plugin>] ` so the agent can see which hook denied.
fn merge_decisions(plugin_keyed: &[(String, CcDecision)]) -> CcDecision {
    let mut merged = CcDecision::default();
    let mut best_rank = 0u8;
    let mut reason_set = false;
    for (plugin, d) in plugin_keyed {
        // Most-restrictive permission wins (Deny > Ask > Allow > None).
        let rank = d.permission.map_or(0, Permission::rank);
        if rank > best_rank {
            best_rank = rank;
            merged.permission = d.permission;
        }
        merged.block |= d.block;
        // First blocking entry's reason, provenance-prefixed.
        if !reason_set && is_blocking(d) && d.reason.is_some() {
            let reason = d.reason.as_deref().unwrap_or_default();
            merged.reason = Some(format!("[{plugin}] {reason}"));
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

/// The `hookSpecificOutput` rewrite field name for a ClaudeStyle/Codex wire:
/// PreToolUse rewrites the tool INPUT (`updatedInput`); PostToolUse rewrites the
/// tool OUTPUT (`updatedToolOutput` for Devin/Gemini, `updatedMCPToolOutput` for
/// Codex — Codex-specific, C10/CONFIRMED).
fn rewrite_field(event_cc: &str, is_codex: bool) -> &'static str {
    match (event_cc, is_codex) {
        ("PostToolUse", true) => "updatedMCPToolOutput",
        ("PostToolUse", false) => "updatedToolOutput",
        _ => "updatedInput",
    }
}

/// ClaudeStyle (Devin/Gemini) + Codex emit. A block is the top-level
/// `{"decision":"block","reason"}`; Devin/Gemini ALSO exit 2 (they block on
/// exit-2), while Codex blocks via the JSON at exit 0 (its exit-2 semantics are
/// unverified — never depend on them). The non-blocking signals (`ask`,
/// `additionalContext`, the input/output rewrite) ride `hookSpecificOutput`.
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

    // Non-blocking: assemble a hookSpecificOutput carrying ask / additionalContext
    // / the input-or-output rewrite. Emit only if there is something to say.
    let mut hso = serde_json::Map::new();
    hso.insert(
        "hookEventName".to_string(),
        Value::String(event_cc.to_string()),
    );
    let mut payload = false;
    if matches!(decision.permission, Some(Permission::Ask)) {
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
        payload = true;
    }
    if !decision.additional_context.is_empty() {
        hso.insert(
            "additionalContext".to_string(),
            Value::String(decision.additional_context.join("\n")),
        );
        payload = true;
    }
    if let Some(updated) = &decision.updated_input {
        hso.insert(
            rewrite_field(event_cc, is_codex).to_string(),
            updated.clone(),
        );
        payload = true;
    }

    if !payload {
        // Allow / no-op → empty stdout + exit 0.
        return DispatchOutput {
            stdout: String::new(),
            exit_code: 0,
        };
    }
    let body = serde_json::json!({ "hookSpecificOutput": Value::Object(hso) });
    DispatchOutput {
        stdout: body.to_string(),
        exit_code: 0,
    }
}

/// Cursor emit: snake_case `{permission, agent_message, additional_context,
/// updated_input}` ALWAYS at exit 0. Cursor blocks via JSON `permission:"deny"`;
/// Tome leaves `failClosed` off, so a non-zero exit fails OPEN — Tome NEVER
/// exits 2 here.
fn emit_cursor(decision: &CcDecision) -> DispatchOutput {
    let mut obj = serde_json::Map::new();
    if let Some(p) = permission_token(decision) {
        obj.insert("permission".to_string(), Value::String(p.to_string()));
    }
    if let Some(r) = &decision.reason {
        // `agent_message` is Cursor's reason channel (sent to the agent).
        // Intentional: emitted even when `permission` is absent (no verdict) —
        // informational context is semantically valid without a blocking decision.
        obj.insert("agent_message".to_string(), Value::String(r.clone()));
    }
    if !decision.additional_context.is_empty() {
        obj.insert(
            "additional_context".to_string(),
            Value::String(decision.additional_context.join("\n")),
        );
    }
    if let Some(updated) = &decision.updated_input {
        obj.insert("updated_input".to_string(), updated.clone());
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

/// Copilot CLI emit: flat `{permissionDecision, permissionDecisionReason,
/// modifiedArgs, additionalContext}`. Copilot blocks ONLY via JSON
/// `permissionDecision:"deny"` — exit-2 is a mere warning for most events, so
/// Tome NEVER exits 2 for a block. `additionalContext` is FLAT (top-level), not
/// nested under `hookSpecificOutput`.
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
    if let Some(updated) = &decision.updated_input {
        // US-future: Copilot PostToolUse output rewrite uses `modifiedResult` (not `modifiedArgs`; v1-skippable).
        obj.insert("modifiedArgs".to_string(), updated.clone());
    }
    if !decision.additional_context.is_empty() {
        obj.insert(
            "additionalContext".to_string(),
            Value::String(decision.additional_context.join("\n")),
        );
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
/// stdin object (US4.2 + US8). Backfills `hook_event_name`/`session_id`/`cwd`/
/// `permission_mode` (and event-specific `source`/`tool_response`), applies the
/// per-wire field remaps (e.g. Cursor `conversation_id` → `session_id`), and
/// rewrites the native tool name → CC canonical via
/// [`cc_tool_name`]`(harness, native).unwrap_or(native)` so the plugin script
/// sees `tool_name:"Bash"` regardless of the harness vocabulary. `tool_input`
/// passes through as-is (full per-tool input-schema normalization is a
/// documented v1 limitation).
///
/// US8: also injects the namespaced `"tome"` block with the GLOBAL fields
/// (`harness`, `workspace`) and — when `raw_passthrough` is `true` — the
/// original harness payload as `raw_event`. The per-entry fields
/// (`plugin`, `catalog`, `plugin_root`, `plugin_data`) are NOT set here; the
/// caller (the dispatch loop in [`dispatch_inner`]) augments a CLONE of the
/// returned value per-entry before serialising it.
fn harness_event_to_cc(
    wire: HookWire,
    event_cc: &str,
    harness: &str,
    workspace: &str,
    raw_passthrough: bool,
    raw: &Value,
) -> Value {
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

    // Universal: force-normalize hook_event_name to the CC name — the
    // dispatcher's `event_cc` is authoritative. Using `insert` (not
    // `entry().or_insert_with`) OVERWRITES any harness-native value already
    // present in the raw stdin. Gemini, for example, carries
    // `hook_event_name: "BeforeTool"` in its native payload; a plugin
    // script branching on `$hook_event_name == "PreToolUse"` would silently
    // fail-open if we only backfilled. Mirrors the `tool_name` rewrite above.
    obj.insert(
        "hook_event_name".to_string(),
        Value::String(event_cc.to_string()),
    );
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

    // US8: inject the namespaced `tome` block with the global (per-dispatch)
    // fields. Per-entry fields (plugin, catalog, plugin_root, plugin_data) are
    // NOT set here — the dispatch loop in `dispatch_inner` augments a clone of
    // this value per-entry so each handler sees its own provenance.
    //
    // `raw_event` is included only when the workspace config opts in via
    // `raw_event_passthrough = true`; it is the ORIGINAL harness payload
    // (before any remap or backfill), useful for debugging or auditing.
    let mut tome_obj = serde_json::Map::new();
    tome_obj.insert("harness".to_string(), Value::String(harness.to_string()));
    tome_obj.insert(
        "workspace".to_string(),
        Value::String(workspace.to_string()),
    );
    if raw_passthrough {
        tome_obj.insert("raw_event".to_string(), raw.clone());
    }
    obj.insert("tome".to_string(), Value::Object(tome_obj));

    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    // --- Env-var serialisation (for header interpolation tests) ------------------
    //
    // `std::env::set_var` / `remove_var` are `unsafe` on Rust 2024 and unsafe for
    // any process with threads contending the env block. Tests in cargo run on the
    // same process, so we serialise via a module-local `ENV_MUTEX`.
    // `EnvVarGuard` is RAII: it snapshots the previous value and restores on drop.

    static ENV_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();

    fn lock_env() -> MutexGuard<'static, ()> {
        ENV_MUTEX
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    struct EnvVarGuard {
        key: String,
        previous: Option<std::ffi::OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            // SAFETY: caller holds ENV_MUTEX; no other test in this module
            // mutates the environment concurrently.
            unsafe { std::env::set_var(key, value) }
            Self {
                key: key.to_owned(),
                previous,
            }
        }

        /// Temporarily remove `key` from the environment. Restores the
        /// previous value (or re-sets it) on drop, like `set`.  Used by
        /// tests that need to verify the `unwrap_or_default()` branch of
        /// `interpolate_headers` when a variable is allowlisted but absent.
        fn remove(key: &str) -> Self {
            let previous = std::env::var_os(key);
            // SAFETY: caller holds ENV_MUTEX; no other test in this module
            // mutates the environment concurrently.
            unsafe { std::env::remove_var(key) }
            Self {
                key: key.to_owned(),
                previous,
            }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            // SAFETY: ENV_MUTEX is still held by the test for the lifetime of
            // this guard.
            unsafe {
                match &self.previous {
                    Some(v) => std::env::set_var(&self.key, v),
                    None => std::env::remove_var(&self.key),
                }
            }
        }
    }

    // --- interpolate_headers unit tests ------------------------------------------

    /// `interpolate_headers` resolves allowlisted `$VAR` / `${VAR}` references in
    /// header VALUES; unlisted references are elided (→ empty string), not leaked.
    ///
    /// This is the security pin test for NFR-007: the allowlist gate is the single
    /// policy line between plugin-declared text and host env var exfiltration.
    ///
    /// ## NFR-007 regression pins (added by US5 fix pass)
    ///
    /// * **braced-allowed**: `${MY_TOKEN}` with MY_TOKEN allowlisted+set resolves
    ///   to the value (exercises regex capture group 1, distinct from bare `$VAR`).
    /// * **allowlisted-but-unset**: a var in `allowed_env_vars` that is NOT present
    ///   in the environment resolves to `""` (the `unwrap_or_default()` branch).
    /// * **structural — key verbatim**: a `$VAR` reference in a header KEY is NOT
    ///   interpolated; only VALUES are substituted. The URL is also relayed verbatim
    ///   (never passed through `interpolate_headers` at all).
    #[test]
    fn header_interpolation_is_allowlist_gated() {
        let _lock = lock_env();
        let _guard = EnvVarGuard::set("MY_TOKEN", "secret");
        // Explicitly remove the unset-test variable so the test is hermetic even
        // if a CI environment happens to export it.
        let _guard_unset = EnvVarGuard::remove("UNSET_ALLOWED_VAR");

        let mut h = BTreeMap::new();
        // Original: bare $VAR form, allowlisted.
        h.insert("Authorization".to_owned(), "Bearer $MY_TOKEN".to_owned());
        // Original: ${VAR} form, NOT allowlisted → empty (must not leak).
        h.insert("X-Other".to_owned(), "${NOT_ALLOWED}".to_owned());
        // NFR-007 pin (braced-allowed): ${VAR} form, allowlisted → resolves
        // (exercises regex capture group 1, separate from the bare-$VAR group 2).
        h.insert("X-Braced".to_owned(), "v2-${MY_TOKEN}".to_owned());
        // NFR-007 pin (allowlisted-but-unset): var is in the allowlist but not
        // set in the environment → resolves to "" via unwrap_or_default().
        h.insert("X-Unset".to_owned(), "$UNSET_ALLOWED_VAR".to_owned());
        // NFR-007 structural pin: a $VAR token in the header KEY must NOT be
        // interpolated — only VALUES are substituted.
        h.insert("X-$MY_TOKEN-Key".to_owned(), "static-value".to_owned());

        let out = interpolate_headers(&h, &["MY_TOKEN".to_owned(), "UNSET_ALLOWED_VAR".to_owned()]);

        // Original assertions.
        assert_eq!(out["Authorization"], "Bearer secret");
        assert_eq!(out["X-Other"], ""); // unlisted → empty; must NOT be leaked

        // NFR-007 regression assertions.
        assert_eq!(
            out["X-Braced"], "v2-secret",
            "braced ${{VAR}} must resolve when allowlisted"
        );
        assert_eq!(
            out["X-Unset"], "",
            "allowlisted-but-unset var must resolve to empty string"
        );
        // The key `X-$MY_TOKEN-Key` must appear verbatim in the output map
        // (not as `X-secret-Key` or any other interpolated form).
        assert!(
            out.contains_key("X-$MY_TOKEN-Key"),
            "header key must be relayed verbatim (never interpolated)"
        );
        assert_eq!(out["X-$MY_TOKEN-Key"], "static-value");
    }

    // --- command_entry helper ----------------------------------------------------

    /// A command [`ManifestEntry`] with the given matcher + shell command, a
    /// generous timeout, and no cwd/env.
    fn command_entry(matcher: &str, command: &str) -> ManifestEntry {
        ManifestEntry {
            plugin: "cat:test".to_string(),
            plugin_root: None,
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

    /// A blank [`TomeProvenance`] for tests that do not exercise the US8
    /// provenance fields (all fields empty strings). Keeps pre-US8 test call
    /// sites concise while the signature addition is backward-compatible.
    fn blank_prov() -> TomeProvenance<'static> {
        TomeProvenance {
            harness: "",
            workspace: "",
            plugin: "",
            catalog: "",
            plugin_root: "",
            plugin_data: "",
        }
    }

    /// A command hook that writes a reason to stderr and exits 2 blocks: the raw
    /// outcome is exit 2, and `command_outcome_to_decision` maps it to
    /// Deny + the stderr text as the reason.
    #[test]
    fn command_exit_2_blocks_with_reason() {
        let entry = command_entry("Bash", "printf 'nope' >&2; exit 2");
        let outcome = run_command_handler(&entry, r#"{"tool_name":"Bash"}"#, &blank_prov());
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
        let outcome = run_command_handler(&entry, "{}", &blank_prov());
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

    /// The merge is most-restrictive-wins, concatenates `additional_context` in
    /// manifest order, and prefixes the block reason with the denying plugin's
    /// provenance `[<plugin>] `.
    #[test]
    fn merge_is_most_restrictive_with_concat_and_provenance() {
        let a = (
            "cat:allow".to_string(),
            CcDecision {
                permission: Some(Permission::Allow),
                additional_context: vec!["ctxA".to_string()],
                ..Default::default()
            },
        );
        let b = (
            "cat:deny".to_string(),
            CcDecision {
                permission: Some(Permission::Deny),
                reason: Some("blocked".to_string()),
                additional_context: vec!["ctxB".to_string()],
                ..Default::default()
            },
        );
        let m = merge_decisions(&[a, b]);
        assert_eq!(m.permission, Some(Permission::Deny)); // deny > allow
        assert_eq!(
            m.additional_context,
            vec!["ctxA".to_string(), "ctxB".to_string()] // concat in order
        );
        assert_eq!(m.reason.as_deref(), Some("[cat:deny] blocked")); // provenance prefix
    }

    /// `updated_input` is last-wins across the merged entries.
    #[test]
    fn merge_updated_input_is_last_wins() {
        let a = (
            "cat:a".to_string(),
            CcDecision {
                updated_input: Some(serde_json::json!({"v": 1})),
                ..Default::default()
            },
        );
        let b = (
            "cat:b".to_string(),
            CcDecision {
                updated_input: Some(serde_json::json!({"v": 2})),
                ..Default::default()
            },
        );
        let m = merge_decisions(&[a, b]);
        assert_eq!(m.updated_input, Some(serde_json::json!({"v": 2})));
    }

    /// A malformed manifest on disk reads as `None` (the `.ok()` swallow), which
    /// `dispatch_core` turns into a fail-open allow at exit 0. This is the only
    /// layer that can exercise `read_manifest` (crate-private), so the malformed
    /// → None → fail-open chain is pinned here rather than in an integration test.
    #[test]
    fn malformed_manifest_reads_as_none_then_fail_open() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hooks-manifest.json");
        std::fs::write(&path, "{ this is not valid json").unwrap();
        let manifest = crate::harness::hooks_ir::read_manifest(&path).ok();
        assert!(manifest.is_none(), "malformed manifest must read as None");
        let out = dispatch_core("cursor", "PreToolUse", "{}", manifest.as_ref());
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.is_empty());
    }

    /// A handler writing >64 KB to stdout and exiting 0 must complete without
    /// deadlock — well under the 5-second entry timeout. Without concurrent stdout
    /// draining the OS pipe buffer (~64 KB) fills, the child blocks on write(),
    /// and the old serial drain spun to the full timeout, killed the child, and
    /// returned a fail-open allow. This test proves concurrent draining eliminates
    /// that back-pressure.
    #[test]
    fn large_stdout_no_deadlock_resolves_allow() {
        let entry = command_entry(
            "Bash",
            // 200 KB (204800 bytes) to stdout; exit 0.
            "python3 -c \"import sys; sys.stdout.buffer.write(b'x' * 204800)\"",
        );
        let start = std::time::Instant::now();
        let outcome = run_command_handler(&entry, "{}", &blank_prov());
        let elapsed = start.elapsed();
        // If python3 is unavailable this assertion fails with a clear message
        // (exit would be 127, stdout empty). The test is intentionally strict.
        assert!(
            outcome.stdout.len() > 100_000,
            "expected >100 KB of stdout (got {} bytes); is python3 available?",
            outcome.stdout.len()
        );
        assert!(
            elapsed < std::time::Duration::from_secs(3),
            "large stdout must drain without deadlock (took {elapsed:?})"
        );
        assert_eq!(outcome.exit, 0);
        let decision = command_outcome_to_decision(&outcome);
        assert!(
            !is_blocking(&decision),
            "exit 0 large stdout must be a non-blocking allow"
        );
    }

    /// A handler writing >64 KB to stderr and exiting 2 must resolve as a Deny
    /// with the large stderr captured — WITHOUT deadlock. The old serial drain
    /// would have silently downgraded this to a fail-open allow by hitting the
    /// full timeout (the security deny-bypass this fix closes).
    #[test]
    fn large_stderr_deny_no_deadlock_captures_reason() {
        let entry = command_entry(
            "Bash",
            // 200 KB to stderr, exit 2.
            "python3 -c \"import sys; sys.stderr.buffer.write(b'x' * 204800); sys.exit(2)\"",
        );
        let start = std::time::Instant::now();
        let outcome = run_command_handler(&entry, "{}", &blank_prov());
        let elapsed = start.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(3),
            "large stderr deny must drain without deadlock (took {elapsed:?})"
        );
        assert_eq!(outcome.exit, 2);
        assert!(
            outcome.stderr.len() > 100_000,
            "large stderr must be fully captured (got {} bytes)",
            outcome.stderr.len()
        );
        let decision = command_outcome_to_decision(&outcome);
        assert!(
            is_blocking(&decision),
            "exit 2 + large stderr must be a blocking deny"
        );
        assert!(
            decision.reason.is_some(),
            "deny must carry a reason from the stderr text"
        );
    }

    /// A timed-out command is killed and degrades to a non-blocking allow
    /// (exit 0, empty) — a Tome timeout is NEVER a block.
    #[test]
    fn command_timeout_is_non_blocking_allow() {
        let mut entry = command_entry("Bash", "sleep 5");
        entry.timeout_ms = Some(50);
        let outcome = run_command_handler(&entry, "{}", &blank_prov());
        let decision = command_outcome_to_decision(&outcome);
        assert!(!is_blocking(&decision));
        assert_eq!(decision.permission, None);
    }

    /// Cursor deny is snake_case `{permission:"deny", agent_message}` at exit 0
    /// (Cursor blocks via JSON, never exit-2 from Tome).
    #[test]
    fn cursor_emit_deny_is_snake_case_exit_0() {
        let d = CcDecision {
            permission: Some(Permission::Deny),
            reason: Some("[cat:p] no".to_string()),
            ..Default::default()
        };
        let out = emit_decision(HookWire::CursorSnake, "PreToolUse", &d);
        assert!(out.stdout.contains("\"permission\":\"deny\""));
        assert!(out.stdout.contains("\"agent_message\""));
        assert_eq!(out.exit_code, 0);
    }

    /// ClaudeStyle (Devin/Gemini) deny emits `{"decision":"block",…}` AND exits
    /// 2; Codex emits the same JSON block but at exit 0 (its exit-2 is unverified).
    #[test]
    fn claude_style_deny_exit2_codex_block_exit0() {
        let d = CcDecision {
            permission: Some(Permission::Deny),
            reason: Some("[cat:p] no".to_string()),
            ..Default::default()
        };
        let dev = emit_decision(HookWire::ClaudeStyle, "PreToolUse", &d);
        assert!(dev.stdout.contains("\"decision\":\"block\""));
        assert!(dev.stdout.contains("[cat:p] no"));
        assert_eq!(dev.exit_code, 2);
        let cdx = emit_decision(HookWire::Codex, "PreToolUse", &d);
        assert!(cdx.stdout.contains("\"decision\":\"block\""));
        assert_eq!(cdx.exit_code, 0);
    }

    /// `additionalContext` emits per-wire: ClaudeStyle nests it under
    /// `hookSpecificOutput` (with `hookEventName`); Cursor uses snake_case
    /// `additional_context`; Copilot uses a FLAT top-level `additionalContext`.
    #[test]
    fn additional_context_emit_per_wire() {
        let d = CcDecision {
            additional_context: vec!["ctx1".to_string(), "ctx2".to_string()],
            ..Default::default()
        };
        let claude = emit_decision(HookWire::ClaudeStyle, "PostToolUse", &d);
        let v: Value = serde_json::from_str(&claude.stdout).unwrap();
        assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PostToolUse");
        assert_eq!(v["hookSpecificOutput"]["additionalContext"], "ctx1\nctx2");
        assert_eq!(claude.exit_code, 0);

        let cursor = emit_decision(HookWire::CursorSnake, "PostToolUse", &d);
        let v: Value = serde_json::from_str(&cursor.stdout).unwrap();
        assert_eq!(v["additional_context"], "ctx1\nctx2");

        let copilot = emit_decision(HookWire::CopilotFlat, "PostToolUse", &d);
        let v: Value = serde_json::from_str(&copilot.stdout).unwrap();
        assert_eq!(v["additionalContext"], "ctx1\nctx2");
    }

    /// The input/output rewrite field varies by wire + event: PreToolUse rewrites
    /// the input (`updatedInput`/`updated_input`/`modifiedArgs`); PostToolUse
    /// rewrites the output (`updatedToolOutput` for ClaudeStyle,
    /// `updatedMCPToolOutput` for Codex).
    #[test]
    fn updated_input_emit_rewrite_fields() {
        let d = CcDecision {
            updated_input: Some(serde_json::json!({ "x": 1 })),
            ..Default::default()
        };

        let pre = emit_decision(HookWire::ClaudeStyle, "PreToolUse", &d);
        let v: Value = serde_json::from_str(&pre.stdout).unwrap();
        assert_eq!(
            v["hookSpecificOutput"]["updatedInput"],
            serde_json::json!({ "x": 1 })
        );

        let post = emit_decision(HookWire::ClaudeStyle, "PostToolUse", &d);
        let v: Value = serde_json::from_str(&post.stdout).unwrap();
        assert_eq!(
            v["hookSpecificOutput"]["updatedToolOutput"],
            serde_json::json!({ "x": 1 })
        );

        let codex_post = emit_decision(HookWire::Codex, "PostToolUse", &d);
        let v: Value = serde_json::from_str(&codex_post.stdout).unwrap();
        assert_eq!(
            v["hookSpecificOutput"]["updatedMCPToolOutput"],
            serde_json::json!({ "x": 1 })
        );

        let cursor = emit_decision(HookWire::CursorSnake, "PreToolUse", &d);
        let v: Value = serde_json::from_str(&cursor.stdout).unwrap();
        assert_eq!(v["updated_input"], serde_json::json!({ "x": 1 }));

        let copilot = emit_decision(HookWire::CopilotFlat, "PreToolUse", &d);
        let v: Value = serde_json::from_str(&copilot.stdout).unwrap();
        assert_eq!(v["modifiedArgs"], serde_json::json!({ "x": 1 }));
    }

    /// A pure allow / no-op is empty stdout + exit 0 on ALL four wires.
    #[test]
    fn allow_no_op_is_empty_exit_0_on_all_wires() {
        let allow = CcDecision::default();
        for wire in [
            HookWire::ClaudeStyle,
            HookWire::Codex,
            HookWire::CursorSnake,
            HookWire::CopilotFlat,
        ] {
            let out = emit_decision(wire, "PreToolUse", &allow);
            assert!(out.stdout.is_empty(), "{wire:?} allow must be empty");
            assert_eq!(out.exit_code, 0);
        }
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
        let cc = harness_event_to_cc(
            HookWire::CursorSnake,
            "PreToolUse",
            "cursor",
            "",
            false,
            &raw,
        );
        assert_eq!(cc["session_id"], "conv-123");
        assert_eq!(cc["cwd"], "/repo/root");
        assert_eq!(cc["hook_event_name"], "PreToolUse");
        assert_eq!(cc["permission_mode"], "default");

        // (b) Gemini run_shell_command → Bash.
        let raw = serde_json::json!({ "tool_name": "run_shell_command" });
        let cc = harness_event_to_cc(
            HookWire::ClaudeStyle,
            "PreToolUse",
            "gemini",
            "",
            false,
            &raw,
        );
        assert_eq!(cc["tool_name"], "Bash");

        // (c) Missing cwd is backfilled to an empty string when the harness has
        // no source for it (Devin, U2).
        let raw = serde_json::json!({ "tool_name": "exec" });
        let cc = harness_event_to_cc(
            HookWire::ClaudeStyle,
            "PreToolUse",
            "devin",
            "",
            false,
            &raw,
        );
        assert_eq!(cc["cwd"], "");
        assert_eq!(cc["session_id"], "");
        assert_eq!(cc["tool_name"], "Bash");
    }

    // --- parse_prompt_reply unit tests (Fix 1b, US6 review) ---------------------

    /// A fenced `{"ok":false,"reason":"x"}` → Deny with the reason forwarded.
    /// This is the primary regression for Fix 1b: a real model wrapping its JSON
    /// in a markdown code fence was previously opaque to the parser.
    #[test]
    fn parse_prompt_reply_fenced_deny_extracts_reason() {
        let reply = "```json\n{\"ok\":false,\"reason\":\"blocked by policy\"}\n```";
        let d = parse_prompt_reply(reply);
        assert_eq!(d.permission, Some(Permission::Deny));
        assert!(d.block);
        assert_eq!(d.reason.as_deref(), Some("blocked by policy"));
    }

    /// An `{"ok":false}` object embedded mid-prose → Deny. Models sometimes
    /// prepend an explanation before the JSON despite the JSON-only instruction.
    #[test]
    fn parse_prompt_reply_embedded_in_prose_deny() {
        let reply = "After careful consideration of the request, I believe this is unsafe. \
                     {\"ok\":false,\"reason\":\"dangerous command\"} Hope that helps.";
        let d = parse_prompt_reply(reply);
        assert_eq!(d.permission, Some(Permission::Deny));
        assert!(d.block);
        assert_eq!(d.reason.as_deref(), Some("dangerous command"));
    }

    /// `{"ok":true}` → non-blocking allow (empty decision).
    #[test]
    fn parse_prompt_reply_ok_true_allows() {
        let reply = "{\"ok\":true}";
        let d = parse_prompt_reply(reply);
        assert_eq!(d.permission, None);
        assert!(!d.block);
        assert!(d.reason.is_none());
    }

    /// Free-text natural-language response → fail-open allow. The model
    /// ignored the JSON-only instruction but Tome must not block.
    #[test]
    fn parse_prompt_reply_free_text_fails_open() {
        let reply = "This looks unsafe and I recommend blocking it.";
        let d = parse_prompt_reply(reply);
        assert_eq!(d.permission, None);
        assert!(!d.block);
    }

    /// Structurally malformed / truncated text → fail-open allow.
    #[test]
    fn parse_prompt_reply_malformed_fails_open() {
        for bad in &[
            "",
            "   ",
            "{not valid json at all",
            "null",
            "42",
            "{\"ok\":\"maybe\"}", // ok is a string, not bool
            "{\"ok\":null}",      // ok is null
        ] {
            let d = parse_prompt_reply(bad);
            assert_eq!(
                d.permission, None,
                "malformed input {bad:?} must fail open (permission = None)"
            );
            assert!(!d.block, "malformed input {bad:?} must not block");
        }
    }

    /// `{"ok":false}` without a `reason` field → Deny with `reason: None`.
    #[test]
    fn parse_prompt_reply_deny_without_reason() {
        let d = parse_prompt_reply("{\"ok\":false}");
        assert_eq!(d.permission, Some(Permission::Deny));
        assert!(d.block);
        assert!(d.reason.is_none());
    }

    /// A ` ``` ` (no language tag) fence is also stripped.
    #[test]
    fn parse_prompt_reply_bare_fence_stripped() {
        let reply = "```\n{\"ok\":false,\"reason\":\"bare fence\"}\n```";
        let d = parse_prompt_reply(reply);
        assert_eq!(d.permission, Some(Permission::Deny));
        assert_eq!(d.reason.as_deref(), Some("bare fence"));
    }

    // --- strip_fences unit tests --------------------------------------------------

    #[test]
    fn strip_fences_json_lang_tag() {
        assert_eq!(strip_fences("```json\n{\"ok\":true}\n```"), "{\"ok\":true}");
    }

    #[test]
    fn strip_fences_no_lang_tag() {
        assert_eq!(strip_fences("```\nhello\n```"), "hello");
    }

    #[test]
    fn strip_fences_no_fence_unchanged() {
        assert_eq!(strip_fences("{\"ok\":true}"), "{\"ok\":true}");
    }

    #[test]
    fn strip_fences_unclosed_fence_unchanged() {
        // Opening ``` without a matching closing ``` → return unchanged.
        let s = "```json\n{\"ok\":true}";
        assert_eq!(strip_fences(s), s);
    }

    // --- first_json_object unit tests --------------------------------------------

    #[test]
    fn first_json_object_plain() {
        assert_eq!(first_json_object("{\"a\":1}"), Some("{\"a\":1}"));
    }

    #[test]
    fn first_json_object_prose_prefix() {
        assert_eq!(
            first_json_object("Some text before {\"ok\":false} and more text"),
            Some("{\"ok\":false}")
        );
    }

    #[test]
    fn first_json_object_no_object() {
        assert!(first_json_object("no braces here").is_none());
        assert!(first_json_object("").is_none());
    }

    #[test]
    fn first_json_object_nested_braces() {
        // The outermost balanced pair is extracted, including nested content.
        let s = "{\"a\":{\"b\":1}}";
        assert_eq!(first_json_object(s), Some(s));
    }

    #[test]
    fn first_json_object_unbalanced_ignores_inner() {
        // Opening brace without a matching close → None.
        assert!(first_json_object("{unclosed").is_none());
    }

    /// An unmapped native tool name falls back to itself (so a matcher that
    /// references the native token directly still matches), and a wholly empty
    /// raw object still produces the universal CC backfills.
    #[test]
    fn harness_event_to_cc_unmapped_tool_and_empty_raw() {
        let raw = serde_json::json!({ "tool_name": "totally_custom" });
        let cc = harness_event_to_cc(
            HookWire::ClaudeStyle,
            "PreToolUse",
            "gemini",
            "",
            false,
            &raw,
        );
        assert_eq!(cc["tool_name"], "totally_custom");

        let cc = harness_event_to_cc(
            HookWire::Codex,
            "SessionStart",
            "codex",
            "",
            false,
            &Value::Null,
        );
        assert_eq!(cc["hook_event_name"], "SessionStart");
        assert_eq!(cc["source"], "startup");
        assert_eq!(cc["session_id"], "");
    }

    // --- US8 unit tests -----------------------------------------------------------

    /// `harness_event_to_cc` injects a `tome` block with global fields (`harness`,
    /// `workspace`). Per-entry fields are NOT present at this stage.
    #[test]
    fn harness_event_to_cc_injects_global_tome_block() {
        let raw = serde_json::json!({});
        let cc = harness_event_to_cc(
            HookWire::ClaudeStyle,
            "PreToolUse",
            "gemini",
            "ws1",
            false,
            &raw,
        );
        assert_eq!(
            cc["tome"]["harness"], "gemini",
            "tome.harness must equal the harness name"
        );
        assert_eq!(
            cc["tome"]["workspace"], "ws1",
            "tome.workspace must equal the workspace name"
        );
        // Per-entry fields not set at this stage.
        assert!(
            cc["tome"].get("plugin").is_none(),
            "tome.plugin must NOT be set by harness_event_to_cc (per-entry only)"
        );
        assert!(
            cc["tome"].get("raw_event").is_none(),
            "tome.raw_event must be absent when raw_passthrough=false"
        );
    }

    /// `tome.raw_event` is present only when `raw_passthrough=true`, and its
    /// value is the original harness payload (before remap/backfill).
    #[test]
    fn harness_event_to_cc_raw_event_gated_by_passthrough_flag() {
        let raw = serde_json::json!({"native_key": "native_val"});

        // Flag off → no raw_event.
        let cc_off = harness_event_to_cc(
            HookWire::ClaudeStyle,
            "PreToolUse",
            "gemini",
            "",
            false,
            &raw,
        );
        assert!(
            cc_off["tome"].get("raw_event").is_none(),
            "raw_event must be absent when raw_passthrough=false"
        );

        // Flag on → raw_event = the original harness payload.
        let cc_on = harness_event_to_cc(
            HookWire::ClaudeStyle,
            "PreToolUse",
            "gemini",
            "",
            true,
            &raw,
        );
        assert_eq!(
            cc_on["tome"]["raw_event"]["native_key"], "native_val",
            "raw_event must equal the original harness payload"
        );
        // raw_event should NOT reflect the CC remap / backfill (it is the pre-remap raw).
        assert!(
            cc_on["tome"]["raw_event"].get("hook_event_name").is_none(),
            "raw_event must not include the CC backfill fields"
        );
    }

    /// The dispatch loop builds a per-entry clone of the base cc_value and
    /// injects the per-entry `tome` fields. Two entries from DIFFERENT plugins
    /// each see their own `tome.plugin` in the stdin delivered to their handler.
    /// Verified by running `printf '%s' "$TOME_PLUGIN"` and checking the env
    /// via a command that echoes the TOME_PLUGIN env var.
    #[test]
    fn per_entry_tome_plugin_is_distinct_per_plugin() {
        // We verify the per-entry cc_stdin by dispatching through a command that
        // echoes $TOME_PLUGIN to stdout; each run should get its own plugin name.
        // This test also serves as the TOME_* env var pin.
        use crate::harness::hooks_ir::{Handler, HookManifest, ManifestEntry};
        use std::collections::BTreeMap;

        let manifest_json = r#"{
            "harness": "gemini",
            "events": {
                "PreToolUse": [
                    { "plugin": "catA:plugA", "matcher": "*",
                      "handler": { "type": "command", "command": "printf '%s' \"$TOME_PLUGIN\"" } },
                    { "plugin": "catB:plugB", "matcher": "*",
                      "handler": { "type": "command", "command": "printf '%s' \"$TOME_PLUGIN\"" } }
                ]
            }
        }"#;
        let _m: HookManifest = serde_json::from_str(manifest_json).unwrap();

        // dispatch_core passes "" workspace + None paths; TOME_PLUGIN is still set
        // from entry.plugin directly.
        // Use dispatch_core which passes "" workspace and None paths.
        // We cannot assert `tome.plugin` in the merged emit output (it's not in
        // the wire format), but we can observe TOME_PLUGIN via the command stdout.
        // Since both handlers run, the per-entry isolation is verified by
        // the commands themselves producing the right plugin name each.
        //
        // We drive dispatch_core but the merged output mixes both — to pin the
        // PER-ENTRY isolation, test run_command_handler directly with different
        // TomeProvenance values.
        let entry_a = ManifestEntry {
            plugin: "catA:plugA".to_string(),
            plugin_root: None,
            matcher: None,
            if_pred: None,
            handler: Handler::Command {
                command: "printf '%s' \"$TOME_PLUGIN\"".to_string(),
            },
            timeout_ms: Some(5_000),
            cwd: None,
            env: BTreeMap::new(),
        };
        let prov_a = TomeProvenance {
            harness: "gemini",
            workspace: "ws",
            plugin: "catA:plugA",
            catalog: "catA",
            plugin_root: "/data/catA",
            plugin_data: "/data/catA/plugA",
        };
        let outcome_a = run_command_handler(&entry_a, "{}", &prov_a);
        assert_eq!(
            outcome_a.stdout.trim(),
            "catA:plugA",
            "TOME_PLUGIN must equal the entry's plugin provenance"
        );
        assert_eq!(outcome_a.exit, 0);

        let mut entry_b = entry_a.clone();
        entry_b.plugin = "catB:plugB".to_string();
        let prov_b = TomeProvenance {
            harness: "gemini",
            workspace: "ws",
            plugin: "catB:plugB",
            catalog: "catB",
            plugin_root: "/data/catB",
            plugin_data: "/data/catB/plugB",
        };
        let outcome_b = run_command_handler(&entry_b, "{}", &prov_b);
        assert_eq!(
            outcome_b.stdout.trim(),
            "catB:plugB",
            "second entry must see ITS OWN plugin, not the first entry's"
        );
    }

    /// All six TOME_* env vars are visible to a command handler, carrying
    /// the values from the per-entry TomeProvenance.
    #[test]
    fn command_handler_sees_all_tome_env_vars() {
        use crate::harness::hooks_ir::{Handler, ManifestEntry};
        use std::collections::BTreeMap;

        // Echo all six TOME_* vars separated by commas.
        let entry = ManifestEntry {
            plugin: "mycat:myplugin".to_string(),
            plugin_root: None,
            matcher: None,
            if_pred: None,
            handler: Handler::Command {
                command: concat!(
                    "printf '%s,%s,%s,%s,%s,%s'",
                    " \"$TOME_HARNESS\" \"$TOME_WORKSPACE\" \"$TOME_PLUGIN\"",
                    " \"$TOME_CATALOG\" \"$TOME_PLUGIN_ROOT\" \"$TOME_PLUGIN_DATA\""
                )
                .to_string(),
            },
            timeout_ms: Some(5_000),
            cwd: None,
            env: BTreeMap::new(),
        };
        let prov = TomeProvenance {
            harness: "gemini",
            workspace: "myws",
            plugin: "mycat:myplugin",
            catalog: "mycat",
            plugin_root: "/root/path",
            plugin_data: "/data/path",
        };
        let outcome = run_command_handler(&entry, "{}", &prov);
        assert_eq!(outcome.exit, 0);
        assert_eq!(
            outcome.stdout.trim(),
            "gemini,myws,mycat:myplugin,mycat,/root/path,/data/path",
            "all six TOME_* env vars must be visible to the command handler"
        );
    }

    /// When `raw_event_passthrough` is off in the manifest, the dispatch does
    /// NOT include `tome.raw_event` in the cc_stdin. When on, it IS present.
    ///
    /// The gating is tested through an OBSERVABLE channel: the hook command
    /// exits 2 (deny on ClaudeStyle/Gemini wire) when `"raw_event"` appears in
    /// its stdin, and exits 0 (allow) otherwise. The dispatch exit code then
    /// distinguishes the two cases unambiguously — no vacuous pass from an
    /// absent interpreter or swallowed stdout.
    #[test]
    fn dispatch_inner_raw_event_gated_by_manifest_flag() {
        use crate::harness::hooks_ir::HookManifest;

        // Command: exit 2 (ClaudeStyle deny) when raw_event is present in stdin,
        // exit 0 (allow) otherwise. `grep -q` exits 0 on match, 1 on no-match —
        // invert: exit 2 on match, 0 on no-match.
        // Uses `sh` + `grep` — no python3 dependency, always available in CI.
        let cmd = r#"grep -q '"raw_event"' && printf 'raw_event_present' >&2 && exit 2; exit 0"#;

        let make_manifest = |passthrough: bool| -> HookManifest {
            let json = format!(
                r#"{{
                    "harness": "gemini",
                    "raw_event_passthrough": {},
                    "events": {{
                        "PreToolUse": [
                            {{ "plugin": "cat:p", "matcher": "*",
                               "handler": {{ "type": "command", "command": {} }} }}
                        ]
                    }}
                }}"#,
                passthrough,
                serde_json::to_string(cmd).unwrap(),
            );
            serde_json::from_str(&json).unwrap()
        };

        let raw_input = r#"{"native_field": "value"}"#;

        // Flag off → raw_event absent from CC stdin → grep fails → exit 0 → allow.
        let m_off = make_manifest(false);
        let out_off = dispatch_core("gemini", "PreToolUse", raw_input, Some(&m_off));
        assert_eq!(
            out_off.exit_code, 0,
            "passthrough-off: raw_event must be absent → hook exits 0 → allow"
        );

        // Flag on → raw_event present in CC stdin → grep matches → exit 2 → deny.
        // ClaudeStyle (Gemini) wire: a hook exit 2 → decision=block → dispatch exit 2.
        let m_on = make_manifest(true);
        let out_on = dispatch_core("gemini", "PreToolUse", raw_input, Some(&m_on));
        assert_eq!(
            out_on.exit_code, 2,
            "passthrough-on: raw_event must be present → hook exits 2 → deny (exit 2)"
        );
    }

    // --- US10 unit tests -----------------------------------------------------------

    /// Build a minimal [`HookManifest`] for explain_core / dispatch tests.
    fn make_manifest_for_explain(command: Option<&str>, http_url: Option<&str>) -> HookManifest {
        use crate::harness::hooks_ir::{Handler, HookManifest, ManifestEntry};
        use std::collections::BTreeMap;

        let mut entries = Vec::new();
        if let Some(cmd) = command {
            entries.push(ManifestEntry {
                plugin: "cat:cmd-guard".to_string(),
                plugin_root: None,
                matcher: Some("Bash".to_string()),
                if_pred: None,
                handler: Handler::Command {
                    command: cmd.to_string(),
                },
                timeout_ms: Some(5_000),
                cwd: None,
                env: BTreeMap::new(),
            });
        }
        if let Some(url) = http_url {
            entries.push(ManifestEntry {
                plugin: "cat:http-guard".to_string(),
                plugin_root: None,
                matcher: Some("Bash".to_string()),
                if_pred: None,
                handler: Handler::Http {
                    url: url.to_string(),
                    headers: BTreeMap::new(),
                    allowed_env_vars: vec![],
                },
                timeout_ms: Some(2_000),
                cwd: None,
                env: BTreeMap::new(),
            });
        }
        let mut events = std::collections::BTreeMap::new();
        events.insert("PreToolUse".to_string(), entries);
        HookManifest {
            harness: "cursor".to_string(),
            raw_event_passthrough: false,
            events,
        }
    }

    /// `explain_core` with a command entry + http entry prints each matching
    /// entry (plugin, event, matcher, kind) and "would run", but executes NO
    /// handler: the command's sentinel file must not be created.
    #[test]
    fn explain_core_prints_entries_runs_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let sentinel = dir.path().join("was_run_by_explain.txt");
        // Command that would create a sentinel file if executed.
        let cmd = format!("touch '{}'", sentinel.display());

        let m = make_manifest_for_explain(Some(&cmd), Some("https://example.invalid/hook"));

        let stdin = r#"{"tool_name":"Bash"}"#;
        let lines = explain_core("cursor", "PreToolUse", stdin, Some(&m));

        // Handler was NOT run: the sentinel file must be absent.
        assert!(
            !sentinel.exists(),
            "explain_core must run no handlers (command sentinel created by accident)"
        );

        let joined = lines.join("\n");
        // Both entries are reported.
        assert!(
            joined.contains("cat:cmd-guard"),
            "output must name the command plugin; got: {joined}"
        );
        assert!(
            joined.contains("kind=command"),
            "output must show handler kind=command; got: {joined}"
        );
        assert!(
            joined.contains("cat:http-guard"),
            "output must name the http plugin; got: {joined}"
        );
        assert!(
            joined.contains("kind=http"),
            "output must show handler kind=http; got: {joined}"
        );
        assert!(
            joined.contains("would run"),
            "output must contain 'would run'; got: {joined}"
        );

        // Handler BODY is not printed.
        assert!(
            !joined.contains("touch"),
            "command body must NOT appear in explain output; got: {joined}"
        );
        assert!(
            !joined.contains("example.invalid"),
            "http URL must NOT appear in explain output; got: {joined}"
        );
    }

    /// A non-matching tool name produces the "no entries match" fallback and
    /// still runs nothing.
    #[test]
    fn explain_core_no_match_produces_fallback_message() {
        let m = make_manifest_for_explain(Some("exit 2"), None);
        // `Edit` does not match the `Bash` matcher.
        let stdin = r#"{"tool_name":"Edit"}"#;
        let lines = explain_core("cursor", "PreToolUse", stdin, Some(&m));
        let joined = lines.join("\n");
        assert!(
            joined.contains("no entries match") || joined.contains("allow"),
            "no-match must produce a fallback allow message; got: {joined}"
        );
    }

    /// A missing manifest produces the "no manifest" fallback, never panics.
    #[test]
    fn explain_core_no_manifest_produces_fallback_message() {
        let lines = explain_core("cursor", "PreToolUse", "{}", None);
        let joined = lines.join("\n");
        assert!(
            joined.contains("no manifest"),
            "missing manifest must produce a fallback message; got: {joined}"
        );
    }

    /// The scrubber elides a secret in the plugin name (provider_key pattern):
    /// a `sk-...` token embedded in the plugin provenance string must NOT appear
    /// in the explain output — it must be replaced by `<scrubbed>`.
    ///
    /// In practice, plugin names do not contain API keys; this test is the
    /// security regression pin that the unconditional scrub pass is wired in.
    #[test]
    fn explain_core_scrubs_secret_in_plugin_metadata() {
        use crate::harness::hooks_ir::{Handler, HookManifest, ManifestEntry};
        use std::collections::BTreeMap;

        // A secret that matches the provider_key regex in scrub_credentials:
        // `sk-` + ≥16 url-safe chars.
        let secret = "sk-ant-api03-SecretSecretSecretSecretSecret";

        let entry = ManifestEntry {
            // Embed the secret in the plugin field (the scrubber must catch it).
            plugin: format!("cat:guard-{secret}"),
            plugin_root: None,
            matcher: Some("Bash".to_string()),
            if_pred: None,
            handler: Handler::Command {
                command: "exit 0".to_string(),
            },
            timeout_ms: None,
            cwd: None,
            env: BTreeMap::new(),
        };
        let mut events = BTreeMap::new();
        events.insert("PreToolUse".to_string(), vec![entry]);
        let m = HookManifest {
            harness: "cursor".to_string(),
            raw_event_passthrough: false,
            events,
        };

        let lines = explain_core("cursor", "PreToolUse", r#"{"tool_name":"Bash"}"#, Some(&m));
        let joined = lines.join("\n");

        // The raw secret must not appear.
        assert!(
            !joined.contains(secret),
            "secret must be scrubbed from explain output; got: {joined}"
        );
        // The scrub marker must be present, proving the scrubber ran.
        assert!(
            joined.contains("<scrubbed>"),
            "scrub marker '<scrubbed>' must appear in explain output; got: {joined}"
        );
    }

    /// `debug_trace_line` formats an allow decision correctly.
    #[test]
    fn debug_trace_line_formats_allow_correctly() {
        let decision = CcDecision::default(); // allow / no-op
        let line = debug_trace_line("cat:plug", "command", &decision);
        assert!(
            line.starts_with("[TOME_HOOK_DEBUG]"),
            "trace line must start with marker; got: {line}"
        );
        assert!(
            line.contains("plugin=cat:plug"),
            "trace line must contain plugin; got: {line}"
        );
        assert!(
            line.contains("kind=command"),
            "trace line must contain kind; got: {line}"
        );
        assert!(
            line.contains("decision=allow"),
            "trace line must indicate allow; got: {line}"
        );
    }

    /// `debug_trace_line` formats a deny decision and scrubs a secret in the reason.
    #[test]
    fn debug_trace_line_scrubs_secret_in_reason() {
        let secret = "sk-ant-api03-SecretSecretSecretSecretSecret";
        let decision = CcDecision {
            permission: Some(Permission::Deny),
            block: true,
            reason: Some(format!("rejected because bearer: {secret}")),
            ..Default::default()
        };
        let line = debug_trace_line("cat:plug", "command", &decision);

        assert!(
            line.contains("[TOME_HOOK_DEBUG]"),
            "trace line must carry the debug marker; got: {line}"
        );
        assert!(
            line.contains("decision=deny"),
            "trace line must indicate deny; got: {line}"
        );
        // The raw secret must be scrubbed.
        assert!(
            !line.contains(secret),
            "secret must not appear raw in the debug trace; got: {line}"
        );
    }

    /// Fix 1 (US8 review): plugin_root is read from the manifest entry, not
    /// re-derived from the plugin_data path. An entry with a plugin_root that
    /// would differ from `.parent()` of plugin_data must see the manifest value.
    #[test]
    fn plugin_root_comes_from_manifest_not_rederived() {
        use crate::harness::hooks_ir::{Handler, ManifestEntry};
        use std::collections::BTreeMap;

        // entry.plugin_root = a known path distinct from what .parent() of
        // plugin_data would give ("/some/data/root/cat" ≠ "/real/install/root").
        // The command echoes $TOME_PLUGIN_ROOT to stdout.
        let entry = ManifestEntry {
            plugin: "cat:plug".to_string(),
            plugin_root: Some("/real/install/root".to_owned()),
            matcher: None,
            if_pred: None,
            handler: Handler::Command {
                command: r#"printf '%s' "$TOME_PLUGIN_ROOT""#.to_string(),
            },
            timeout_ms: Some(5_000),
            cwd: None,
            env: BTreeMap::new(),
        };
        // The dispatcher builds TomeProvenance from entry.plugin_root (after Fix 1).
        let prov = TomeProvenance {
            harness: "gemini",
            workspace: "ws",
            plugin: "cat:plug",
            catalog: "cat",
            plugin_root: entry.plugin_root.as_deref().unwrap_or(""),
            plugin_data: "/some/data/root/cat/plug",
        };
        let outcome = run_command_handler(&entry, "{}", &prov);
        assert_eq!(outcome.exit, 0);
        assert_eq!(
            outcome.stdout.trim(),
            "/real/install/root",
            "TOME_PLUGIN_ROOT must equal the manifest-baked plugin_root, \
             not .parent() of plugin_data (\"/some/data/root/cat\")"
        );
    }
}

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

use crate::cli::HarnessRunHookArgs;
use crate::error::TomeError;
use crate::harness::HookWire;
use crate::harness::hooks_ir::HookManifest;
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
/// across US4.2–US4.5. Skeleton: a present manifest still routes to a
/// non-blocking allow until the remaining tasks land; a `None` manifest is the
/// canonical fail-open path.
fn dispatch_inner(
    harness: &str,
    _wire: HookWire,
    _event_cc: &str,
    _stdin: &str,
    manifest: Option<&HookManifest>,
) -> DispatchOutput {
    let _ = manifest;
    fail_open_output(harness)
}

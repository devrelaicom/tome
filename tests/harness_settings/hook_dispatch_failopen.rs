//! US4.1 — the runtime dispatcher's fail-open contract.
//!
//! Fail-open totality is the dispatcher's #1 invariant: any Tome-side fault
//! degrades to the harness's allow/no-op at exit 0. This drives the pure core
//! [`dispatch_core`] directly (no stdin/process-exit plumbing) so the contract
//! is asserted without a subprocess.

use tome::commands::harness::run_hook;

/// A missing manifest (`None`) feeding any event JSON must NOT error and must
/// emit a fail-open allow/no-op for the harness at exit 0.
#[test]
fn dispatch_fails_open_when_manifest_missing() {
    let out = run_hook::dispatch_core("cursor", "PreToolUse", "{}", None);
    assert_eq!(out.exit_code, 0);
    // Cursor allow shape: an empty no-op is the preferred fail-open allow.
    assert!(
        out.stdout.is_empty() || out.stdout.contains("\"permission\":\"allow\""),
        "fail-open allow must be empty or an explicit cursor allow, got: {:?}",
        out.stdout,
    );
}

/// An UNKNOWN harness (no `hook_support()` in the registry) also fails open:
/// `wire_for` returns `None`, so the dispatcher emits the empty allow + exit 0
/// rather than panicking on the missing wire.
#[test]
fn dispatch_fails_open_for_unknown_harness() {
    let out = run_hook::dispatch_core("not-a-real-harness", "PreToolUse", "{}", None);
    assert_eq!(out.exit_code, 0);
    assert!(out.stdout.is_empty());
}

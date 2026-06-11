//! Local-first, fire-and-forget telemetry (Phase 10).
//!
//! The defining invariant is **zero foreground network / no blocking**: the
//! CLI and the MCP handlers only ever append one bounded line to a local JSONL
//! queue; delivery is a best-effort detached flusher. This module is
//! deliberately `tokio`-free — it is sync-only (the MCP timer `spawn_blocking`s
//! into [`flush`]). See `specs/010-phase-10-telemetry/`.
//!
//! Phase 1 (this slice) lands the module skeleton + the typed event enums only;
//! every function below is a stub filled in by a later phase.

pub mod event;

/// Whether telemetry is enabled for this process (opt-out + CI auto-disable).
///
/// Phase-2 fill: reads `telemetry/config.toml` + the `TOME_TELEMETRY`/CI env
/// signals. The skeleton reports disabled so no caller emits before the queue
/// path exists.
pub fn is_enabled() -> bool {
    false
}

/// Append one event to the local JSONL queue (`O_APPEND`, ≤4 KiB line).
///
/// Phase-2 fill: takes a typed `Event` and serialises it; the signature is
/// intentionally left no-arg in the skeleton because the event type does not
/// exist yet and an unused parameter would not be clippy-clean.
pub fn enqueue() {}

/// Best-effort, blocking delivery of the queued events to the collector.
///
/// Phase-2 fill: non-blocking `flush.lock` → read queue → `reqwest::blocking`
/// POST → rewrite-after-2xx. Foreground callers surface the
/// `TelemetryEndpointUnreachable` (exit 90) error; background/`--quiet`
/// flushes fail silent.
pub fn flush() -> Result<(), crate::error::TomeError> {
    Ok(())
}

/// Spawn the detached flusher at process exit (CLI: `setsid` child).
///
/// Phase-2 fill: spawns `tome telemetry flush --quiet` and does not wait.
pub fn teardown_at_exit() {}

/// Emit the one-line first-run opt-out notice if it has not been shown.
///
/// Phase-2 fill: prints the CLI-only notice once, guarded by a marker file.
pub fn first_run_notice_if_needed() {}

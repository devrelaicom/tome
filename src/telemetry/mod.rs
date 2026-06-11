//! Local-first, fire-and-forget telemetry (Phase 10).
//!
//! The defining invariant is **zero foreground network / no blocking**: the
//! CLI and the MCP handlers only ever append one bounded line to a local JSONL
//! queue; delivery is a best-effort detached flusher. This module is
//! deliberately `tokio`-free — it is sync-only (the MCP timer `spawn_blocking`s
//! into [`flush`]). See `specs/010-phase-10-telemetry/`.
//!
//! Phase 2 (this slice) lands config + clock + transport-scaffolding plus the
//! enqueue gate; the actual queue append (US2) and delivery POST (US3) are
//! still stubs below.

pub mod buckets;
pub mod clock;
pub mod config;
pub mod event;
pub mod transport;

/// Whether telemetry is enabled for this process (opt-out + CI auto-disable).
///
/// This is the **best-effort, infallible** gate the silent enqueue path uses:
/// it resolves the default [`Paths`](crate::paths::Paths) from `$HOME` and calls
/// [`config::resolve_enabled`].
///
/// FAIL-SAFE-OFF: any error — `$HOME` unresolvable, or a malformed
/// `config.toml` (exit 91 on the *CLI* path) — collapses to `false` here. The
/// silent path must NEVER emit under a broken config and must NEVER crash the
/// user's foreground command, so it diverges from the CLI surface (which
/// surfaces the 91): a background emit just stays quiet.
pub fn is_enabled() -> bool {
    let paths = match crate::paths::Paths::resolve() {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(error = %e, "telemetry disabled: $HOME unresolvable");
            return false;
        }
    };
    match config::resolve_enabled(&paths) {
        Ok(enabled) => enabled,
        Err(e) => {
            // Malformed config on the silent path: fail safe OFF, never panic.
            tracing::debug!(error = %e, "telemetry disabled: config resolve failed (fail-safe-off)");
            false
        }
    }
}

/// Append one event to the local JSONL queue (`O_APPEND`, ≤4 KiB line).
///
/// Gated on [`is_enabled`]: when telemetry is disabled NOTHING is enqueued and
/// no flusher is spawned (FR-010). The enabled branch stamps the shared
/// envelope and appends the line — filled in by US2.
pub fn enqueue<E: event::AnonymousEvent>(event: E) {
    if !is_enabled() {
        // Disabled ⇒ no queue write, no flusher. Return before any I/O.
        return;
    }

    // US1/US2 fill: stamp Envelope (identity install/session UUID +
    // `clock::now_utc` timestamp) → `event::to_line` → `queue::append`. Until
    // the queue lands, consume the event so the bound is exercised and the
    // signature is clippy-clean.
    let _ = event;
}

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

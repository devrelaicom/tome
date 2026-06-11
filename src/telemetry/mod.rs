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
pub mod heartbeat;
pub mod identity;
pub mod install_method;
pub mod lock;
pub mod notice;
pub mod queue;
pub mod transport;

pub use install_method::detect_install_method;

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

/// CLI process-start telemetry orchestration: first-run notice + the
/// `tome.install` / `tome.upgrade` lifecycle emits (FR-026).
///
/// This REPLACES the bare first-run-notice call `main.rs` used to make. The key
/// reason install + notice are CO-LOCATED here (rather than letting `notice`
/// mint and `enqueue` mint again) is that BOTH are driven off the SINGLE
/// `ensure_install_id` mint: `just_minted` is the one signal that says "this is
/// the very first run of this install", so the notice and the once-per-install
/// `tome.install` event must fire from the same call — minting twice would risk
/// two `just_minted = true` observations (or, worse, a notice with no matching
/// install event).
///
/// Best-effort throughout (NFR-001): every branch fails safe to a `debug!` +
/// return — a broken telemetry path must NEVER crash, alter the exit code, or
/// block the user's foreground command. The MCP surface does NOT call this (it
/// has no human stderr and mints silently on its first enqueue); `main.rs` gates
/// this on the non-`Mcp`/non-`Telemetry` commands.
pub fn cli_startup(paths: &crate::paths::Paths) {
    // 1. Gate — fail-safe-OFF. A disabled install (opt-out / CI / malformed
    //    config → exit 91 on the CLI path) mints NOTHING here and emits nothing,
    //    so a fresh CI checkout leaves no trace (FR-010). We never propagate the
    //    resolve error: the startup path is best-effort.
    match config::resolve_enabled(paths) {
        Ok(true) => {}
        Ok(false) => return,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "cli_startup skipped: enabled-resolve failed (fail-safe-off)");
            return;
        }
    }

    // 2. The SINGLE mint. `just_minted` drives both the notice and the install
    //    event below. On error we cannot establish identity — skip silently and
    //    let the next run retry (the foreground command is unaffected).
    let just_minted = match identity::ensure_install_id(paths) {
        Ok((_uuid, minted)) => minted,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "cli_startup skipped: install id mint failed");
            return;
        }
    };

    // 3. First-ever run for this install: print the CLI opt-out notice AND emit
    //    `tome.install` exactly ONCE (FR-026). Both hang off the same mint.
    if just_minted {
        notice::print_first_run_notice();
        enqueue(event::Install {
            install_method: detect_install_method(),
        });
    }

    // 4. Always: detect a version change and emit `tome.upgrade` on a real
    //    upgrade. `detect_and_record_version` returns `Some(old)` ONLY on an
    //    actual version change (never on first run, where it just stamps the
    //    current version), so this can't double-fire alongside the install
    //    event above on a brand-new install.
    match identity::detect_and_record_version(paths) {
        Ok(Some(from_version)) => {
            enqueue(event::Upgrade { from_version });
        }
        Ok(None) => {}
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "cli_startup: version-detect failed (best-effort)");
        }
    }

    // 5. Heartbeat LAST (FR-039): once-per-UTC-day inventory snapshot. Its own
    //    cheap date gate keeps the expensive read-only count-gathering to at
    //    most once daily; it shares the `paths` gated on above. Best-effort and
    //    read-only (NEVER takes the index advisory lock).
    heartbeat::maybe_emit_heartbeat(paths);
}

/// Append one event to the local JSONL queue (`O_APPEND`, ≤4 KiB line).
///
/// Gated on [`is_enabled`]: when telemetry is disabled NOTHING is enqueued and
/// no flusher is spawned (FR-010). The enabled branch stamps the shared envelope
/// and appends the line via [`enqueue_to`] against the default
/// [`Paths`](crate::paths::Paths).
///
/// INFALLIBLE + best-effort: this NEVER panics or propagates. Every failure
/// branch (`$HOME` unresolvable, id mint failure, serialization failure, queue
/// write failure) collapses to a `debug!` + return — a broken telemetry path
/// must never crash or block the user's foreground command.
///
/// This is EXACTLY ONE append — no network, no contended lock, no wait
/// (NFR-001). The flush trigger (CLI exit hook / MCP timer) is US3; `enqueue`
/// itself NEVER flushes or spawns.
pub fn enqueue<E: event::AnonymousEvent>(event: E) {
    if !is_enabled() {
        // Disabled ⇒ no queue write, no flusher. Return before any I/O.
        return;
    }

    // Resolve the default Paths. On failure (no `$HOME`) we have nowhere to
    // write — fail safe, stay quiet.
    let paths = match crate::paths::Paths::resolve() {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "enqueue skipped: $HOME unresolvable");
            return;
        }
    };
    enqueue_to(&paths, event);
}

/// Path-injectable [`enqueue`] (the shared body). Doc-hidden — exposed so tests
/// can target a `TempDir`-rooted [`Paths`](crate::paths::Paths) without touching
/// the real `$HOME`; production callers use [`enqueue`], which resolves the
/// default paths and delegates here.
///
/// Same best-effort contract as [`enqueue`]: infallible, exactly one append, no
/// network, no contended lock, no wait.
///
/// UN-GATED primitive: unlike [`enqueue`], this does NOT call [`is_enabled`].
/// Callers MUST gate themselves on the enabled state before calling (as
/// `cli_startup` / `heartbeat::maybe_emit_heartbeat` do, via `resolve_enabled`),
/// or use the public [`enqueue`], which resolves the default paths and gates.
#[doc(hidden)]
pub fn enqueue_to<E: event::AnonymousEvent>(paths: &crate::paths::Paths, event: E) {
    // Lazily mint (or read) the install id. This LAZY mint realizes the MCP
    // silent-mint (AC#7): the MCP server's first enqueue mints the id with no
    // first-run notice (the notice is a CLI-only concern). On error we cannot
    // form an envelope — stay quiet.
    let (install_uuid, _just_minted) = match identity::ensure_install_id(paths) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "enqueue skipped: install id unavailable");
            return;
        }
    };

    let envelope = event::Envelope::new(
        install_uuid,
        identity::session_id(),
        event::CURRENT_OS,
        event::CURRENT_ARCH,
        event::format_rfc3339_millis(clock::now_utc()),
        E::EVENT_TYPE,
    );

    let line = match event::to_line(&envelope, &event) {
        Ok(l) => l,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "enqueue skipped: serialization failed");
            return;
        }
    };

    // The one and only side effect: a single append. Any I/O error is a dropped
    // event, never a propagated failure on this silent path.
    if let Err(e) = queue::append(paths, &line) {
        tracing::debug!(target: "telemetry", error = %e, "enqueue dropped: queue append failed");
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::Paths;
    use crate::telemetry::event::{Install, InstallMethod};
    use tempfile::TempDir;

    fn paths_in(dir: &TempDir) -> Paths {
        Paths::from_root(dir.path().to_path_buf())
    }

    #[test]
    fn enqueue_to_appends_one_parseable_envelope_stamped_line() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);

        enqueue_to(
            &paths,
            Install {
                install_method: InstallMethod::Brew,
            },
        );

        // Exactly one line landed.
        let lines = queue::read_lines(&paths).unwrap();
        assert_eq!(lines.len(), 1, "exactly one append");

        // It parses as JSON and carries the right event_type + envelope shape.
        let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(v["event_type"], "tome.install");
        assert_eq!(v["schema_version"], 1);
        assert_eq!(v["sample_rate"], 1.0);
        assert_eq!(v["install_method"], "brew");
        // Install uuid is a valid v4 (the lazy mint ran).
        let install = v["install_uuid"].as_str().unwrap();
        assert!(crate::telemetry::event::Uuid::parse(install).is_some());
        // Session uuid present + valid.
        let session = v["session_uuid"].as_str().unwrap();
        assert!(crate::telemetry::event::Uuid::parse(session).is_some());
        // Timestamp matches the pinned shape (ends in `Z`, has a `.mmm` field).
        let ts = v["timestamp"].as_str().unwrap();
        assert!(ts.ends_with('Z'), "timestamp ends in Z: {ts}");
        assert!(
            ts.len() == "2026-06-11T14:11:45.123Z".len(),
            "timestamp is millisecond RFC3339: {ts}"
        );
    }

    #[test]
    fn enqueue_to_lazily_mints_install_id_on_first_call() {
        // AC#7: the first enqueue mints the install id with no notice.
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        assert!(!paths.telemetry_id().exists(), "no id before first enqueue");

        enqueue_to(
            &paths,
            Install {
                install_method: InstallMethod::Cargo,
            },
        );

        assert!(paths.telemetry_id().exists(), "id minted by first enqueue");
    }
}

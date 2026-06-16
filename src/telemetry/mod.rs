//! Local-first, fire-and-forget telemetry (Phase 10).
//!
//! The defining invariant is **zero foreground network / no blocking**: the
//! CLI and the MCP handlers only ever append one bounded line to a local JSONL
//! queue; delivery is a best-effort detached flusher. This module is
//! deliberately `tokio`-free — it is sync-only (the MCP timer `spawn_blocking`s
//! into [`flush`]). See `specs/010-phase-10-telemetry/`.
//!
//! US1 lands config + clock + transport-scaffolding plus the enqueue gate; US2
//! the queue append; US3 the delivery POST ([`flush`]) plus the two delivery
//! callers — the CLI single-exit-path [`teardown_at_exit`] (which forks the
//! detached `setsid` flusher via [`spawn`]) and the MCP timer (in `src/mcp/`).

pub mod allowlist;
pub mod buckets;
pub mod clock;
pub mod config;
pub mod event;
pub mod flush;
pub mod heartbeat;
pub mod identity;
pub mod install_method;
pub mod lock;
pub mod notice;
pub mod queue;
pub mod resolver;
pub mod spawn;
pub mod transport;

pub use install_method::detect_install_method;
pub use resolver::resolve_attribution;

use std::sync::atomic::{AtomicBool, Ordering};

/// THE one serialisation lock every lib test that touches a PROCESS-GLOBAL flush
/// seam must hold for its whole duration.
///
/// The flush seams — `flush::TRANSPORT_OVERRIDE` / `flush::CRASH_POINT`,
/// `transport::NETWORK_CALLS`, and the DEFAULT-`$HOME` queue that
/// [`flush`]/[`enqueue`] resolve — are all process-global, and they are exercised
/// from THREE different lib-test modules (`flush.rs`, `transport.rs`, and
/// `mcp::telemetry_flush_loop_tests`). Before this lock each module serialised on
/// its OWN mutex, so a `flush.rs` seam test and the MCP loop test (or a
/// `transport.rs` counter test) could run CONCURRENTLY and clobber each other's
/// transport override / crash slot / `NETWORK_CALLS` delta / `$HOME` queue — the
/// `notify_drives_exactly_one_drain` flake. One shared lock makes that impossible.
///
/// Doc-hidden, test-only. Acquire it via [`test_serial`] (lock-poison tolerant).
/// A test must acquire it EXACTLY ONCE — never via two helpers that both lock it.
#[doc(hidden)]
pub static TELEMETRY_TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Acquire [`TELEMETRY_TEST_SERIAL`], recovering a poisoned mutex (a panicking
/// test must not deadlock the rest of the suite). Test-only.
#[doc(hidden)]
pub fn test_serial() -> std::sync::MutexGuard<'static, ()> {
    TELEMETRY_TEST_SERIAL
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Set `true` exactly once when THIS process's `cli_startup` minted a fresh
/// install id. The exit hook reads it: a brand-new install should schedule a
/// flush even with `< 50` queued events, so the delivery cadence is established
/// from the first run (the 10-min grace means the child sends nothing yet —
/// this just primes the throttle/cadence). Process-global because there is one
/// install id per process and the mint observation and the exit-hook read are
/// both single-threaded on the CLI path.
static MINTED_THIS_RUN: AtomicBool = AtomicBool::new(false);

/// The spawn throttle window (FR-048): the exit hook forks at most ONE detached
/// flusher per minute, so a scripted loop of hundreds of invocations can't
/// fork-storm (SC-003). Compared against the `telemetry/last-flush-attempt`
/// stamp the hook writes before each spawn.
const SPAWN_THROTTLE: time::Duration = time::Duration::minutes(1);

/// The queue-depth threshold (FR-047): a queue at/over this many pending events
/// triggers a spawn regardless of age.
const SPAWN_QUEUE_THRESHOLD: usize = 50;

/// The oldest-event age threshold (FR-047): the oldest queued event being older
/// than this triggers a spawn even below the depth threshold.
const SPAWN_OLDEST_AGE: time::Duration = time::Duration::minutes(5);

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
        // Record the fresh mint so the exit hook schedules the first flush even
        // below the queue-depth threshold (establishes the delivery cadence).
        MINTED_THIS_RUN.store(true, Ordering::Relaxed);
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

    // The session id can fail to mint if the OS RNG is unavailable. On this
    // silent best-effort path we drop the event (debug log) rather than panic.
    let session_uuid = match identity::session_id() {
        Some(u) => u,
        None => {
            tracing::debug!(target: "telemetry", "enqueue skipped: session id unavailable (RNG)");
            return;
        }
    };

    let envelope = event::Envelope::new(
        install_uuid,
        session_uuid,
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

/// Append one CATALOG-ATTRIBUTED event to the local JSONL queue.
///
/// Mirrors [`enqueue`] exactly (same gate, same best-effort/infallible contract,
/// same single `O_APPEND`) with ONE difference: the envelope's `event_type` is
/// built dynamically as `catalog.<catalog_id>.<suffix>` and `sample_rate` is
/// omitted. The caller has ALREADY resolved attribution (via
/// [`allowlist::match_source`]) and constructed the typed event with the matched
/// short id; this function does NOT re-resolve — it just stamps and appends.
///
/// Attributed events are NEVER sampled (FR-058): there is no client-side sampling
/// in v1, but should one ever be added, the attributed path MUST bypass the
/// sample gate entirely (their volume is bounded by allowlist size, per-event
/// value is high). This function is the place that bypass belongs.
pub fn enqueue_attributed<E: event::AttributedEvent>(event: E) {
    if !is_enabled() {
        return;
    }
    let paths = match crate::paths::Paths::resolve() {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "enqueue_attributed skipped: $HOME unresolvable");
            return;
        }
    };
    enqueue_attributed_to(&paths, event);
}

/// Path-injectable [`enqueue_attributed`] (the shared body). Doc-hidden test seam,
/// mirroring [`enqueue_to`]: UN-GATED (callers must gate on the enabled state, or
/// use the public [`enqueue_attributed`]).
#[doc(hidden)]
pub fn enqueue_attributed_to<E: event::AttributedEvent>(paths: &crate::paths::Paths, event: E) {
    let (install_uuid, _just_minted) = match identity::ensure_install_id(paths) {
        Ok(pair) => pair,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "enqueue_attributed skipped: install id unavailable");
            return;
        }
    };

    // Drop silently if the session id can't be minted (RNG unavailable) — same
    // best-effort contract as the anonymous path.
    let session_uuid = match identity::session_id() {
        Some(u) => u,
        None => {
            tracing::debug!(target: "telemetry", "enqueue_attributed skipped: session id unavailable (RNG)");
            return;
        }
    };

    // Build the dynamic dotted type `catalog.<id>.<suffix>` and an envelope with
    // NO `sample_rate` (FR-058 — attributed events are never sampled).
    let event_type = format!("catalog.{}.{}", event.catalog_id(), E::EVENT_SUFFIX);
    let envelope = event::Envelope::new_attributed(
        install_uuid,
        session_uuid,
        event::CURRENT_OS,
        event::CURRENT_ARCH,
        event::format_rfc3339_millis(clock::now_utc()),
        event_type,
    );

    let line = match event::to_line(&envelope, &event) {
        Ok(l) => l,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "enqueue_attributed skipped: serialization failed");
            return;
        }
    };

    if let Err(e) = queue::append(paths, &line) {
        tracing::debug!(target: "telemetry", error = %e, "enqueue_attributed dropped: queue append failed");
    }
}

/// Best-effort, blocking delivery of the queued events to the collector.
///
/// THE single shared sync drain (NFR-010): non-blocking `flush.lock` → grace
/// gate → read queue → batch → `reqwest::blocking` POST → rewrite-after-2xx →
/// `last-flush` stamp. Delegates to [`flush::run`] against the default
/// [`Paths`](crate::paths::Paths); an unresolvable `$HOME` is best-effort
/// `Ok(())` (nothing to flush). Foreground callers surface the
/// `TelemetryEndpointUnreachable` (exit 90) error; background/`--quiet` flushes
/// ignore it.
pub fn flush() -> Result<(), crate::error::TomeError> {
    let paths = match crate::paths::Paths::resolve() {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "flush skipped: $HOME unresolvable");
            return Ok(());
        }
    };
    flush::run(&paths)
}

/// The CLI single-exit-path delivery hook (FR-047/047b): decide whether to fork
/// a detached flusher, and if so throttle + spawn it.
///
/// This is THE one explicit call site that spawns the background flusher (never a
/// `Drop`/`atexit` — the release profile is `panic = "abort"` and runs no
/// destructors, FR-047b). `main.rs` gates it OFF for the `Mcp` command (which
/// runs its own `tokio` timer) and the `Telemetry` command (so the spawned
/// `flush --quiet` child — itself a `Telemetry` command — never forks ANOTHER
/// flusher: that gating is what prevents fork-bomb recursion).
///
/// Best-effort throughout: an unresolvable `$HOME`, a disabled install, or a
/// spawn failure all just return — the user's foreground exit is never affected.
pub fn teardown_at_exit() {
    // Resolve the default Paths; nowhere to look ⇒ nothing to do.
    let paths = match crate::paths::Paths::resolve() {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "teardown skipped: $HOME unresolvable");
            return;
        }
    };

    // Disabled ⇒ never spawn (no delivery for an opted-out / CI install).
    if !is_enabled() {
        return;
    }

    // Cheap pre-check (no lock, no I/O beyond a stamp+queue read): skip the
    // common no-op exit where the FR-047 threshold isn't met.
    if !should_spawn(&paths) {
        return;
    }

    // Atomically CLAIM the spawn window. The bare pre-check above is a TOCTOU: the
    // `last-flush-attempt` stamp throttles a SEQUENTIAL loop (each run sees the
    // prior run's fresh stamp), but NOT a burst of processes exiting at once —
    // they all read the same stale/absent stamp before any one records its
    // attempt, and all fork (#225: a flusher STORM, the very thing FR-048/SC-003
    // forbid). Serialise the re-check + stamp under the non-blocking flush lock so
    // at most ONE process per root wins the window. Contended ⇒ another process
    // already owns delivery this window (or is mid-drain) ⇒ skip.
    let guard = match claim_spawn_window(&paths) {
        Some(g) => g,
        None => return,
    };

    // Holding the lock and re-confirmed under it: record the attempt so the next
    // concurrent claimant AND a sequential re-run are throttled out (FR-048).
    record_attempt(&paths);

    // Release the lock BEFORE forking — the detached child takes the SAME flush
    // lock to drain, so holding it through the spawn would make the child no-op.
    drop(guard);

    // Fork the detached `tome telemetry flush --quiet` child; best-effort.
    if let Err(e) = spawn::spawn_detached_flusher() {
        tracing::debug!(target: "telemetry", error = %e, "teardown: flusher spawn failed (best-effort)");
    }
}

/// Atomically claim the right to fork a flusher this throttle window.
///
/// Returns the held [`lock::FlushLock`] guard iff THIS process won the claim: the
/// non-blocking flush lock was acquired AND [`should_spawn`] is still true under
/// it. The lock makes the read-decide-stamp sequence atomic across processes, so a
/// burst of concurrent exits against a shared root forks ≤ 1 flusher/window
/// (FR-048/SC-003) instead of one per process (#225). `None` means: another
/// process holds the lock (owns the window, or is mid-drain), OR the re-check
/// under the lock failed (a process that won the lock just before us already
/// stamped — double-checked locking). Best-effort: a lock-open error ⇒ `None`
/// (skip rather than risk an unsynchronised fork).
#[doc(hidden)]
pub fn claim_spawn_window(paths: &crate::paths::Paths) -> Option<lock::FlushLock> {
    let guard = match lock::try_acquire(paths) {
        Ok(Some(g)) => g,
        // Contended — another process owns this window (deciding or draining).
        Ok(None) => return None,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "teardown: spawn-claim lock unavailable");
            return None;
        }
    };
    // Re-check under the lock (double-checked locking): a process that acquired the
    // lock just before us may already have stamped, throttling us out.
    if !should_spawn(paths) {
        return None; // `guard` drops here, releasing the lock
    }
    Some(guard)
}

/// Whether the exit hook should fork a flusher now: the FR-047 threshold AND the
/// FR-048 throttle, both evaluated against `paths`. Pure decision (no spawn, no
/// stamp) so it is unit-testable with a `TempDir`-rooted `Paths`.
///
/// Spawn iff:
/// - the `last-flush-attempt` stamp is ABSENT or older than the 1-min throttle
///   window (FR-048) — a scripted loop forks ≤ 1 flusher/minute; AND
/// - any threshold holds (FR-047): `queue >= 50` events, OR the oldest queued
///   event is older than 5 min, OR this process just minted the install id.
#[doc(hidden)]
pub fn should_spawn(paths: &crate::paths::Paths) -> bool {
    let now = clock::now_utc();

    // Throttle gate FIRST (cheap, and the dominant SC-003 protection): a recent
    // attempt within the window suppresses the spawn outright.
    if let Some(attempt) = read_last_flush_attempt(paths) {
        // `now < attempt` (a backward clock) is treated as "still inside the
        // window" — fail-safe-no-spawn, never fork off a skewed clock.
        if now < attempt || now < attempt + SPAWN_THROTTLE {
            return false;
        }
    }

    // Threshold: a fresh mint this run primes the cadence even below the depth.
    if MINTED_THIS_RUN.load(Ordering::Relaxed) {
        return true;
    }

    // Depth: a full-enough queue triggers regardless of age.
    if queue::count_pending(paths) >= SPAWN_QUEUE_THRESHOLD {
        return true;
    }

    // Age: the OLDEST queued event (the first line, FIFO) older than 5 min. The
    // timestamp parse is best-effort — an unparsable first line is treated as
    // "not old" (never spawn off a garbage stamp). A missing/empty queue reads
    // as no first line ⇒ no age trigger.
    oldest_event_age_exceeds(paths, now, SPAWN_OLDEST_AGE)
}

/// Stamp `telemetry/last-flush-attempt` with the current instant (atomic, 0600).
/// The SPAWN throttle key (distinct from `last-flush`, which records the last
/// successful DRAIN). Best-effort: a stamp failure is logged, never propagated —
/// worst case a single extra flusher forks next run.
#[doc(hidden)]
pub fn record_attempt(paths: &crate::paths::Paths) {
    let stamp = event::format_rfc3339_millis(clock::now_utc());
    let mut body = stamp;
    body.push('\n');
    if let Err(e) =
        crate::catalog::store::write_atomic(&paths.telemetry_last_flush_attempt(), body.as_bytes())
    {
        tracing::debug!(target: "telemetry", error = %e, "last-flush-attempt stamp failed (best-effort)");
        return;
    }
    reassert_attempt_0600(paths);
}

/// Read + parse the `telemetry/last-flush-attempt` throttle stamp. `None` when
/// absent/unreadable/unparsable (fail-safe: treated as "no recent attempt").
fn read_last_flush_attempt(paths: &crate::paths::Paths) -> Option<time::OffsetDateTime> {
    let path = paths.telemetry_last_flush_attempt();
    // Sec-L1: read/write containment parity — `record_attempt` writes via
    // `write_atomic` (symlink-refusing); refuse a symlinked component on the read
    // too. A hostile stamp is treated as absent (best-effort `None` → "no recent
    // attempt"), never propagated.
    crate::util::refuse_symlinked_component(&path).ok()?;
    let body = crate::util::bounded_read_to_string(&path, crate::util::TOME_CONFIG_MAX).ok()?;
    let first = body.lines().next().unwrap_or("").trim();
    clock::parse_rfc3339(first)
}

/// Whether the OLDEST queued event (first FIFO line) is older than `max_age`.
/// Best-effort: a missing queue, an empty queue, an unparsable first line, or a
/// first line without a `timestamp` field all return `false` (no age trigger).
fn oldest_event_age_exceeds(
    paths: &crate::paths::Paths,
    now: time::OffsetDateTime,
    max_age: time::Duration,
) -> bool {
    let lines = match queue::read_lines(paths) {
        Ok(l) => l,
        Err(_) => return false,
    };
    let first = match lines.first() {
        Some(l) => l,
        None => return false,
    };
    let ts = serde_json::from_str::<serde_json::Value>(first)
        .ok()
        .and_then(|v| {
            v.get("timestamp")
                .and_then(|t| t.as_str())
                .and_then(clock::parse_rfc3339)
        });
    match ts {
        // Only a parseable, forward-in-time-enough timestamp triggers. A future
        // timestamp (now < ts) is "not old" (never negative-age trigger).
        Some(t) => now >= t && (now - t) > max_age,
        None => false,
    }
}

/// Re-assert `0600` on the throttle stamp after an atomic replace (`write_atomic`
/// preserves the prior mode). Best-effort, Unix-only — mirrors the id/last-flush
/// re-tighten.
#[cfg(unix)]
fn reassert_attempt_0600(paths: &crate::paths::Paths) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(
        paths.telemetry_last_flush_attempt(),
        std::fs::Permissions::from_mode(0o600),
    );
}

#[cfg(not(unix))]
fn reassert_attempt_0600(_paths: &crate::paths::Paths) {}

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

    #[test]
    fn enqueue_attributed_to_builds_catalog_event_type_and_keeps_names() {
        use crate::telemetry::event::{AttributedEntryInvoked, EntryKind, Harness};
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);

        enqueue_attributed_to(
            &paths,
            AttributedEntryInvoked {
                entry_name: "midnight-compact-debug".to_string(),
                entry_kind: EntryKind::Skill,
                plugin_name: "midnight-expert".to_string(),
                plugin_version: "1.2.0".to_string(),
                catalog_id: "midnight",
                calling_harness: Some(Harness::ClaudeCode),
            },
        );

        let lines = queue::read_lines(&paths).unwrap();
        assert_eq!(lines.len(), 1, "exactly one append");
        let v: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        // The dynamic `catalog.<id>.<suffix>` type.
        assert_eq!(v["event_type"], "catalog.midnight.entry_invoked");
        // The artefact-name carve-out fields are present (FR-059).
        assert_eq!(v["entry_name"], "midnight-compact-debug");
        assert_eq!(v["plugin_name"], "midnight-expert");
        assert_eq!(v["plugin_version"], "1.2.0");
        assert_eq!(v["catalog_id"], "midnight");
        // Attributed events are never sampled ⇒ no `sample_rate` field (FR-058).
        assert!(
            v.get("sample_rate").is_none(),
            "attributed events omit sample_rate"
        );
    }

    // -----------------------------------------------------------------------
    // Exit-hook decision helpers (`should_spawn` / `record_attempt`) — the
    // throttle + threshold logic the detached-spawn cadence rests on. Driven
    // directly (a real detached child is non-deterministic to assert; the
    // throttle STAMP + the pure decision are the testable surface).
    //
    // `MINTED_THIS_RUN` is a process-global flag; these tests touch it, so they
    // serialise on a local mutex and snapshot/restore it around each case.
    // -----------------------------------------------------------------------

    use crate::telemetry::clock::ClockGuard;
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;

    static MINTED_SERIAL: Mutex<()> = Mutex::new(());

    /// RAII: force `MINTED_THIS_RUN` to `value` for the test, restore on drop.
    struct MintedGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        prior: bool,
    }
    impl MintedGuard {
        fn install(value: bool) -> Self {
            let lock = MINTED_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
            let prior = MINTED_THIS_RUN.swap(value, Ordering::Relaxed);
            Self { _lock: lock, prior }
        }
    }
    impl Drop for MintedGuard {
        fn drop(&mut self) {
            MINTED_THIS_RUN.store(self.prior, Ordering::Relaxed);
        }
    }

    /// Seed the queue with `n` anonymous lines, each carrying a `timestamp` set
    /// to `ts`. Also mints an id (so the queue/dir exist consistently).
    fn seed_queue_with_ts(paths: &Paths, n: usize, ts: &str) {
        let line = format!("{{\"event_type\":\"tome.search\",\"timestamp\":\"{ts}\"}}");
        let lines: Vec<String> = std::iter::repeat_n(line, n).collect();
        queue::rewrite(paths, &lines).unwrap();
    }

    #[test]
    fn should_spawn_false_when_attempt_is_fresh() {
        let _minted = MintedGuard::install(true); // even a fresh mint can't override the throttle
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // 50 events past the threshold, but a JUST-written attempt stamp.
        seed_queue_with_ts(&paths, 60, "2020-01-01T00:00:00.000Z");
        record_attempt(&paths);
        let before = std::fs::read_to_string(paths.telemetry_last_flush_attempt()).unwrap();

        assert!(!should_spawn(&paths), "a fresh attempt throttles the spawn");
        // The stamp is unchanged (should_spawn never writes).
        let after = std::fs::read_to_string(paths.telemetry_last_flush_attempt()).unwrap();
        assert_eq!(before, after, "should_spawn must not write the stamp");
    }

    #[test]
    fn should_spawn_true_on_full_queue_with_stale_attempt() {
        let _minted = MintedGuard::install(false);
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // A queue at/over the 50-event threshold.
        seed_queue_with_ts(&paths, 50, "2020-01-01T00:00:00.000Z");
        // Plant a STALE attempt stamp (older than the 1-min window).
        let now = clock::now_utc();
        let stale = event::format_rfc3339_millis(now - time::Duration::minutes(5));
        crate::catalog::store::write_atomic(
            &paths.telemetry_last_flush_attempt(),
            format!("{stale}\n").as_bytes(),
        )
        .unwrap();

        assert!(should_spawn(&paths), "≥50 events + a stale attempt ⇒ spawn");
    }

    #[test]
    fn should_spawn_false_when_below_threshold_and_no_mint() {
        let _minted = MintedGuard::install(false);
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // A small, recent queue (timestamp = now) and no attempt stamp.
        let now = clock::now_utc();
        let fresh = event::format_rfc3339_millis(now);
        seed_queue_with_ts(&paths, 3, &fresh);
        assert!(
            !should_spawn(&paths),
            "few recent events + no mint ⇒ no spawn"
        );
    }

    #[test]
    fn should_spawn_true_on_old_oldest_event() {
        let _minted = MintedGuard::install(false);
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // Pin "now" so the seeded timestamp is deterministically > 5 min old.
        let now = time::Date::from_calendar_date(2026, time::Month::June, 11)
            .unwrap()
            .with_hms(14, 0, 0)
            .unwrap()
            .assume_utc();
        let _clk = ClockGuard::install(now);
        let old = event::format_rfc3339_millis(now - time::Duration::minutes(6));
        // Just a couple of events (below the depth threshold), but the oldest is
        // older than 5 min ⇒ age trigger.
        seed_queue_with_ts(&paths, 2, &old);
        assert!(should_spawn(&paths), "an old oldest-event triggers a spawn");
    }

    #[test]
    fn should_spawn_true_on_fresh_mint_even_below_threshold() {
        let _minted = MintedGuard::install(true);
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // Empty queue, no attempt stamp, but a fresh mint this run.
        assert!(
            should_spawn(&paths),
            "a fresh mint primes the cadence even with no queued events"
        );
    }

    #[test]
    fn record_attempt_writes_a_parseable_stamp() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        assert!(!paths.telemetry_last_flush_attempt().exists());
        record_attempt(&paths);
        let body = std::fs::read_to_string(paths.telemetry_last_flush_attempt()).unwrap();
        // One line that parses back as a timestamp.
        assert!(
            clock::parse_rfc3339(body.trim()).is_some(),
            "attempt stamp is a parseable timestamp: {body:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn record_attempt_stamp_is_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        record_attempt(&paths);
        let mode = std::fs::metadata(paths.telemetry_last_flush_attempt())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn claim_spawn_window_skips_when_lock_is_held() {
        // #225 concurrent-exit guard: if another process already holds the flush
        // lock (mid-claim or mid-drain), THIS process must NOT also fork — even
        // when `should_spawn` would otherwise be true. This is what bounds the
        // burst to ≤ 1 flusher/window (FR-048/SC-003) where the bare stamp can't.
        let _minted = MintedGuard::install(true); // should_spawn would be true
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);

        // Hold the flush lock on another fd (stands in for a concurrent process).
        let held = lock::try_acquire(&paths)
            .expect("open flush lock")
            .expect("lock is free");
        assert!(
            claim_spawn_window(&paths).is_none(),
            "a held flush lock must block a concurrent spawn claim",
        );

        // Once released, the claim succeeds (lock free AND should_spawn holds).
        drop(held);
        assert!(
            claim_spawn_window(&paths).is_some(),
            "claim succeeds once the lock is free and should_spawn is true",
        );
    }

    #[test]
    fn claim_spawn_window_skips_when_should_not_spawn() {
        // Lock is free, but the throttle/threshold says no: the under-lock
        // re-check returns None and releases (no stamp, no fork).
        let _minted = MintedGuard::install(false);
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // A just-written attempt stamp throttles the spawn (mirrors a process that
        // won the lock immediately before us and stamped).
        record_attempt(&paths);
        assert!(
            claim_spawn_window(&paths).is_none(),
            "a fresh attempt stamp throttles the claim even with the lock free",
        );
    }

    #[test]
    fn spawn_detached_flusher_is_best_effort_ok() {
        // It must never panic; on this matrix it returns Ok (the child reparents).
        // We don't assert the child ran — only that the parent path is non-fatal.
        spawn::spawn_detached_flusher().expect("spawn is best-effort Ok");
    }
}

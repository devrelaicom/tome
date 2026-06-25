//! Local-first, fire-and-forget telemetry, re-homed onto the `gauge-telemetry`
//! kernel.
//!
//! The defining invariant is **zero foreground network / no blocking**: the CLI
//! and the MCP handlers only ever append one bounded line to a local JSONL queue
//! (the kernel's `emit`); delivery is a best-effort detached flusher (the
//! kernel's `spawn_detached_flush`, gated behind a thin Tome throttle here so a
//! scripted loop can't fork-storm). This module is deliberately `tokio`-free —
//! it is sync-only; the MCP timer `spawn_blocking`s into the kernel's drain.
//!
//! The process-global [`Telemetry`] handle is built ONCE by [`init`] early in
//! `main` (and the MCP boot). A disabled handle (consent off) is a pure no-op, so
//! every later [`emit`] / [`cli_startup`] / [`teardown_at_exit`] is a safe no-op
//! without a per-call config read.

pub mod allowlist;
pub mod clock;
pub mod config;
pub mod event;
pub mod heartbeat;
pub mod identity;
pub mod install_method;
pub mod notice;
pub mod resolver;

pub use install_method::detect_install_method;
pub use resolver::resolve_attribution;

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use gauge_telemetry::Telemetry;

/// The process-global telemetry handle. Built once by [`init`]; a disabled handle
/// (consent off, or a build error) is a pure no-op. `Telemetry` is `Send + Sync`
/// (its inner is plain config + an env snapshot), so a `OnceLock` global is sound.
static HANDLE: OnceLock<Telemetry> = OnceLock::new();

/// Test-only override for [`HANDLE`]. The production handle is a set-once
/// `OnceLock`, so a test that drives an in-process emit path (a `query`/MCP
/// handler that routes through [`emit`]) can't re-point the global at its own
/// isolated `TempDir`-rooted queue across multiple tests in one binary. This slot,
/// when `Some`, takes precedence over `HANDLE` in the two readers that route
/// through [`with_handle`]: [`is_enabled`] and [`emit`]. By contrast [`handle`],
/// [`cli_startup`], and [`teardown_at_exit`] deliberately read the global
/// [`HANDLE`] directly (the first needs a `&'static`, the latter two are the
/// startup/exit paths) — so those are exercised via the real-binary subprocess
/// tests, NOT this override. Installed + restored by a RAII guard (see
/// [`TelemetryHandleGuard`]); cleared back to `None` on drop so the next test sees
/// the real global. Doc-hidden, test-only.
#[doc(hidden)]
pub static HANDLE_OVERRIDE: std::sync::RwLock<Option<Telemetry>> = std::sync::RwLock::new(None);

/// Resolve the active handle: the test override if installed, else the global.
/// The override is read under a short-lived read lock; the borrow is handed to
/// `f` so the lock guard stays alive for the call (a `Telemetry` is not `Clone`).
fn with_handle<R>(f: impl FnOnce(Option<&Telemetry>) -> R) -> R {
    let guard = HANDLE_OVERRIDE.read().unwrap_or_else(|e| e.into_inner());
    if let Some(h) = guard.as_ref() {
        return f(Some(h));
    }
    drop(guard);
    f(HANDLE.get())
}

/// RAII guard installing a test handle into [`HANDLE_OVERRIDE`], cleared on drop.
/// Lets an in-process emit test point the process-global emit path at a
/// `TempDir`-rooted queue for the duration of the test. Hold the
/// [`test_serial`]/`HOME_MUTEX` lock around it so co-resident tests don't clobber
/// the single slot. Doc-hidden, test-only.
#[doc(hidden)]
pub struct TelemetryHandleGuard;

impl TelemetryHandleGuard {
    /// Install `handle` as the active override for the test's duration.
    pub fn install(handle: Telemetry) -> Self {
        *HANDLE_OVERRIDE.write().unwrap_or_else(|e| e.into_inner()) = Some(handle);
        Self
    }
}

impl Drop for TelemetryHandleGuard {
    fn drop(&mut self) {
        *HANDLE_OVERRIDE.write().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// THE one serialisation lock every lib test that touches a PROCESS-GLOBAL
/// telemetry seam (the default-`$HOME` queue, the clock guard) must hold for its
/// whole duration. Doc-hidden, test-only. Acquire it via [`test_serial`].
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

/// Set `true` exactly once when THIS process's [`init`] built an ENABLED handle
/// for a fresh install (the id file was absent before build). The exit hook
/// reads it: a brand-new install should schedule a flush even with `< 50` queued
/// events, so the delivery cadence is established from the first run (the kernel
/// grace period means the child sends nothing yet — this just primes the
/// throttle/cadence).
static MINTED_THIS_RUN: AtomicBool = AtomicBool::new(false);

/// The spawn throttle window: the exit hook forks at most ONE detached flusher
/// per minute, so a scripted loop of hundreds of invocations can't fork-storm.
/// Compared against the `telemetry/last-flush-attempt` stamp the hook writes
/// before each spawn.
const SPAWN_THROTTLE: time::Duration = time::Duration::minutes(1);

/// The queue-depth threshold: a queue at/over this many pending events triggers a
/// spawn regardless of age.
const SPAWN_QUEUE_THRESHOLD: usize = 50;

/// The oldest-event age threshold: the oldest queued event being older than this
/// triggers a spawn even below the depth threshold.
const SPAWN_OLDEST_AGE: time::Duration = time::Duration::minutes(5);

/// Build the global telemetry handle. Called once, early in `main` (and the MCP
/// boot), for EVERY command — the flush child + MCP timer + every emit site need
/// the handle. Best-effort: a `BuildError` (e.g. a misconfigured endpoint) stores
/// a forced-disabled handle so every later [`emit`] is a safe no-op. Records a
/// fresh-mint observation (the id file was absent before build) for the first-run
/// notice + flush priming.
///
/// The kernel resolves consent itself (env opt-out / CI auto-off / global var) on
/// top of the `config.toml [telemetry] enabled` bool we pass; we do NOT
/// double-gate here.
pub fn init(paths: &crate::paths::Paths) {
    let first_run = !paths.telemetry_id().exists();
    let handle = build_handle(paths);

    if handle.is_enabled() && first_run {
        MINTED_THIS_RUN.store(true, Ordering::Relaxed);
    }
    let _ = HANDLE.set(handle);
}

/// Build the telemetry handle for `paths` from the resolved config + endpoint.
/// The shared body of [`init`]; also the seam an in-process emit test installs
/// via [`TelemetryHandleGuard`] (the production `init` sets the set-once global,
/// which a test can't re-point at its own `TempDir` queue).
fn build_handle(paths: &crate::paths::Paths) -> Telemetry {
    let config_enabled = config::config_enabled_value(paths);
    let endpoint = config::resolve_endpoint(paths);

    Telemetry::builder()
        .app("tome")
        .app_version(env!("CARGO_PKG_VERSION"))
        .endpoint(endpoint)
        .install_id_path(paths.telemetry_id())
        .queue_path(paths.telemetry_queue())
        .app_env_var("TOME_TELEMETRY")
        .config_enabled(config_enabled)
        .runtime_enabled(true)
        // Thread Tome's richer 8-vendor CI detection into the kernel's consent
        // (the kernel default only inspects `CI`). Without this, a Jenkins box
        // (`JENKINS_URL` set, `CI` unset) would EMIT while `tome telemetry status`
        // reports "CI auto-off" — a disagreement between consent and the report.
        .ci(config::is_ci())
        .accel("cpu")
        .flush_args(vec!["telemetry".into(), "flush".into(), "--quiet".into()])
        .build()
        .unwrap_or_else(|e| {
            tracing::debug!(target: "telemetry", error = %e, "telemetry disabled: handle build failed");
            disabled_handle(paths)
        })
}

/// Build a handle for `paths` exactly as [`init`] would, for installation into
/// the [`HANDLE_OVERRIDE`] test slot. Doc-hidden, test-only.
#[doc(hidden)]
pub fn build_handle_for_test(paths: &crate::paths::Paths) -> Telemetry {
    build_handle(paths)
}

/// A guaranteed-disabled handle (the kernel has no `disabled()` constructor): a
/// forced `config_enabled(false)` + `runtime_enabled(false)` build resolves
/// consent to off and yields a pure no-op `Telemetry(None)`. A disabled build
/// never touches the filesystem, so the placeholder endpoint is unused.
fn disabled_handle(paths: &crate::paths::Paths) -> Telemetry {
    Telemetry::builder()
        .app("tome")
        .app_version(env!("CARGO_PKG_VERSION"))
        .endpoint("https://invalid.localhost")
        .install_id_path(paths.telemetry_id())
        .app_env_var("TOME_TELEMETRY")
        .config_enabled(false)
        .runtime_enabled(false)
        .build()
        .unwrap_or_else(|_| unreachable!("a disabled-consent build cannot fail"))
}

/// The global handle, if [`init`] has run. Used by the MCP `Flusher` and the
/// `tome telemetry` CLI surface (`run_flush`/`reset`).
pub fn handle() -> Option<&'static Telemetry> {
    HANDLE.get()
}

/// Whether telemetry is enabled for this process (the resolved kernel consent).
/// `false` before [`init`] runs or on a disabled handle. Honours the test
/// override slot when installed.
pub fn is_enabled() -> bool {
    with_handle(|h| h.map(|h| h.is_enabled()).unwrap_or(false))
}

/// Emit one event. Best-effort, infallible, no-op if the handle is absent or
/// disabled. Both tiers route through here — Tier-2 (attributed) events are just
/// `Event`s with bounded artefact-name fields. The kernel appends one bounded
/// line to the queue (no network, never fails the caller). Honours the test
/// override slot when installed.
pub fn emit<E: gauge_telemetry::event::Event>(event: E) {
    with_handle(|h| {
        if let Some(h) = h {
            h.emit(&event);
        }
    });
}

/// CLI process-start telemetry: the first-run notice, the `tome.install` /
/// `tome.upgrade` lifecycle emits, and the daily heartbeat (FR-026). Best-effort
/// throughout. Gated OFF in `main` for the `Mcp`/`Telemetry` commands. Assumes
/// [`init`] already ran.
pub fn cli_startup(paths: &crate::paths::Paths) {
    let Some(h) = HANDLE.get() else {
        return;
    };
    if !h.is_enabled() {
        return;
    }

    // First run for this install (the id was absent before `init` built the
    // handle): print the opt-out notice + emit `tome.install` exactly once. The
    // kernel's first emit mints the id; `MINTED_THIS_RUN` was observed in `init`.
    if MINTED_THIS_RUN.load(Ordering::Relaxed) {
        notice::print_first_run_notice();
        emit(event::Install {
            install_method: detect_install_method(),
            env: h.env(),
        });
    }

    // Upgrade detection (Tome-owned via the `last-version` stamp). `Some(old)`
    // only on a real version change (never first run, where it just stamps the
    // current version), so it can't double-fire alongside the install event.
    match identity::detect_and_record_version(paths) {
        Ok(Some(from_version)) => emit(event::Upgrade { from_version }),
        Ok(None) => {}
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "cli_startup: version-detect failed (best-effort)")
        }
    }

    // Daily heartbeat LAST (FR-039): Tome-owned once-per-UTC-day gate + read-only
    // inventory gather. Flattens the kernel env snapshot.
    heartbeat::maybe_emit_heartbeat(paths, h.env());
}

/// The CLI single-exit-path delivery hook (FR-047/048, SC-003): gate the kernel's
/// detached flush behind a thin Tome throttle — spawn iff a queue threshold holds
/// AND the 1-min attempt stamp is stale. This is THE one explicit call site that
/// forks the background flusher (never a `Drop`/`atexit` — the release profile is
/// `panic = "abort"` and runs no destructors). `main.rs` gates it OFF for the
/// `Mcp`/`Telemetry` commands (so the spawned `flush --quiet` child — itself a
/// `Telemetry` command — never forks ANOTHER flusher).
///
/// Best-effort throughout: an unresolvable `$HOME`, a disabled install, or a spawn
/// failure all just return.
///
/// SC-003 tradeoff (honest): the 1-min `last-flush-attempt` stamp throttles a
/// SEQUENTIAL loop (each run sees the prior run's fresh stamp), but it is NOT a
/// cross-process lock — under a BURST of concurrent CLI exits, up to N children
/// may be spawned at once (the bespoke `claim_spawn_window`/`lock.rs` that made
/// the burst fork ≤ 1 child was retired with the rest of the bespoke machinery).
/// This is acceptable because only ONE of those children actually DRAINS: the
/// kernel's internal queue drain lock makes the rest skip-drain and exit
/// immediately (no duplicate sends, no storm of POSTs). And the original #225
/// trigger — the local test suite spawning a flusher storm — is independently
/// prevented by the kernel's CI auto-off (telemetry is disabled under CI, so no
/// child spawns there at all).
pub fn teardown_at_exit() {
    let Some(h) = HANDLE.get() else {
        return;
    };
    if !h.is_enabled() {
        return;
    }
    let Ok(paths) = crate::paths::Paths::resolve() else {
        return;
    };
    if !should_spawn(&paths) {
        return;
    }
    record_attempt(&paths);
    h.spawn_detached_flush();
}

/// Whether the exit hook should fork a flusher now: the FR-047 threshold AND the
/// FR-048 throttle, both evaluated against `paths`. Pure decision (no spawn, no
/// stamp) so it is unit-testable with a `TempDir`-rooted `Paths`.
///
/// Spawn iff:
/// - the `last-flush-attempt` stamp is ABSENT or older than the 1-min throttle
///   window — a scripted loop forks ≤ 1 flusher/minute; AND
/// - any threshold holds: `queue >= 50` events, OR the oldest queued event is
///   older than 5 min, OR this process just minted the install id.
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
    if count_queue_lines(paths) >= SPAWN_QUEUE_THRESHOLD {
        return true;
    }

    // Age: the OLDEST queued event (the first line, FIFO) older than 5 min. The
    // timestamp parse is best-effort — an unparsable first line is treated as
    // "not old" (never spawn off a garbage stamp). A missing/empty queue reads as
    // no first line ⇒ no age trigger.
    oldest_event_age_exceeds(paths, now, SPAWN_OLDEST_AGE)
}

/// Stamp `telemetry/last-flush-attempt` with the current instant (atomic, 0600).
/// The SPAWN throttle key (distinct from the kernel's `last-flush`). Best-effort:
/// a stamp failure is logged, never propagated — worst case a single extra flusher
/// forks next run.
#[doc(hidden)]
pub fn record_attempt(paths: &crate::paths::Paths) {
    let mut body = match format_now_rfc3339() {
        Some(s) => s,
        None => return,
    };
    body.push('\n');
    if let Err(e) =
        crate::catalog::store::write_atomic(&paths.telemetry_last_flush_attempt(), body.as_bytes())
    {
        tracing::debug!(target: "telemetry", error = %e, "last-flush-attempt stamp failed (best-effort)");
        return;
    }
    reassert_attempt_0600(paths);
}

/// The current instant as an RFC3339 string (the throttle-stamp body and the
/// round-trip key `read_last_flush_attempt` parses). `None` if the host's clock
/// can't be formatted (effectively never on a sane platform).
fn format_now_rfc3339() -> Option<String> {
    clock::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .ok()
}

/// Read + parse the `telemetry/last-flush-attempt` throttle stamp. `None` when
/// absent/unreadable/unparsable (fail-safe: treated as "no recent attempt").
fn read_last_flush_attempt(paths: &crate::paths::Paths) -> Option<time::OffsetDateTime> {
    let path = paths.telemetry_last_flush_attempt();
    // Read/write containment parity — `record_attempt` writes via `write_atomic`
    // (symlink-refusing); refuse a symlinked component on the read too. A hostile
    // stamp is treated as absent (best-effort `None`), never propagated.
    crate::util::refuse_symlinked_component(&path).ok()?;
    let body = crate::util::bounded_read_to_string(&path, crate::util::TOME_CONFIG_MAX).ok()?;
    let first = body.lines().next().unwrap_or("").trim();
    clock::parse_rfc3339(first)
}

/// Count pending events = non-blank lines in the kernel queue file. Read-only;
/// a missing queue or any read error ⇒ `0`.
fn count_queue_lines(paths: &crate::paths::Paths) -> usize {
    std::fs::read_to_string(paths.telemetry_queue())
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// Read the kernel queue file and split each non-blank line into a parsed JSON
/// value (oldest first, FIFO) or a corrupt count. Read-only; a missing/unreadable
/// queue is `(empty, 0)` — a read-only report never fails on the queue.
///
/// This is the ONE shared classifier both `commands::telemetry::inspect` and
/// `doctor::telemetry::queue_report` route through (they emitted byte-identical
/// copies before this SSOT lift): one place owns "how a queue line is parsed and
/// what counts as corrupt".
pub(crate) fn classify_queue_lines(paths: &crate::paths::Paths) -> (Vec<serde_json::Value>, usize) {
    let body = match std::fs::read_to_string(paths.telemetry_queue()) {
        Ok(b) => b,
        Err(_) => return (Vec::new(), 0),
    };
    let mut events = Vec::new();
    let mut corrupt = 0usize;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(v) => events.push(v),
            Err(_) => corrupt += 1,
        }
    }
    (events, corrupt)
}

/// Whether the OLDEST queued event (first FIFO line) is older than `max_age`.
/// Best-effort: a missing queue, an empty queue, an unparsable first line, or a
/// first line without a `time_unix_nano` field all return `false` (no age
/// trigger).
///
/// The kernel queue envelope is `{"event_name":..,"time_unix_nano":<u64 nanos>,
/// "attributes":{..}}` — the age key is `time_unix_nano` (nanoseconds since the
/// Unix epoch), NOT an RFC3339 `timestamp` string.
fn oldest_event_age_exceeds(
    paths: &crate::paths::Paths,
    now: time::OffsetDateTime,
    max_age: time::Duration,
) -> bool {
    let body = match std::fs::read_to_string(paths.telemetry_queue()) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let first = match body.lines().find(|l| !l.trim().is_empty()) {
        Some(l) => l,
        None => return false,
    };
    let ts = serde_json::from_str::<serde_json::Value>(first)
        .ok()
        .and_then(|v| {
            v.get("time_unix_nano")
                .and_then(serde_json::Value::as_u64)
                .and_then(unix_nano_to_offset)
        });
    match ts {
        // Only a parseable, forward-in-time-enough timestamp triggers. A future
        // timestamp (now < ts) is "not old" (never a negative-age trigger).
        Some(t) => now >= t && (now - t) > max_age,
        None => false,
    }
}

/// Convert the kernel envelope's `time_unix_nano` (u64 nanoseconds since the
/// Unix epoch) to an `OffsetDateTime`. `None` if the value is out of range.
fn unix_nano_to_offset(nanos: u64) -> Option<time::OffsetDateTime> {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(nanos)).ok()
}

/// Re-assert `0600` on the throttle stamp after an atomic replace (`write_atomic`
/// preserves the prior mode). Best-effort, Unix-only.
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
    use tempfile::TempDir;

    fn paths_in(dir: &TempDir) -> Paths {
        Paths::from_root(dir.path().to_path_buf())
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

    /// Seed the kernel queue file with `n` REAL kernel-shaped lines, each
    /// carrying `time_unix_nano` set to `at` (the kernel envelope is
    /// `{"event_name":..,"time_unix_nano":<u64 nanos>,"attributes":{..}}`).
    fn seed_queue_at(paths: &Paths, n: usize, at: time::OffsetDateTime) {
        std::fs::create_dir_all(paths.telemetry_dir()).unwrap();
        let nanos = at.unix_timestamp_nanos().max(0) as u64;
        let line = format!(
            "{{\"event_name\":\"tome.search\",\"time_unix_nano\":{nanos},\"attributes\":{{}}}}\n"
        );
        let body: String = std::iter::repeat_n(line, n).collect();
        std::fs::write(paths.telemetry_queue(), body).unwrap();
    }

    /// A fixed, far-past instant (2020-01-01T00:00:00Z) for the threshold tests
    /// where the depth/throttle gate decides and the event age is irrelevant.
    fn old_2020() -> time::OffsetDateTime {
        time::Date::from_calendar_date(2020, time::Month::January, 1)
            .unwrap()
            .midnight()
            .assume_utc()
    }

    #[test]
    fn count_queue_lines_ignores_blank_lines() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.telemetry_dir()).unwrap();
        std::fs::write(paths.telemetry_queue(), "a\n\nb\n  \nc\n").unwrap();
        assert_eq!(count_queue_lines(&paths), 3);
        // Missing queue ⇒ 0.
        let empty = TempDir::new().unwrap();
        assert_eq!(count_queue_lines(&paths_in(&empty)), 0);
    }

    #[test]
    fn should_spawn_false_when_attempt_is_fresh() {
        let _minted = MintedGuard::install(true); // even a fresh mint can't override the throttle
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // 60 events past the threshold, but a JUST-written attempt stamp.
        seed_queue_at(&paths, 60, old_2020());
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
        seed_queue_at(&paths, 50, old_2020());
        // Plant a STALE attempt stamp (older than the 1-min window).
        let now = clock::now_utc();
        let stale = (now - time::Duration::minutes(5))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
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
        // A small, recent queue (event time = now) and no attempt stamp.
        let now = clock::now_utc();
        seed_queue_at(&paths, 3, now);
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
        // Just a couple of events (below the depth threshold), but the oldest is
        // older than 5 min ⇒ age trigger.
        seed_queue_at(&paths, 2, now - time::Duration::minutes(6));
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
    fn oldest_event_age_exceeds_reads_kernel_time_unix_nano() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let now = time::Date::from_calendar_date(2026, time::Month::June, 25)
            .unwrap()
            .with_hms(14, 0, 0)
            .unwrap()
            .assume_utc();

        // A real kernel-shaped queue whose oldest event is 6 min old ⇒ exceeds 5 min.
        seed_queue_at(&paths, 1, now - time::Duration::minutes(6));
        assert!(
            oldest_event_age_exceeds(&paths, now, SPAWN_OLDEST_AGE),
            "a kernel `time_unix_nano` 6 min in the past must exceed the 5-min age"
        );

        // A fresh (now) event does NOT exceed the age.
        seed_queue_at(&paths, 1, now);
        assert!(
            !oldest_event_age_exceeds(&paths, now, SPAWN_OLDEST_AGE),
            "a current-time event must not trip the age trigger"
        );

        // The OLD envelope key (`timestamp`) must NOT be read — an envelope
        // carrying only the legacy key reports "not old" (no false age trigger),
        // proving the reader keys on `time_unix_nano`.
        std::fs::write(
            paths.telemetry_queue(),
            "{\"event_type\":\"tome.search\",\"timestamp\":\"2020-01-01T00:00:00.000Z\"}\n",
        )
        .unwrap();
        assert!(
            !oldest_event_age_exceeds(&paths, now, SPAWN_OLDEST_AGE),
            "a legacy `timestamp`-only line carries no `time_unix_nano` ⇒ no age trigger"
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

    /// I1: `build_handle` threads Tome's 8-vendor CI detection into the kernel
    /// consent. A Jenkins box (`JENKINS_URL` set, `CI` unset) must build a
    /// DISABLED handle — otherwise it would emit while `tome telemetry status`
    /// reports "CI auto-off".
    #[test]
    fn build_handle_disabled_under_jenkins_only_env() {
        // Serialise on the telemetry seam lock (env is process-global) and clear
        // every CI/opt-out var so only the planted `JENKINS_URL` is in play.
        let _serial = test_serial();

        const CI_VARS: &[&str] = &[
            "TOME_TELEMETRY",
            "GAUGE_TELEMETRY_DISABLE",
            "CI",
            "GITHUB_ACTIONS",
            "GITLAB_CI",
            "CIRCLECI",
            "BUILDKITE",
            "JENKINS_URL",
            "TF_BUILD",
            "TEAMCITY_VERSION",
        ];
        let saved: Vec<(&str, Option<std::ffi::OsString>)> =
            CI_VARS.iter().map(|&k| (k, std::env::var_os(k))).collect();
        // SAFETY: serialised by `test_serial()`; this is the only mutator.
        for &k in CI_VARS {
            unsafe { std::env::remove_var(k) };
        }
        // Jenkins: `JENKINS_URL` set, `CI` unset.
        unsafe { std::env::set_var("JENKINS_URL", "http://ci.local/") };

        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let handle = build_handle_for_test(&paths);
        let enabled = handle.is_enabled();

        // Restore the prior env before asserting (so a failure can't leak state).
        for (k, v) in saved {
            match v {
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
        }

        assert!(
            !enabled,
            "Jenkins-only env (JENKINS_URL set, CI unset) must disable the handle"
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
}

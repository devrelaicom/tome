//! The single shared sync drain (Phase 10, US3) — NFR-010.
//!
//! [`run`] is THE delivery path every caller routes through (the detached CLI
//! `tome telemetry flush --quiet` child, and the MCP timer's `spawn_blocking`).
//! It is `tokio`-free and best-effort: it holds the `telemetry/flush.lock` for
//! the whole drain, honours the 10-minute post-mint grace period, batches the
//! queue, POSTs each batch (HTTPS-only, 5 s, no retry), and rewrites the queue
//! ONLY after a 2xx (R-9/FR-042) so a crash mid-drain loses nothing and never
//! double-sends.
//!
//! ## Crash safety (FR-042/042a, R-9)
//!
//! Two injection seam points let tests prove the at-least-once / no-double-send
//! contract without killing a real process:
//! - [`CrashPoint::AfterResponseBeforeRewrite`] — a crash here (after a 2xx,
//!   before the queue rewrite) leaves the queue UNCHANGED, so the sent batch is
//!   re-sent on the next drain (at-least-once; no LOSS).
//! - [`CrashPoint::AfterRewriteBeforeStamp`] — a crash here (after the rewrite,
//!   before the `last-flush` stamp) leaves the sent batch GONE (no double-send)
//!   with the remainder + self-heal intact and the stamp absent/stale.

use std::sync::Mutex;

use crate::error::TomeError;
use crate::paths::Paths;
use crate::telemetry::{clock, identity, queue, transport};

/// The two streams events split across (FR-044). A parsed `event_type` starting
/// with `"catalog."` is catalog-attributed; everything else (`tome.*`) is
/// anonymous. US3's queue is all `tome.*` ⇒ anonymous; the catalog branch is
/// wired now so US4 only has to add events.
const STREAM_ANONYMOUS: &str = "anonymous";
const STREAM_CATALOG: &str = "catalog";

/// A parsed queue line: the exact ORIGINAL bytes (needed verbatim for the
/// rewrite) paired with the stream it belongs to. Unparsable lines never make
/// a `ParsedLine` — they are dropped (self-heal, R-9/FR-046).
struct ParsedLine {
    /// The original line string, exactly as read (the rewrite must preserve bytes).
    original: String,
    /// `true` ⇒ catalog-attributed stream, `false` ⇒ anonymous.
    is_catalog: bool,
}

/// Test-only crash-injection points (R-9/FR-042a). When the process-global slot
/// (set by a [`CrashGuard`]) matches the point [`run`] reaches, `run` returns
/// `Ok(())` early — simulating process death mid-drain.
#[doc(hidden)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashPoint {
    /// After `post_batch` returns 2xx, BEFORE the queue rewrite.
    AfterResponseBeforeRewrite,
    /// After the queue rewrite, BEFORE the `last-flush` stamp.
    AfterRewriteBeforeStamp,
}

/// Process-global crash-injection slot. Production never sets it; a
/// [`CrashGuard`] installs/clears it for one test. A `Mutex` (not thread-local)
/// because `run` is a single shared drain that a test drives directly.
#[doc(hidden)]
static CRASH_POINT: Mutex<Option<CrashPoint>> = Mutex::new(None);

/// RAII guard installing a crash point for its lifetime; clears on drop
/// (including on panic), mirroring [`clock::ClockGuard`]. Doc-hidden — tests only.
#[doc(hidden)]
pub struct CrashGuard;

impl CrashGuard {
    /// Install `point` as the active crash injection for the current scope.
    pub fn install(point: CrashPoint) -> Self {
        *CRASH_POINT.lock().unwrap_or_else(|e| e.into_inner()) = Some(point);
        CrashGuard
    }
}

impl Drop for CrashGuard {
    fn drop(&mut self) {
        *CRASH_POINT.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Whether the crash slot currently matches `point`.
fn crash_at(point: CrashPoint) -> bool {
    *CRASH_POINT.lock().unwrap_or_else(|e| e.into_inner()) == Some(point)
}

/// Test-only injectable transport seam: a recording fake `(stream, body) ->
/// status` the flush drain can be driven against WITHOUT real TLS (production
/// `post_batch` is HTTPS-only, so the real transport can't talk to a localhost
/// stub). When `Some`, [`deliver`] calls it; when `None` (production), it calls
/// [`transport::post_batch`].
#[doc(hidden)]
#[allow(clippy::type_complexity)]
static TRANSPORT_OVERRIDE: Mutex<
    Option<Box<dyn Fn(&str, &[u8]) -> Result<u16, TomeError> + Send>>,
> = Mutex::new(None);

/// RAII guard installing a fake transport for its lifetime; clears on drop.
/// Doc-hidden — tests only.
#[doc(hidden)]
pub struct TransportGuard;

impl TransportGuard {
    /// Install `f` as the active transport for the current scope.
    pub fn install<F>(f: F) -> Self
    where
        F: Fn(&str, &[u8]) -> Result<u16, TomeError> + Send + 'static,
    {
        *TRANSPORT_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
        TransportGuard
    }
}

impl Drop for TransportGuard {
    fn drop(&mut self) {
        *TRANSPORT_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Deliver one batch through the active transport: the test override if
/// installed, else the real HTTPS-only [`transport::post_batch`].
fn deliver(stream: &str, body: &[u8]) -> Result<u16, TomeError> {
    let slot = TRANSPORT_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner());
    match slot.as_ref() {
        Some(f) => f(stream, body),
        None => transport::post_batch(stream, body),
    }
}

/// The single shared sync drain (NFR-010).
///
/// 1. Acquire `telemetry/flush.lock` non-blocking; contention ⇒ silent `Ok(())`
///    no-op (R-6/FR-041a). The guard is held for the whole drain.
/// 2. Honour the 10-minute post-mint grace period (R-7/FR-040): no mint time ⇒
///    `Ok(())` (nothing to send); grace active ⇒ `Ok(())` (send nothing).
/// 3. Read + parse the queue; drop unparsable lines (counted, `debug!` — R-9).
/// 4. Group by stream (`catalog.*` ⇒ catalog, else anonymous — FR-044).
/// 5. Per stream (anonymous first, then catalog), batch (≤100 lines AND ≤256
///    KiB) and POST each. On a 2xx, the batch's line indices are SENT; on a
///    non-2xx or transport error, stop draining (no retry) — remaining lines
///    stay unsent.
/// 6. Rewrite the queue to the UNSENT parsed lines (the dropped/unparsable
///    lines are excluded — self-heal); atomic temp+rename, 0600 (R-9/FR-042).
/// 7. Stamp `telemetry/last-flush` (atomic, 0600) with `{timestamp, last_status}`.
///
/// Returns `Ok(())` on a clean drain; on a transport/non-2xx error mid-drain it
/// still rewrites what WAS sent and returns the transport error so a FOREGROUND
/// `tome telemetry flush` can surface exit 90. Background/`--quiet` callers
/// ignore the error.
pub fn run(paths: &Paths) -> Result<(), TomeError> {
    // 1. Lock — contention is a silent no-op (only one delivery at a time).
    let _guard = match crate::telemetry::lock::try_acquire(paths)? {
        Some(g) => g,
        None => return Ok(()),
    };

    // 2. Grace period. No mint time ⇒ nothing minted, nothing to send.
    let mint = match identity::install_mint_time(paths) {
        Some(m) => m,
        None => return Ok(()),
    };
    if clock::grace_period_active(mint, clock::now_utc()) {
        return Ok(());
    }

    // 3. Read + parse. We keep BOTH the parsed stream classification AND the
    //    exact original bytes (the rewrite must be byte-faithful). Unparsable
    //    lines are dropped here (excluded from `parsed`), self-healing the queue.
    let raw_lines = queue::read_lines(paths)?;
    let mut parsed: Vec<ParsedLine> = Vec::with_capacity(raw_lines.len());
    let mut dropped = 0usize;
    for line in raw_lines {
        match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(value) => {
                let is_catalog = value
                    .get("event_type")
                    .and_then(|v| v.as_str())
                    .is_some_and(|t| t.starts_with("catalog."));
                parsed.push(ParsedLine {
                    original: line,
                    is_catalog,
                });
            }
            Err(_) => dropped += 1,
        }
    }
    if dropped > 0 {
        tracing::debug!(target: "telemetry", dropped, "flush self-heal: dropped unparsable lines");
    }

    // Nothing parseable to send: still rewrite (to drop any unparsable lines we
    // just self-healed away) and stamp, so the queue converges to clean.
    // 4–5. Per-stream batching + delivery. `sent` collects the parsed-index of
    //      every line a 2xx acknowledged.
    let mut sent: Vec<bool> = vec![false; parsed.len()];
    let mut transport_err: Option<TomeError> = None;
    let mut last_status: Option<u16> = None;

    // Anonymous first, then catalog (FR-044: deterministic order).
    'streams: for (stream, want_catalog) in [(STREAM_ANONYMOUS, false), (STREAM_CATALOG, true)] {
        // The parsed-index list for this stream, preserving FIFO order.
        let stream_indices: Vec<usize> = parsed
            .iter()
            .enumerate()
            .filter(|(_, p)| p.is_catalog == want_catalog)
            .map(|(i, _)| i)
            .collect();
        if stream_indices.is_empty() {
            continue;
        }

        // Batch by the ORIGINAL line strings (the wire shape `deliver` POSTs).
        let stream_lines: Vec<String> = stream_indices
            .iter()
            .map(|&i| parsed[i].original.clone())
            .collect();
        let batches = transport::split_batches(&stream_lines);

        for batch in batches {
            // Build the NDJSON body: original lines joined with `\n`, trailing `\n`.
            let mut body = String::new();
            for &local in &batch {
                body.push_str(&stream_lines[local]);
                body.push('\n');
            }

            match deliver(stream, body.as_bytes()) {
                Ok(status) => {
                    last_status = Some(status);
                    if (200..300).contains(&status) {
                        // CRASH SEAM 1 — after the response, before marking sent
                        // / before the rewrite. A crash here leaves the queue
                        // UNCHANGED (this batch re-sends next drain): no LOSS.
                        if crash_at(CrashPoint::AfterResponseBeforeRewrite) {
                            return Ok(());
                        }
                        for &local in &batch {
                            sent[stream_indices[local]] = true;
                        }
                    } else {
                        // Non-2xx: stop draining (no retry). Leave the rest unsent.
                        break 'streams;
                    }
                }
                Err(e) => {
                    // Transport error: stop draining, remember the (scrubbed)
                    // error so a foreground caller can surface exit 90.
                    transport_err = Some(e);
                    break 'streams;
                }
            }
        }
    }

    // 6. Rewrite-after-2xx (R-9/FR-042): the surviving queue is every parsed
    //    line NOT acknowledged by a 2xx. Unparsable lines are already excluded
    //    (we never carried them into `parsed`) — the self-heal.
    let unsent: Vec<String> = parsed
        .iter()
        .enumerate()
        .filter(|(i, _)| !sent[*i])
        .map(|(_, p)| p.original.clone())
        .collect();
    queue::rewrite(paths, &unsent)?;
    queue::reassert_queue_0600(paths);

    // CRASH SEAM 2 — after the rewrite, before the stamp. A crash here leaves
    // the sent batch GONE (no double-send) with the stamp absent/stale.
    if crash_at(CrashPoint::AfterRewriteBeforeStamp) {
        return Ok(());
    }

    // 7. Stamp `last-flush` (best-effort: a stamp failure must not mask the
    //    drain outcome). `last-flush-attempt` is the SPAWN-throttle stamp written
    //    by the exit hook (US3 part 2), NOT here.
    stamp_last_flush(paths, last_status);

    // Surface a transport/non-2xx error so a FOREGROUND flush exits 90; the
    // background/`--quiet` callers ignore it. The queue is already consistent
    // (everything 2xx'd was removed).
    match transport_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Stamp `telemetry/last-flush` with a small JSON record `{timestamp,
/// last_status}` (atomic, 0600). `status`/`doctor` read it. `last_status` is
/// `None` when no batch was POSTed (e.g. an empty queue), serialized as JSON
/// `null`. Best-effort: a stamp failure is logged, never propagated.
fn stamp_last_flush(paths: &Paths, last_status: Option<u16>) {
    let timestamp = crate::telemetry::event::format_rfc3339_millis(clock::now_utc());
    let status_field = match last_status {
        Some(s) => s.to_string(),
        None => "null".to_string(),
    };
    let body = format!("{{\"timestamp\":\"{timestamp}\",\"last_status\":{status_field}}}\n");
    if let Err(e) =
        crate::catalog::store::write_atomic(&paths.telemetry_last_flush(), body.as_bytes())
    {
        tracing::debug!(target: "telemetry", error = %e, "last-flush stamp failed (best-effort)");
        return;
    }
    reassert_last_flush_0600(paths);
}

/// Re-assert `0600` on the `last-flush` stamp (`write_atomic` preserves the
/// prior mode). Best-effort, Unix-only.
#[cfg(unix)]
fn reassert_last_flush_0600(paths: &Paths) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(
        paths.telemetry_last_flush(),
        std::fs::Permissions::from_mode(0o600),
    );
}

#[cfg(not(unix))]
fn reassert_last_flush_0600(_paths: &Paths) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::telemetry::clock::ClockGuard;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    /// The `TRANSPORT_OVERRIDE` / `CRASH_POINT` slots are PROCESS-GLOBAL, so two
    /// flush tests running on different threads would clobber each other's
    /// override (one's `Drop` clears the slot mid-run of another). Serialise
    /// every test that drives `run` against the global seams. (Pure-`split_batches`
    /// transport tests don't touch these slots and live in `transport.rs`.)
    static FLUSH_SERIAL: Mutex<()> = Mutex::new(());

    /// Acquire the serialisation lock; recover a poisoned mutex (a panicking test
    /// must not deadlock the rest).
    fn serial() -> std::sync::MutexGuard<'static, ()> {
        FLUSH_SERIAL.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn paths_in(dir: &TempDir) -> Paths {
        Paths::from_root(dir.path().to_path_buf())
    }

    /// Seed a queue with the given raw lines (bypassing the append caps) and mint
    /// an install id so `install_mint_time` returns a value.
    fn seed(paths: &Paths, lines: &[&str]) {
        identity::ensure_install_id(paths).unwrap();
        let owned: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        queue::rewrite(paths, &owned).unwrap();
    }

    /// A `tome.*` (anonymous) line carrying a parseable `event_type`.
    fn anon_line(n: u32) -> String {
        format!("{{\"event_type\":\"tome.search\",\"n\":{n}}}")
    }

    /// A `catalog.*` (attributed) line.
    fn catalog_line(n: u32) -> String {
        format!("{{\"event_type\":\"catalog.midnight.compile\",\"n\":{n}}}")
    }

    #[test]
    fn contention_is_a_silent_noop() {
        let _serial = serial();
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        seed(&paths, &["{\"event_type\":\"tome.search\"}"]);

        // Hold the flush lock so `run` sees contention.
        let _held = crate::telemetry::lock::try_acquire(&paths)
            .unwrap()
            .unwrap();

        let recorded = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::clone(&recorded);
        let _t = TransportGuard::install(move |_s, _b| {
            r2.fetch_add(1, Ordering::SeqCst);
            Ok(200)
        });

        // Contended ⇒ Ok, sends nothing, queue untouched.
        run(&paths).unwrap();
        assert_eq!(
            recorded.load(Ordering::SeqCst),
            0,
            "no POST under contention"
        );
        assert_eq!(queue::count_pending(&paths), 1, "queue untouched");
    }

    #[test]
    fn grace_active_sends_nothing() {
        let _serial = serial();
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        seed(&paths, &[&anon_line(1)]);

        // Pin "now" inside the 10-minute grace window after the (just-now) mint.
        let mint = identity::install_mint_time(&paths).unwrap();
        let _clk = ClockGuard::install(mint + time::Duration::minutes(5));

        let recorded = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::clone(&recorded);
        let _t = TransportGuard::install(move |_s, _b| {
            r2.fetch_add(1, Ordering::SeqCst);
            Ok(200)
        });

        run(&paths).unwrap();
        assert_eq!(recorded.load(Ordering::SeqCst), 0, "grace holds delivery");
        assert_eq!(
            queue::count_pending(&paths),
            1,
            "queue untouched under grace"
        );
    }

    #[test]
    fn no_mint_time_sends_nothing() {
        let _serial = serial();
        // No install id ⇒ no mint time ⇒ nothing to send (and we never seed one).
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // Seed a queue WITHOUT minting an id.
        queue::rewrite(&paths, &[anon_line(1)]).unwrap();

        let recorded = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::clone(&recorded);
        let _t = TransportGuard::install(move |_s, _b| {
            r2.fetch_add(1, Ordering::SeqCst);
            Ok(200)
        });

        run(&paths).unwrap();
        assert_eq!(recorded.load(Ordering::SeqCst), 0, "no id ⇒ no send");
    }

    #[test]
    fn grace_elapsed_drains_and_rewrites_empty_on_2xx() {
        let _serial = serial();
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        seed(&paths, &[&anon_line(1), &anon_line(2)]);

        // Push "now" 11 minutes past the mint so the grace has elapsed.
        let mint = identity::install_mint_time(&paths).unwrap();
        let _clk = ClockGuard::install(mint + time::Duration::minutes(11));

        let seen_stream = Arc::new(Mutex::new(Vec::<String>::new()));
        let s2 = Arc::clone(&seen_stream);
        let _t = TransportGuard::install(move |stream, _body| {
            s2.lock().unwrap().push(stream.to_string());
            Ok(200)
        });

        run(&paths).unwrap();
        // The anonymous stream got the events.
        assert_eq!(*seen_stream.lock().unwrap(), vec!["anonymous".to_string()]);
        // On 2xx the queue is rewritten empty.
        assert_eq!(queue::count_pending(&paths), 0, "2xx drains the queue");
        // The last-flush stamp landed and records the 200.
        let stamp = std::fs::read_to_string(paths.telemetry_last_flush()).unwrap();
        assert!(stamp.contains("\"last_status\":200"), "stamp: {stamp}");
    }

    #[test]
    fn non_2xx_leaves_queue_untouched() {
        let _serial = serial();
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        seed(&paths, &[&anon_line(1), &anon_line(2)]);
        let mint = identity::install_mint_time(&paths).unwrap();
        let _clk = ClockGuard::install(mint + time::Duration::minutes(11));

        let _t = TransportGuard::install(|_s, _b| Ok(503));
        run(&paths).unwrap();
        // A non-2xx leaves the lines in place (no retry, no rewrite-away).
        assert_eq!(queue::count_pending(&paths), 2, "non-2xx keeps the queue");
    }

    #[test]
    fn transport_error_surfaces_exit_90_and_keeps_queue() {
        let _serial = serial();
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        seed(&paths, &[&anon_line(1)]);
        let mint = identity::install_mint_time(&paths).unwrap();
        let _clk = ClockGuard::install(mint + time::Duration::minutes(11));

        let _t = TransportGuard::install(|_s, _b| {
            Err(TomeError::TelemetryEndpointUnreachable {
                endpoint: "https://collector.example/v1/events".to_string(),
            })
        });
        let err = run(&paths).unwrap_err();
        assert_eq!(err.exit_code(), 90, "foreground flush surfaces exit 90");
        assert_eq!(queue::count_pending(&paths), 1, "error keeps the queue");
    }

    #[test]
    fn batching_splits_over_100_lines_into_multiple_posts() {
        let _serial = serial();
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let lines: Vec<String> = (0..250).map(anon_line).collect();
        let refs: Vec<&str> = lines.iter().map(|s| s.as_str()).collect();
        seed(&paths, &refs);
        let mint = identity::install_mint_time(&paths).unwrap();
        let _clk = ClockGuard::install(mint + time::Duration::minutes(11));

        let posts = Arc::new(AtomicUsize::new(0));
        let p2 = Arc::clone(&posts);
        let _t = TransportGuard::install(move |_s, _b| {
            p2.fetch_add(1, Ordering::SeqCst);
            Ok(200)
        });
        run(&paths).unwrap();
        // 250 lines ⇒ at least 3 batches at the 100-line cap.
        assert!(
            posts.load(Ordering::SeqCst) >= 3,
            "≥3 batches for 250 lines"
        );
        assert_eq!(queue::count_pending(&paths), 0, "all drained on 2xx");
    }

    #[test]
    fn stream_split_posts_both_anonymous_and_catalog() {
        let _serial = serial();
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        seed(&paths, &[&anon_line(1), &catalog_line(1), &anon_line(2)]);
        let mint = identity::install_mint_time(&paths).unwrap();
        let _clk = ClockGuard::install(mint + time::Duration::minutes(11));

        let streams = Arc::new(Mutex::new(Vec::<String>::new()));
        let s2 = Arc::clone(&streams);
        let _t = TransportGuard::install(move |stream, _b| {
            s2.lock().unwrap().push(stream.to_string());
            Ok(200)
        });
        run(&paths).unwrap();
        let got = streams.lock().unwrap().clone();
        // Anonymous first, then catalog (deterministic order).
        assert_eq!(got, vec!["anonymous".to_string(), "catalog".to_string()]);
        assert_eq!(queue::count_pending(&paths), 0);
    }

    #[test]
    fn crash_seam_point1_leaves_queue_unchanged() {
        let _serial = serial();
        // Crash after a 2xx, before the rewrite: the batch is NOT removed, so
        // it re-sends next drain (at-least-once, no LOSS).
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        seed(&paths, &[&anon_line(1), &anon_line(2)]);
        let mint = identity::install_mint_time(&paths).unwrap();
        let _clk = ClockGuard::install(mint + time::Duration::minutes(11));

        let _t = TransportGuard::install(|_s, _b| Ok(200));
        let _crash = CrashGuard::install(CrashPoint::AfterResponseBeforeRewrite);

        run(&paths).unwrap();
        // Queue is UNCHANGED — nothing was rewritten away.
        assert_eq!(
            queue::count_pending(&paths),
            2,
            "crash@1 preserves the queue"
        );
        // And no stamp was written (we died before it).
        assert!(!paths.telemetry_last_flush().exists(), "crash@1 ⇒ no stamp");
    }

    #[test]
    fn crash_seam_point2_removes_sent_batch_but_skips_stamp() {
        let _serial = serial();
        // Crash after the rewrite, before the stamp: the sent batch is GONE (no
        // double-send) and the stamp is absent/stale.
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        seed(&paths, &[&anon_line(1), &anon_line(2)]);
        let mint = identity::install_mint_time(&paths).unwrap();
        let _clk = ClockGuard::install(mint + time::Duration::minutes(11));

        let _t = TransportGuard::install(|_s, _b| Ok(200));
        let _crash = CrashGuard::install(CrashPoint::AfterRewriteBeforeStamp);

        run(&paths).unwrap();
        // The 2xx'd batch was rewritten away ⇒ queue drained.
        assert_eq!(
            queue::count_pending(&paths),
            0,
            "crash@2 ⇒ sent batch removed"
        );
        // The stamp is absent (we died before writing it).
        assert!(!paths.telemetry_last_flush().exists(), "crash@2 ⇒ no stamp");
    }

    #[test]
    fn unparsable_lines_are_self_healed_away() {
        let _serial = serial();
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // Two good lines + one corrupt fragment.
        seed(&paths, &[&anon_line(1), "not json at all", &anon_line(2)]);
        let mint = identity::install_mint_time(&paths).unwrap();
        let _clk = ClockGuard::install(mint + time::Duration::minutes(11));

        // Count the events the transport actually received (corrupt line excluded).
        let received = Arc::new(AtomicUsize::new(0));
        let r2 = Arc::clone(&received);
        let _t = TransportGuard::install(move |_s, body| {
            let n = body.iter().filter(|&&b| b == b'\n').count();
            r2.fetch_add(n, Ordering::SeqCst);
            Ok(200)
        });
        run(&paths).unwrap();
        // Only the two parseable lines were sent.
        assert_eq!(received.load(Ordering::SeqCst), 2, "corrupt line not sent");
        // And the queue is fully drained (the corrupt line self-healed away too).
        assert_eq!(
            queue::count_pending(&paths),
            0,
            "self-heal drops the corrupt line"
        );
    }

    #[test]
    fn at_least_once_then_no_double_send_across_two_drains() {
        let _serial = serial();
        // End-to-end at-least-once / no-double-send: crash@1 first (queue
        // preserved), then a clean drain removes it exactly once.
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        seed(&paths, &[&anon_line(1)]);
        let mint = identity::install_mint_time(&paths).unwrap();
        let _clk = ClockGuard::install(mint + time::Duration::minutes(11));

        let sends = Arc::new(AtomicUsize::new(0));

        // Drain 1: crash before rewrite ⇒ queue kept.
        {
            let s2 = Arc::clone(&sends);
            let _t = TransportGuard::install(move |_s, _b| {
                s2.fetch_add(1, Ordering::SeqCst);
                Ok(200)
            });
            let _crash = CrashGuard::install(CrashPoint::AfterResponseBeforeRewrite);
            run(&paths).unwrap();
        }
        assert_eq!(queue::count_pending(&paths), 1, "drain1 kept the queue");

        // Drain 2: clean ⇒ sends again (the at-least-once "again") and removes it.
        {
            let s2 = Arc::clone(&sends);
            let _t = TransportGuard::install(move |_s, _b| {
                s2.fetch_add(1, Ordering::SeqCst);
                Ok(200)
            });
            run(&paths).unwrap();
        }
        assert_eq!(queue::count_pending(&paths), 0, "drain2 removed it once");
        assert_eq!(
            sends.load(Ordering::SeqCst),
            2,
            "at-least-once: sent across both drains"
        );
    }
}

//! Phase 10 / US3 (T054) — the delivery DRAIN, exercised across the crate
//! boundary through the public `#[doc(hidden)]` seams on `tome::telemetry::*`.
//!
//! Where `flush.rs`'s in-crate unit tests assert the `run` internals in
//! isolation (each branch in turn), THIS file proves the assembled
//! integration-level guarantees the US3 acceptance scenarios name: a full
//! `flush::run` drain (lock → grace → batch → POST → rewrite → stamp) driven
//! against the public API + the injectable transport/crash/clock seams, with the
//! `?stream=` split, the endpoint scrub, and the now-FALSIFIABLE network-counter
//! increment.
//!
//! ## Serialising the process-global seams
//!
//! `TRANSPORT_OVERRIDE`, `CRASH_POINT`, the `time::clock` override, and the
//! `transport::NETWORK_CALLS` counter are ALL process-global. The in-crate lib
//! tests guard them with the `#[cfg(test)]`-only `crate::telemetry::test_serial()`
//! mutex, which is invisible across the crate boundary — so THIS binary uses the
//! next coarser available lock that the sibling telemetry integration files
//! (`mcp_funnel.rs`, `queue_behavior.rs`) ALSO hold: `HOME_MUTEX`, taken via a
//! `HomeGuard`. Every test here installs a `HomeGuard` for its whole body, so it
//! can never run concurrently with another seam-touching telemetry test in this
//! binary (which would clobber the override slot mid-drain). The actual telemetry
//! state lives under a per-test `TempDir`-rooted `Paths` (NOT `$HOME`), so the
//! tests stay independent of each other's on-disk state.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use tempfile::TempDir;
use tome::paths::Paths;
use tome::telemetry::clock::ClockGuard;
use tome::telemetry::flush::{self, CrashGuard, CrashPoint, TransportGuard};
use tome::telemetry::transport;
use tome::telemetry::{identity, lock, queue};

use crate::common::HomeGuard;

// ---------------------------------------------------------------------------
// Shared fixtures.
// ---------------------------------------------------------------------------

fn paths_in(dir: &TempDir) -> Paths {
    Paths::from_root(dir.path().to_path_buf())
}

/// Mint an install id (so `install_mint_time` returns a value) and seed the
/// queue with `lines` (bypassing the append caps via `rewrite`).
fn seed(paths: &Paths, lines: &[&str]) {
    identity::ensure_install_id(paths).expect("mint install id");
    let owned: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
    queue::rewrite(paths, &owned).expect("seed queue");
}

/// An anonymous (`tome.*`) queue line carrying a parseable `event_type`.
fn anon_line(n: u32) -> String {
    format!("{{\"event_type\":\"tome.search\",\"n\":{n}}}")
}

/// A catalog-attributed (`catalog.*`) line — the FR-044 second stream.
fn catalog_line(n: u32) -> String {
    format!("{{\"event_type\":\"catalog.midnight.compile\",\"n\":{n}}}")
}

/// Push the test clock 11 minutes past the (just-now) mint so the 10-minute
/// grace has elapsed and `run` actually attempts delivery. Returns the guard,
/// which must be held for the drain.
fn past_grace(paths: &Paths) -> ClockGuard {
    let mint = identity::install_mint_time(paths).expect("mint time");
    ClockGuard::install(mint + time::Duration::minutes(11))
}

/// The shared sink a [`recording_transport`] captures into: every
/// `(stream, body-as-string)` pair the fake transport was handed.
type RecordedCalls = Arc<Mutex<Vec<(String, String)>>>;

/// A recording transport that captures every `(stream, body-as-string)` pair it
/// is handed and always returns 2xx. The shared `Vec` is returned for assertions.
fn recording_transport() -> (TransportGuard, RecordedCalls) {
    let calls = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let sink = Arc::clone(&calls);
    let guard = TransportGuard::install(move |stream, body| {
        sink.lock().unwrap_or_else(|e| e.into_inner()).push((
            stream.to_string(),
            String::from_utf8_lossy(body).into_owned(),
        ));
        Ok(200)
    });
    (guard, calls)
}

// ===========================================================================
// SC-008 — a concurrent second flusher no-ops (lock loser exits immediately).
// ===========================================================================

/// SC-008 / FR-041a: while the flush lock is HELD by someone else, `flush::run`
/// returns `Ok` immediately, POSTs NOTHING, and leaves the queue UNCHANGED — a
/// lost lock race is a silent no-op, never an error and never a partial drain.
#[test]
fn sc008_lock_loser_is_silent_noop_no_post_queue_unchanged() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());

    let dir = TempDir::new().unwrap();
    let paths = paths_in(&dir);
    seed(&paths, &[&anon_line(1), &anon_line(2)]);
    let _clk = past_grace(&paths);

    // Hold the flush lock on a second in-process fd so `run` sees contention.
    let _held = lock::try_acquire(&paths)
        .expect("lock open ok")
        .expect("fresh lock acquired");

    let (_t, calls) = recording_transport();

    // Contended ⇒ Ok, no POST, queue untouched.
    flush::run(&paths).expect("a contended drain is a silent Ok no-op");

    assert!(
        calls.lock().unwrap().is_empty(),
        "the lock loser must POST nothing"
    );
    assert_eq!(
        queue::count_pending(&paths),
        2,
        "the lock loser leaves the queue untouched"
    );
}

// ===========================================================================
// SC-010 — nothing leaves the box within the 10-minute post-mint grace.
// ===========================================================================

/// SC-010 / FR-040: inside the 10-minute grace the drain sends nothing and keeps
/// the queue intact; once the grace has ELAPSED the same queue drains (the fake
/// transport receives the events, the queue is rewritten empty on 2xx). One test
/// proves both halves of the boundary against ONE seeded queue.
#[test]
fn sc010_grace_holds_then_elapses_and_drains() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());

    let dir = TempDir::new().unwrap();
    let paths = paths_in(&dir);
    seed(&paths, &[&anon_line(1), &anon_line(2)]);
    let mint = identity::install_mint_time(&paths).expect("mint time");

    // --- Half 1: WITHIN grace (mint + 5 min) ⇒ send nothing, queue intact.
    {
        let _clk = ClockGuard::install(mint + time::Duration::minutes(5));
        let (_t, calls) = recording_transport();
        flush::run(&paths).expect("in-grace drain is Ok");
        assert!(
            calls.lock().unwrap().is_empty(),
            "the grace window holds back ALL delivery"
        );
        assert_eq!(
            queue::count_pending(&paths),
            2,
            "the queue is intact under grace"
        );
    }

    // --- Half 2: PAST grace (mint + 11 min) ⇒ the same queue drains.
    {
        let _clk = ClockGuard::install(mint + time::Duration::minutes(11));
        let (_t, calls) = recording_transport();
        flush::run(&paths).expect("post-grace drain is Ok");
        let got = calls.lock().unwrap().clone();
        assert_eq!(got.len(), 1, "one anonymous batch once grace elapsed");
        assert_eq!(
            got[0].0, "anonymous",
            "the events go on the anonymous stream"
        );
        // The body carries BOTH seeded events.
        assert!(got[0].1.contains("\"n\":1") && got[0].1.contains("\"n\":2"));
        assert_eq!(
            queue::count_pending(&paths),
            0,
            "a 2xx rewrites the queue empty"
        );
    }
}

// ===========================================================================
// SC-002 — crash-safe across two drains: no loss, no double-send.
// ===========================================================================

/// SC-002 / FR-042: a crash AFTER a 2xx but BEFORE the queue rewrite
/// (`AfterResponseBeforeRewrite`) loses nothing — the 2xx'd batch is still in the
/// queue. A second (clean) drain then RECEIVES the events and removes them
/// EXACTLY ONCE. We assert the integration guarantee end-to-end: the events are
/// received (no loss), and across the two drains the rewrite removed them once
/// (the queue ends empty, and the second drain's received set equals the events).
#[test]
fn sc002_crash_before_rewrite_loses_nothing_then_drains_exactly_once() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());

    let dir = TempDir::new().unwrap();
    let paths = paths_in(&dir);
    seed(&paths, &[&anon_line(1), &anon_line(2)]);
    let _clk = past_grace(&paths);

    // --- Drain 1: crash after the 2xx, before the rewrite ⇒ queue PRESERVED.
    {
        let (_t, calls) = recording_transport();
        let _crash = CrashGuard::install(CrashPoint::AfterResponseBeforeRewrite);
        flush::run(&paths).expect("crash@1 returns early Ok");
        // The batch WAS sent (the 2xx happened before the crash seam)...
        let got = calls.lock().unwrap().clone();
        assert_eq!(got.len(), 1, "drain1 POSTed the batch before the crash");
        assert!(got[0].1.contains("\"n\":1") && got[0].1.contains("\"n\":2"));
    }
    // ...but the queue is UNCHANGED — the rewrite never ran (at-least-once: no LOSS).
    assert_eq!(
        queue::count_pending(&paths),
        2,
        "crash before rewrite keeps the 2xx'd batch for re-send"
    );
    assert!(
        !paths.telemetry_last_flush().exists(),
        "crash before rewrite leaves no last-flush stamp"
    );

    // --- Drain 2: clean ⇒ the events are RECEIVED again and removed once.
    {
        let (_t, calls) = recording_transport();
        flush::run(&paths).expect("clean drain2 ok");
        let got = calls.lock().unwrap().clone();
        // The second drain's received set IS exactly the seeded events.
        assert_eq!(got.len(), 1, "drain2 POSTed the surviving batch");
        let body = &got[0].1;
        assert!(
            body.contains("\"n\":1") && body.contains("\"n\":2"),
            "drain2 received exactly the seeded events (no loss): {body}"
        );
    }
    // The rewrite removed them EXACTLY ONCE across the two drains: queue empty.
    assert_eq!(
        queue::count_pending(&paths),
        0,
        "after a clean drain the 2xx'd batch is removed exactly once"
    );
    let stamp =
        std::fs::read_to_string(paths.telemetry_last_flush()).expect("drain2 stamped last-flush");
    assert!(
        stamp.contains("\"last_status\":200"),
        "drain2 records the 200: {stamp}"
    );
}

/// SC-002 (companion) / FR-042a: a crash AFTER the rewrite but BEFORE the stamp
/// (`AfterRewriteBeforeStamp`) leaves the sent batch GONE from the queue (no
/// double-send on the next drain) AND no `last-flush` stamp.
#[test]
fn sc002_crash_after_rewrite_removes_batch_and_skips_stamp() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());

    let dir = TempDir::new().unwrap();
    let paths = paths_in(&dir);
    seed(&paths, &[&anon_line(1), &anon_line(2)]);
    let _clk = past_grace(&paths);

    let (_t, calls) = recording_transport();
    let _crash = CrashGuard::install(CrashPoint::AfterRewriteBeforeStamp);
    flush::run(&paths).expect("crash@2 returns early Ok");

    // The batch was POSTed once and the rewrite removed it ⇒ no double-send.
    assert_eq!(calls.lock().unwrap().len(), 1, "exactly one POST");
    assert_eq!(
        queue::count_pending(&paths),
        0,
        "the sent batch is GONE after the rewrite (no double-send next drain)"
    );
    // The stamp was skipped — we died before it.
    assert!(
        !paths.telemetry_last_flush().exists(),
        "crash after rewrite leaves no last-flush stamp"
    );
}

/// SC-008 (the "queue stays PARSEABLE after a kill" half) / FR-042/042a: across
/// BOTH crash windows, EVERY surviving queue line must round-trip through
/// `serde_json::from_str::<Value>` — a mid-drain process death never leaves a
/// torn/partial JSON fragment behind (the prior crash tests assert the COUNT is
/// right; this asserts the surviving lines are still well-formed JSON).
///
/// - crash@`AfterResponseBeforeRewrite`: the queue is UNCHANGED (nothing
///   rewritten) — every seeded line survives and must parse.
/// - crash@`AfterRewriteBeforeStamp`: the rewrite ran, so survivors are the
///   UN-acked lines. We 2xx the anonymous stream but FAIL the catalog stream so
///   the catalog line is kept by the rewrite — a real survivor to parse-check
///   (an all-2xx queue would drain empty and make the check vacuous).
#[test]
fn crash_windows_leave_a_parseable_queue() {
    /// Assert the queue at `paths` is non-empty and every line is valid JSON.
    fn assert_queue_all_parseable(paths: &Paths, context: &str) {
        let lines = queue::read_lines(paths).expect("read queue");
        assert!(
            !lines.is_empty(),
            "{context}: expected at least one surviving line to parse-check"
        );
        for line in &lines {
            serde_json::from_str::<serde_json::Value>(line).unwrap_or_else(|e| {
                panic!("{context}: surviving queue line is not parseable JSON ({e}): {line}")
            });
        }
    }

    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());

    // --- Crash window 1: AfterResponseBeforeRewrite ⇒ queue UNCHANGED.
    {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        seed(&paths, &[&anon_line(1), &anon_line(2)]);
        let _clk = past_grace(&paths);

        let (_t, _calls) = recording_transport();
        let _crash = CrashGuard::install(CrashPoint::AfterResponseBeforeRewrite);
        flush::run(&paths).expect("crash@1 returns early Ok");

        assert_eq!(
            queue::count_pending(&paths),
            2,
            "crash@1 preserves the queue (no rewrite)"
        );
        assert_queue_all_parseable(&paths, "crash@AfterResponseBeforeRewrite");
    }

    // --- Crash window 2: AfterRewriteBeforeStamp, with the catalog stream UNSENT
    //     so a real survivor is kept by the rewrite.
    {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        seed(&paths, &[&anon_line(1), &catalog_line(7)]);
        let _clk = past_grace(&paths);

        // 2xx the anonymous stream (sent ⇒ removed by the rewrite), but FAIL the
        // catalog stream (transport error ⇒ kept by the rewrite). The catalog
        // line is the surviving line we parse-check after crash@2.
        let _t = TransportGuard::install(|stream, _body| {
            if stream == "anonymous" {
                Ok(200)
            } else {
                Err(tome::error::TomeError::TelemetryEndpointUnreachable {
                    endpoint: "https://collector.example/v1/events".to_string(),
                })
            }
        });
        let _crash = CrashGuard::install(CrashPoint::AfterRewriteBeforeStamp);
        // crash@2 returns Ok early (the transport_err is never surfaced past the
        // crash seam), so `run` is Ok here.
        flush::run(&paths).expect("crash@2 returns early Ok");

        // The anonymous line was 2xx'd + rewritten away; the catalog line is kept.
        assert_eq!(
            queue::count_pending(&paths),
            1,
            "crash@2 keeps the un-acked catalog line as a survivor"
        );
        assert_queue_all_parseable(&paths, "crash@AfterRewriteBeforeStamp");
        // And it is specifically the catalog survivor (no double-send risk on the
        // already-acked anonymous line).
        let survivors = queue::read_lines(&paths).unwrap();
        assert!(
            survivors[0].contains("catalog.midnight.compile"),
            "the survivor is the un-acked catalog line: {}",
            survivors[0]
        );
    }
}

// ===========================================================================
// FR-044 — the `?stream=` split: tome.* → anonymous, catalog.* → catalog.
// ===========================================================================

/// FR-044: a mixed queue of `tome.*` and `catalog.*` lines splits into TWO POSTs
/// — one `stream == "anonymous"` carrying the `tome.*` events, one
/// `stream == "catalog"` carrying the `catalog.*` event — in deterministic order
/// (anonymous first). The integration-level proof of the stream partition through
/// the public `run` drain (the in-crate test only checks the stream-name order).
#[test]
fn fr044_stream_split_routes_anonymous_and_catalog_bodies() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());

    let dir = TempDir::new().unwrap();
    let paths = paths_in(&dir);
    // Two anonymous lines interleaved with one catalog line.
    seed(&paths, &[&anon_line(1), &catalog_line(7), &anon_line(2)]);
    let _clk = past_grace(&paths);

    let (_t, calls) = recording_transport();
    flush::run(&paths).expect("split drain ok");

    let got = calls.lock().unwrap().clone();
    assert_eq!(got.len(), 2, "one POST per non-empty stream");

    // Anonymous first (deterministic order).
    let (s0, body0) = &got[0];
    let (s1, body1) = &got[1];
    assert_eq!(s0, "anonymous", "the first POST is the anonymous stream");
    assert_eq!(s1, "catalog", "the second POST is the catalog stream");

    // The anonymous body carries the two tome.* events and NO catalog line.
    assert!(
        body0.contains("tome.search") && !body0.contains("catalog.midnight"),
        "anonymous body is the tome.* events only: {body0}"
    );
    assert!(
        body0.contains("\"n\":1") && body0.contains("\"n\":2"),
        "anonymous body carries both tome.* events: {body0}"
    );
    // The catalog body carries the catalog.* event and NO tome.* line.
    assert!(
        body1.contains("catalog.midnight.compile") && !body1.contains("tome.search"),
        "catalog body is the catalog.* event only: {body1}"
    );
    assert!(
        body1.contains("\"n\":7"),
        "catalog body is the catalog line: {body1}"
    );

    assert_eq!(
        queue::count_pending(&paths),
        0,
        "both streams 2xx'd ⇒ drained"
    );
}

// ===========================================================================
// NFR-006 / FR-045 — a credential-bearing endpoint is scrubbed everywhere it
// surfaces (here: the transport error, through the REAL post_batch).
// ===========================================================================

/// NFR-006 / FR-045: a `TOME_TELEMETRY_ENDPOINT` carrying URL credentials is
/// scrubbed before it can reach any surface. We point the REAL `post_batch` at a
/// non-routable `https://user:secret@192.0.2.1` (TEST-NET-1, RFC 5737 — connects
/// fast-fails within the 5 s timeout) and assert the returned error's `Display`
/// AND `Debug` contain neither `secret` nor `user:`.
#[test]
fn fr045_endpoint_credentials_are_scrubbed_in_the_transport_error() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());

    // The endpoint env var is process-global; the HomeGuard serialises us against
    // every sibling telemetry test that also touches it. Snapshot + restore.
    let prior = std::env::var_os("TOME_TELEMETRY_ENDPOINT");
    // SAFETY: HomeGuard holds HOME_MUTEX for this whole body, serialising env use.
    unsafe {
        std::env::set_var(
            "TOME_TELEMETRY_ENDPOINT",
            "https://user:secret@192.0.2.1/ingest",
        );
    }

    let err = transport::post_batch("anonymous", b"{}\n")
        .expect_err("a non-routable https endpoint fails transport");

    let display = format!("{err}");
    let debug = format!("{err:?}");
    for surface in [&display, &debug] {
        assert!(
            !surface.contains("secret"),
            "the password must be scrubbed from the error: {surface}"
        );
        assert!(
            !surface.contains("user:"),
            "the userinfo must be scrubbed from the error: {surface}"
        );
    }
    // It IS the right error class, and the host survives (so it's really scrubbed,
    // not just absent because the URL never appears).
    assert!(
        display.contains("192.0.2.1"),
        "the scrubbed host is still present: {display}"
    );

    // SAFETY: still under HOME_MUTEX via the HomeGuard.
    unsafe {
        match prior {
            Some(v) => std::env::set_var("TOME_TELEMETRY_ENDPOINT", v),
            None => std::env::remove_var("TOME_TELEMETRY_ENDPOINT"),
        }
    }
}

// ===========================================================================
// NFR-001 (now FALSIFIABLE) — record_network_call is the ONLY network site, and
// a real drain through post_batch DOES reach it.
// ===========================================================================

/// NFR-001 / R-10 (the falsifiable half): the foreground proofs in
/// `queue_behavior.rs` assert `network_call_count()` stays UNCHANGED after a
/// foreground enqueue. That `== before` is only meaningful if the counter CAN
/// move — so this test drives a full `flush::run` through the REAL `post_batch`
/// (no transport override) against a non-routable `https://192.0.2.1` past grace
/// and asserts the counter INCREMENTS by at least 1. This closes the US2
/// reviewer's "vacuous counter" note: the POST exists and reaches the network
/// site, so the foreground `== before` assertions are genuinely falsifiable.
///
/// (The complementary "foreground stays 0" direction is proven in
/// `queue_behavior.rs::{cli_foreground_enqueue_does_no_network,
/// mcp_tool_foreground_call_does_no_network}` and the load-bearing-seam negative
/// control there — referenced, not duplicated.)
#[test]
fn nfr001_real_drain_increments_the_network_counter() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());

    let dir = TempDir::new().unwrap();
    let paths = paths_in(&dir);
    seed(&paths, &[&anon_line(1)]);
    let _clk = past_grace(&paths);

    // Point the REAL transport at a non-routable https host (fast-fails within the
    // 5 s timeout). The endpoint env var is process-global; we hold HOME_MUTEX.
    let prior = std::env::var_os("TOME_TELEMETRY_ENDPOINT");
    // SAFETY: HomeGuard holds HOME_MUTEX for this whole body.
    unsafe { std::env::set_var("TOME_TELEMETRY_ENDPOINT", "https://192.0.2.1/v1/events") };

    let before = transport::network_call_count();
    // No TransportGuard installed ⇒ `run` uses the REAL `post_batch`. The drain
    // surfaces the transport error (foreground exit 90), which is exactly the
    // path that increments the counter.
    let err = flush::run(&paths).expect_err("a non-routable real drain surfaces exit 90");
    assert_eq!(
        err.exit_code(),
        90,
        "a real unreachable drain is TelemetryEndpointUnreachable (90)"
    );
    let after = transport::network_call_count();

    assert!(
        after > before,
        "the real POST must reach record_network_call (counter {before} → {after}) — \
         the increment that makes the foreground `== before` proofs falsifiable"
    );
    // The queue is intact (a transport error never rewrites away unsent events).
    assert_eq!(
        queue::count_pending(&paths),
        1,
        "a transport error keeps the queue for the next drain"
    );

    // SAFETY: still under HOME_MUTEX via the HomeGuard.
    unsafe {
        match prior {
            Some(v) => std::env::set_var("TOME_TELEMETRY_ENDPOINT", v),
            None => std::env::remove_var("TOME_TELEMETRY_ENDPOINT"),
        }
    }
}

// ===========================================================================
// SC-003 / throttle — covered in `delivery.rs`
// (`fresh_attempt_throttles_should_spawn_and_stamp_unchanged` +
// `stale_attempt_with_full_queue_allows_should_spawn`). Referenced, not
// duplicated: the spawn cadence is a teardown-hook concern, not a `run` drain.
// ===========================================================================

/// Smoke guard so the cross-reference above stays honest: the throttle/threshold
/// decision lives on `should_spawn`, NOT on the `flush::run` drain — a `run`
/// never forks a flusher. A clean drain past grace removes the queue WITHOUT ever
/// touching the `last-flush-attempt` throttle stamp (which only the exit hook
/// writes via `record_attempt`).
#[test]
fn run_drain_does_not_touch_the_spawn_throttle_stamp() {
    let home = TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());

    let dir = TempDir::new().unwrap();
    let paths = paths_in(&dir);
    seed(&paths, &[&anon_line(1)]);
    let _clk = past_grace(&paths);

    let posts = Arc::new(AtomicUsize::new(0));
    let p2 = Arc::clone(&posts);
    let _t = TransportGuard::install(move |_s, _b| {
        p2.fetch_add(1, Ordering::SeqCst);
        Ok(200)
    });

    flush::run(&paths).expect("drain ok");

    assert_eq!(posts.load(Ordering::SeqCst), 1, "the drain POSTed once");
    assert_eq!(
        queue::count_pending(&paths),
        0,
        "the drain emptied the queue"
    );
    // The drain stamps `last-flush` (the DRAIN record) but NOT
    // `last-flush-attempt` (the SPAWN-throttle key — the exit hook's job).
    assert!(
        paths.telemetry_last_flush().exists(),
        "a drain stamps last-flush"
    );
    assert!(
        !paths.telemetry_last_flush_attempt().exists(),
        "a drain must NOT write the spawn-throttle stamp (that is the exit hook)"
    );
}

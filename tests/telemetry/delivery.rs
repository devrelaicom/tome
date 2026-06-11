//! Phase 10 / US3 (part 2) — the `tome telemetry flush` foreground drain and the
//! exit-hook spawn-decision helpers, driven through the REAL `tome` binary over
//! an isolated `$HOME`.
//!
//! The flush surface only manifests end-to-end against a real process + a real
//! (non-routable) endpoint: exit 90 on an unreachable collector vs `--quiet`'s
//! always-0 silence. The spawn cadence (throttle + threshold) is asserted
//! deterministically through the library `should_spawn`/`record_attempt`
//! decision split (a real detached child is non-deterministic to observe).

use std::process::Command;

use crate::common::{HomeGuard, ToolEnv};

/// Every env var that can flip the telemetry enabled-state precedence OR the
/// endpoint — cleared on every spawned command so the test controls them.
const TELEMETRY_ENV_VARS: &[&str] = &[
    "TOME_TELEMETRY",
    "TOME_TELEMETRY_ENDPOINT",
    "CI",
    "GITHUB_ACTIONS",
    "GITLAB_CI",
    "CIRCLECI",
    "BUILDKITE",
    "JENKINS_URL",
    "TF_BUILD",
    "TEAMCITY_VERSION",
];

/// A spawned `tome` command with the telemetry/CI env cleared, then telemetry
/// force-enabled and pointed at a non-routable TEST-NET-1 HTTPS endpoint (so a
/// real drain attempt fails transport ⇒ exit 90, deterministically, fast).
fn flush_cmd(env: &ToolEnv) -> Command {
    let mut cmd = env.cmd();
    for &k in TELEMETRY_ENV_VARS {
        cmd.env_remove(k);
    }
    cmd.env("TOME_TELEMETRY", "1")
        // 192.0.2.0/24 (TEST-NET-1, RFC 5737) is reserved/non-routable: the POST
        // can't connect, so `post_batch` returns the transport error fast.
        .env("TOME_TELEMETRY_ENDPOINT", "https://192.0.2.1/v1/events");
    cmd
}

/// Seed `telemetry/id` with a valid v4 UUID and BACKDATE its mtime well past the
/// 10-minute grace window, so a drain actually attempts delivery (grace elapsed).
fn seed_minted_id_past_grace(env: &ToolEnv) {
    let dir = env.tome_root().join("telemetry");
    std::fs::create_dir_all(&dir).expect("create telemetry dir");
    let id = dir.join("id");
    std::fs::write(&id, "0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f\n").expect("write id");
    // The mint time is the id file's mtime; backdate it 1 hour so grace elapsed.
    let past = filetime::FileTime::from_system_time(
        std::time::SystemTime::now() - std::time::Duration::from_secs(3600),
    );
    filetime::set_file_mtime(&id, past).expect("backdate id mtime");
}

/// Seed a queue with `n` anonymous (`tome.*`) JSONL lines.
fn seed_queue(env: &ToolEnv, n: usize) {
    let dir = env.tome_root().join("telemetry");
    std::fs::create_dir_all(&dir).expect("create telemetry dir");
    let mut body = String::new();
    for i in 0..n {
        body.push_str(&format!("{{\"event_type\":\"tome.search\",\"n\":{i}}}\n"));
    }
    std::fs::write(dir.join("queue.jsonl"), body).expect("write queue");
}

// ---------------------------------------------------------------------------
// 90 — foreground flush against an unreachable endpoint
// ---------------------------------------------------------------------------

#[test]
fn flush_unreachable_endpoint_is_exit_90() {
    let env = ToolEnv::new();
    seed_minted_id_past_grace(&env);
    seed_queue(&env, 2);

    let out = flush_cmd(&env)
        .args(["telemetry", "flush"])
        .output()
        .expect("spawn tome");
    assert_eq!(
        out.status.code(),
        Some(90),
        "an unreachable collector surfaces TelemetryEndpointUnreachable (90); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

#[test]
fn flush_quiet_unreachable_is_exit_0_with_no_output() {
    let env = ToolEnv::new();
    seed_minted_id_past_grace(&env);
    seed_queue(&env, 2);

    let out = flush_cmd(&env)
        .args(["telemetry", "flush", "--quiet"])
        .output()
        .expect("spawn tome");
    // FR-020: the detached child is silent and ALWAYS exits 0, even though the
    // transport failed (the queue stays for the next drain).
    assert_eq!(
        out.status.code(),
        Some(0),
        "--quiet flush always exits 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    assert!(
        out.stdout.is_empty(),
        "--quiet flush writes nothing to stdout: {:?}",
        String::from_utf8_lossy(&out.stdout),
    );
    assert!(
        out.stderr.is_empty(),
        "--quiet flush writes nothing to stderr: {:?}",
        String::from_utf8_lossy(&out.stderr),
    );
}

// ---------------------------------------------------------------------------
// empty queue — nothing to send, clean exit 0
// ---------------------------------------------------------------------------

#[test]
fn flush_empty_queue_is_exit_0() {
    let env = ToolEnv::new();
    // A minted id past grace but NO queued events: the drain reaches the empty
    // queue, POSTs nothing, and exits 0 (no transport attempt, no error).
    seed_minted_id_past_grace(&env);

    let out = flush_cmd(&env)
        .args(["telemetry", "flush"])
        .output()
        .expect("spawn tome");
    assert_eq!(
        out.status.code(),
        Some(0),
        "an empty queue flushes cleanly (exit 0); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

// ---------------------------------------------------------------------------
// fork-bomb guard — the `flush --quiet` child writes NO throttle stamp
// ---------------------------------------------------------------------------

/// FR-047b / the anti-recursion contract: the detached `tome telemetry flush
/// --quiet` child is itself a `Telemetry` command, so `main.rs` gates
/// `teardown_at_exit` OFF for it — it NEVER reaches `record_attempt`, so it
/// cannot write `telemetry/last-flush-attempt` and therefore cannot fork ANOTHER
/// flusher (no fork-bomb). We run the REAL binary force-on over an isolated
/// `$HOME` with a drainable queue (the child actually attempts a drain, against
/// the non-routable endpoint), then assert the throttle stamp was NOT created.
#[test]
fn flush_quiet_child_does_not_fork_or_throttle_stamp() {
    let env = ToolEnv::new();
    seed_minted_id_past_grace(&env);
    seed_queue(&env, 2);

    let attempt_stamp = env.tome_root().join("telemetry").join("last-flush-attempt");
    assert!(
        !attempt_stamp.exists(),
        "no throttle stamp before the flush --quiet child runs"
    );

    let out = flush_cmd(&env)
        .args(["telemetry", "flush", "--quiet"])
        .output()
        .expect("spawn tome");
    // The child always exits 0 (silent), even though the drain hit the
    // unreachable endpoint.
    assert_eq!(
        out.status.code(),
        Some(0),
        "--quiet flush exits 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // The fork-bomb guard: a `Telemetry` command never runs `teardown_at_exit`,
    // so it never stamps `last-flush-attempt` and so never forks a grandchild.
    assert!(
        !attempt_stamp.exists(),
        "the flush --quiet child must NOT write the spawn-throttle stamp \
         (teardown_at_exit is gated OFF for Telemetry commands — no fork-bomb)"
    );
}

// ---------------------------------------------------------------------------
// throttle + threshold — the spawn decision, via the library helpers
// ---------------------------------------------------------------------------

#[test]
fn fresh_attempt_throttles_should_spawn_and_stamp_unchanged() {
    use tome::paths::Paths;
    use tome::telemetry;

    let env = ToolEnv::new();
    let paths = Paths::from_root(env.tome_root());

    // A queue past the depth threshold, but a JUST-written attempt stamp.
    seed_queue(&env, 60);
    telemetry::record_attempt(&paths);
    let stamp_before =
        std::fs::read_to_string(paths.telemetry_last_flush_attempt()).expect("attempt stamp");

    // Within the 1-min throttle window ⇒ no spawn.
    assert!(
        !telemetry::should_spawn(&paths),
        "a fresh last-flush-attempt throttles the spawn"
    );
    // And `should_spawn` never writes the stamp (the decision is pure).
    let stamp_after =
        std::fs::read_to_string(paths.telemetry_last_flush_attempt()).expect("attempt stamp");
    assert_eq!(
        stamp_before, stamp_after,
        "should_spawn must not rewrite the throttle stamp"
    );
}

#[test]
fn stale_attempt_with_full_queue_allows_should_spawn() {
    use tome::paths::Paths;
    use tome::telemetry::{self, event};

    let env = ToolEnv::new();
    let paths = Paths::from_root(env.tome_root());

    // 50 events (at the threshold) + a STALE attempt stamp (older than 1 min).
    seed_queue(&env, 50);
    std::fs::create_dir_all(env.tome_root().join("telemetry")).unwrap();
    let stale = event::format_rfc3339_millis(
        tome::telemetry::clock::now_utc() - time::Duration::minutes(5),
    );
    std::fs::write(paths.telemetry_last_flush_attempt(), format!("{stale}\n"))
        .expect("write stale attempt");

    assert!(
        telemetry::should_spawn(&paths),
        "≥50 events + a stale attempt ⇒ a spawn is allowed"
    );

    // And `record_attempt` rewrites the stamp to a fresh, parseable value — the
    // forward-progress the exit hook makes before forking the flusher.
    telemetry::record_attempt(&paths);
    let rewritten =
        std::fs::read_to_string(paths.telemetry_last_flush_attempt()).expect("attempt stamp");
    assert_ne!(
        rewritten.trim(),
        stale,
        "record_attempt advances the throttle stamp"
    );
    assert!(
        tome::telemetry::clock::parse_rfc3339(rewritten.trim()).is_some(),
        "the rewritten stamp is a parseable timestamp: {rewritten:?}"
    );
}

// ---------------------------------------------------------------------------
// SC-003 — the COMPOSED throttle loop: ≤1 spawn-decision advances the stamp per
// window. The sibling tests above check the throttle in ISOLATION (one fresh /
// one stale stamp); this drives the actual exit-hook SEQUENCE
// (`if should_spawn(p) { record_attempt(p) }`) in a tight loop and proves the
// "≤1 flusher per window" + throttle-before-spawn ordering directly.
// ---------------------------------------------------------------------------

/// SC-003 / FR-048: the exit-hook decision SEQUENCE, looped within ONE throttle
/// window, forks at most ONE flusher — `should_spawn` returns `true` only on the
/// FIRST iteration (no `last-flush-attempt` stamp yet, depth threshold met), and
/// `false` on every later iteration (the first `record_attempt` planted a fresh
/// stamp inside the 1-min window). The stamp's timestamp advances EXACTLY ONCE
/// across the loop. Advancing the clock past the window reopens it: the next
/// decision is `true` again.
///
/// Force-on + `HomeGuard` per the telemetry-test pattern (so the process-global
/// `MINTED_THIS_RUN` flag + `$HOME` are serialised against sibling telemetry
/// tests in this binary); the on-disk state lives under a per-test `TempDir`
/// `Paths`. The clock is driven by a `ClockGuard` (both `should_spawn` and
/// `record_attempt` read `clock::now_utc`), so "within one window" / "past the
/// window" is deterministic — no sleeps.
#[test]
fn throttle_loop_advances_attempt_stamp_exactly_once_per_window() {
    use tome::paths::Paths;
    use tome::telemetry::{self, clock::ClockGuard};

    // `HomeGuard` serialises us against every other seam-touching telemetry test
    // in this binary (the process-global `MINTED_THIS_RUN`); a stale env can't
    // flip our decision. The actual paths are a per-test `TempDir`, not `$HOME`.
    let home = tempfile::TempDir::new().unwrap();
    let _home = HomeGuard::install(home.path());

    let dir = tempfile::TempDir::new().unwrap();
    let paths = Paths::from_root(dir.path().to_path_buf());

    // Pin the clock so the whole loop runs WITHIN one throttle window.
    let t0 = time::Date::from_calendar_date(2026, time::Month::June, 11)
        .unwrap()
        .with_hms(14, 0, 0)
        .unwrap()
        .assume_utc();
    let clk = ClockGuard::install(t0);

    // Seed a queue past the depth threshold so the FIRST decision fires on depth
    // (independent of the process-global mint flag), and ensure NO attempt stamp
    // exists yet (the throttle must be open on iteration 1). We write via
    // `queue::rewrite` (bypasses the per-append caps) so all 60 lines land.
    let lines: Vec<String> = (0..60)
        .map(|n| format!("{{\"event_type\":\"tome.search\",\"n\":{n}}}"))
        .collect();
    tome::telemetry::queue::rewrite(&paths, &lines).expect("seed queue");
    assert!(
        !paths.telemetry_last_flush_attempt().exists(),
        "no throttle stamp before the loop starts"
    );

    // Drive the exit-hook decision SEQUENCE ~5 times in a tight loop, no clock
    // advance. Only the FIRST iteration may spawn; the rest are throttled out.
    let mut spawn_decisions = 0usize;
    let mut stamp_after_first: Option<String> = None;
    for iter in 0..5 {
        if telemetry::should_spawn(&paths) {
            spawn_decisions += 1;
            telemetry::record_attempt(&paths);
            if iter == 0 {
                stamp_after_first = Some(
                    std::fs::read_to_string(paths.telemetry_last_flush_attempt())
                        .expect("stamp after first record_attempt"),
                );
            }
        }
        // The stamp NEVER changes after iteration 0: every later `should_spawn` is
        // throttled (false) so `record_attempt` never re-runs.
        if iter > 0 {
            let now = std::fs::read_to_string(paths.telemetry_last_flush_attempt())
                .expect("stamp persists");
            assert_eq!(
                now,
                *stamp_after_first.as_ref().unwrap(),
                "iteration {iter}: the throttle stamp must not advance within the window"
            );
        }
    }

    assert_eq!(
        spawn_decisions, 1,
        "exactly ONE spawn decision fired across the windowed loop (≤1 flusher/window)"
    );

    // The window reopened? Advance the clock 61 s past the stamp and re-check.
    drop(clk);
    let _clk2 = ClockGuard::install(t0 + time::Duration::seconds(61));
    assert!(
        telemetry::should_spawn(&paths),
        "past the 60 s throttle window the decision reopens (true again)"
    );
}

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

use crate::common::ToolEnv;

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

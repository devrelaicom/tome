//! Phase 10 / US2 — `tome telemetry inspect` driven through the REAL `tome`
//! binary over an isolated `$HOME` (`ToolEnv`).
//!
//! `inspect` pretty-prints the PENDING queue WITHOUT sending it, leaves the
//! queue file byte-identical (it NEVER repairs — the flusher self-heals on
//! drain), reports corrupt/unparsable lines, and exits **92**
//! (`TelemetryQueueCorrupt`) when any unparsable line exists (after the report).
//!
//! **Env hygiene is mandatory.** Like the `identity` suite, `ToolEnv::cmd()`
//! inherits the parent env, so every spawned `Command` must clear the CI +
//! `TOME_TELEMETRY*` overrides — otherwise a CI run would skew the baseline.
//! `inspect` is read-only and consent-independent (it only reads the queue
//! file), but we keep the same hygiene so the suite is deterministic.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::process::Command;

use serde_json::Value;

use crate::common::ToolEnv;

/// Every env var that can flip the telemetry enabled-state precedence. Cleared
/// on every spawned `Command` for a deterministic baseline. Mirrors the
/// `identity` suite's list.
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

/// Build a `tome` command over the isolated `$HOME` with every telemetry/CI env
/// var removed.
fn clean_cmd(env: &ToolEnv) -> Command {
    let mut cmd = env.cmd();
    for &k in TELEMETRY_ENV_VARS {
        cmd.env_remove(k);
    }
    cmd
}

/// Run `clean_cmd` with the given args, asserting the process spawned.
fn run(env: &ToolEnv, args: &[&str]) -> std::process::Output {
    clean_cmd(env)
        .args(args)
        .output()
        .expect("spawn tome binary")
}

/// The `telemetry/queue.jsonl` file path under the isolated home.
fn queue_path(env: &ToolEnv) -> std::path::PathBuf {
    env.tome_root().join("telemetry").join("queue.jsonl")
}

/// Seed the queue file with `body`, creating `telemetry/` first.
fn seed_queue(env: &ToolEnv, body: &str) {
    let q = queue_path(env);
    std::fs::create_dir_all(q.parent().unwrap()).expect("create telemetry dir");
    std::fs::write(&q, body).expect("seed queue");
}

/// Hash the raw queue-file bytes (or `None` if absent) so a before/after compare
/// proves byte-identity without re-printing the whole body on failure.
fn queue_digest(env: &ToolEnv) -> Option<u64> {
    let bytes = std::fs::read(queue_path(env)).ok()?;
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    Some(h.finish())
}

// ---------------------------------------------------------------------------
// empty queue — 0 pending, exit 0
// ---------------------------------------------------------------------------

#[test]
fn inspect_empty_queue_is_exit_0_zero_pending() {
    let env = ToolEnv::new();
    let out = run(&env, &["telemetry", "inspect"]);
    assert!(
        out.status.success(),
        "inspect on an empty queue must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("pending: 0"),
        "inspect must report 0 pending on an empty queue; stdout: {stdout}"
    );
}

// ---------------------------------------------------------------------------
// two valid events — --json reports pending==2, corrupt==0, FIFO order; the
// queue file is byte-identical before/after (read-only).
// ---------------------------------------------------------------------------

#[test]
fn inspect_json_two_valid_events_is_read_only() {
    let env = ToolEnv::new();
    // Two distinct, fixed valid event lines.
    seed_queue(
        &env,
        "{\"event_type\":\"tome.cold_start\",\"n\":1}\n{\"event_type\":\"tome.command\",\"n\":2}\n",
    );

    let before = queue_digest(&env).expect("queue exists before inspect");

    let out = run(&env, &["telemetry", "inspect", "--json"]);
    assert!(
        out.status.success(),
        "inspect --json with all-valid lines must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    let v: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "inspect --json is not valid JSON: {e}; stdout: {}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    assert_eq!(v["pending"], 2, "two parsable events: {v}");
    assert_eq!(v["corrupt"], 0, "no corrupt lines: {v}");

    let events = v["events"].as_array().expect("events is an array");
    assert_eq!(events.len(), 2, "events array length: {v}");
    // FIFO (queue) order preserved, values embedded as-is.
    assert_eq!(events[0]["event_type"], "tome.cold_start");
    assert_eq!(events[0]["n"], 1);
    assert_eq!(events[1]["event_type"], "tome.command");
    assert_eq!(events[1]["n"], 2);

    // Read-only proof: the queue file bytes are IDENTICAL after inspect.
    let after = queue_digest(&env).expect("queue still exists after inspect");
    assert_eq!(
        before, after,
        "inspect must NOT mutate the queue file (read-only)"
    );
}

// ---------------------------------------------------------------------------
// one valid + one corrupt line — exit 92; the valid event is still listed; the
// queue is byte-identical after (NOT repaired).
// ---------------------------------------------------------------------------

#[test]
fn inspect_with_corrupt_line_is_exit_92_and_does_not_repair() {
    let env = ToolEnv::new();
    // One valid JSON event, one non-JSON (corrupt) line.
    let seeded = "{\"event_type\":\"tome.command\",\"ok\":true}\nthis is not json\n";
    seed_queue(&env, seeded);

    let before = queue_digest(&env).expect("queue exists before inspect");

    let out = run(&env, &["telemetry", "inspect"]);
    assert_eq!(
        out.status.code(),
        Some(92),
        "a corrupt line must surface TelemetryQueueCorrupt (exit 92); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // The report still printed FIRST: the valid event is listed and the corrupt
    // count is surfaced.
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("pending: 1"),
        "the one valid event must still be reported; stdout: {stdout}"
    );
    assert!(
        stdout.contains("tome.command"),
        "the valid event must be listed despite the corrupt line; stdout: {stdout}"
    );
    assert!(
        stdout.contains("unparsable"),
        "the corrupt line must be reported; stdout: {stdout}"
    );

    // Read-only / never-repair, restored to the STRICT form (C-H1): the queue
    // file is byte-for-byte IDENTICAL after inspect. The `tome telemetry`
    // control surface is now excluded from the `tome.error` boundary emit (see
    // `main.rs` `is_telemetry_cmd`), so the exit-92 corrupt-queue report no
    // longer appends a self-referential `tome.error` line to the very file it
    // just reported. inspect leaves the corrupt line exactly where it was for
    // the flusher to self-heal later — and touches nothing else.
    let after = queue_digest(&env).expect("queue still exists after inspect");
    assert_eq!(
        before, after,
        "inspect must NOT mutate the queue file at all (byte-identical, corrupt line preserved)"
    );
}

// ---------------------------------------------------------------------------
// --json byte-stable shape pin for a fixed seeded queue. The seeded lines are
// fixed strings, so the emitted JSON is fully deterministic.
// ---------------------------------------------------------------------------

#[test]
fn inspect_json_shape_is_byte_stable() {
    let env = ToolEnv::new();
    seed_queue(
        &env,
        "{\"event_type\":\"tome.cold_start\",\"v\":1}\n{\"event_type\":\"tome.command\",\"v\":2}\n",
    );

    let out = run(&env, &["telemetry", "inspect", "--json"]);
    assert!(out.status.success(), "all-valid inspect --json exits 0");

    // Field order: pending, corrupt, events — events preserves FIFO order and
    // embeds the parsed values verbatim (object key order within each value is
    // serde_json's preserve_order from the parsed input).
    let expected = "{\"pending\":2,\"corrupt\":0,\"events\":[\
{\"event_type\":\"tome.cold_start\",\"v\":1},\
{\"event_type\":\"tome.command\",\"v\":2}]}";
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        stdout.trim_end(),
        expected,
        "inspect --json wire shape drifted; stdout: {stdout}"
    );
}

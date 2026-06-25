//! Phase 10 / US1 — telemetry CLI exit codes + byte-stable `status --json`
//! pins, driven through the REAL `tome` binary over an isolated `$HOME`.
//!
//! The closed-set round-trip table (every `TomeError` variant → code → slug)
//! lives in `tests/index_query_misc/exit_codes.rs`; this file covers the
//! telemetry surfaces that only manifest end-to-end: the malformed-config exit
//! code and the deterministic (uuid-free) `status --json` wire shapes.
//!
//! **Task-3 note (unified-global-config):** telemetry opt-out moved from the
//! old `telemetry/config.toml` into `config.toml [telemetry] enabled`. A
//! malformed `config.toml` now surfaces as `ManifestInvalid::TomlParse`
//! (exit 5), consistent with the unified config policy. `TelemetryConfigInvalid`
//! (exit 91) is vestigial (the variant is kept in `error.rs` so the closed-set
//! coverage test keeps passing, but it is never constructed by this code path).

use std::process::Command;

use crate::common::ToolEnv;
use crate::queue_util::TELEMETRY_ENV_VARS;

fn clean_cmd(env: &ToolEnv) -> Command {
    let mut cmd = env.cmd();
    for &k in TELEMETRY_ENV_VARS {
        cmd.env_remove(k);
    }
    cmd
}

/// Write a `config.toml` (unified config) under the isolated home.
fn write_config(env: &ToolEnv, body: &str) {
    let root = env.tome_root();
    std::fs::create_dir_all(&root).expect("create tome root dir");
    std::fs::write(root.join("config.toml"), body).expect("write config.toml");
}

// ---------------------------------------------------------------------------
// 5 — malformed config.toml surfaces as ManifestInvalid (exit 5) on the
//     foreground telemetry CLI path (Task-3: old exit 91 is vestigial).
// ---------------------------------------------------------------------------

#[test]
fn malformed_config_toml_is_exit_5() {
    let env = ToolEnv::new();
    // A wrong-typed value for `[telemetry] enabled` causes the unified
    // config.toml strict parse to fail → ManifestInvalid::TomlParse (exit 5).
    // CI + TOME_TELEMETRY are cleared (via clean_cmd) so the resolver actually
    // reaches the file.
    write_config(&env, "[telemetry]\nenabled = \"not a bool\"\n");

    let out = clean_cmd(&env)
        .args(["telemetry", "status"])
        .output()
        .expect("spawn tome");
    assert_eq!(
        out.status.code(),
        Some(5),
        "malformed config.toml must be ManifestInvalid (5); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

// ---------------------------------------------------------------------------
// #225 negative control — the test harness must fork ZERO detached flushers
// ---------------------------------------------------------------------------

/// A `tome` invocation under the DEFAULT `ToolEnv` (telemetry force-disabled by
/// `cmd()`) must leave no telemetry footprint. The load-bearing assertion is the
/// absence of `telemetry/last-flush-attempt`: `teardown_at_exit` writes that
/// stamp ONLY on the path where it is about to fork a detached `telemetry flush`
/// child, so its absence proves the exit hook never spawned a flusher — the #225
/// storm cannot occur. (`teardown_at_exit` runs on both the success and error
/// exit arms, so `catalog list` exercises it regardless of its exit code.)
#[test]
fn default_harness_forks_no_detached_flusher() {
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["catalog", "list"])
        .output()
        .expect("spawn tome catalog list");
    // It actually ran through the normal dispatch (so teardown_at_exit ran too).
    assert!(
        out.status.code().is_some(),
        "command ran to completion; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    let tel = env.tome_root().join("telemetry");
    assert!(
        !tel.join("last-flush-attempt").exists(),
        "disabled telemetry must not stamp a spawn attempt (#225: no detached flusher forked)",
    );
    assert!(
        !tel.join("id").exists(),
        "disabled telemetry mints no install id",
    );
    assert!(
        !tel.join("queue.jsonl").exists(),
        "disabled telemetry enqueues nothing",
    );
}

// ---------------------------------------------------------------------------
// status --json byte-stable pins (deterministic, uuid-free states only)
// ---------------------------------------------------------------------------

/// Run `telemetry status --json` and return stdout with a single trailing
/// newline trimmed (the `write_json` emitter appends exactly one `\n`).
fn status_json_bytes(cmd: &mut Command) -> String {
    let out = cmd.output().expect("spawn tome");
    assert!(
        out.status.success(),
        "status --json exited {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let s = String::from_utf8(out.stdout).expect("status --json is utf-8");
    s.strip_suffix('\n').unwrap_or(&s).to_string()
}

#[test]
fn status_json_fresh_default_has_stable_fields_and_a_minted_uuid() {
    let env = ToolEnv::new();
    // Default-on (CI/TOME_TELEMETRY cleared): `init` builds an ENABLED handle and
    // mints the install id, so `status --json` surfaces a fresh `install_uuid`.
    // The uuid is non-deterministic, so we pin the deterministic fields + the
    // default kernel endpoint and assert the uuid is present + shaped.
    let body = status_json_bytes(clean_cmd(&env).args(["telemetry", "status", "--json"]));
    let v: serde_json::Value = serde_json::from_str(&body).expect("status --json is JSON");
    assert_eq!(v["enabled"], serde_json::Value::Bool(true));
    assert_eq!(v["source"], "default");
    assert_eq!(v["endpoint"], "https://gauge-telemetry.fly.dev");
    assert_eq!(v["pending"], 0);
    let uuid = v["install_uuid"]
        .as_str()
        .expect("a default-on status surfaces the minted install_uuid");
    assert_eq!(
        uuid.split('-').count(),
        5,
        "install_uuid is uuid-shaped: {uuid}"
    );
}

#[test]
fn status_json_ci_disabled_is_byte_stable() {
    let env = ToolEnv::new();
    // Under CI the kernel auto-disables and mints nothing, so the wire shape is
    // fully deterministic (no install_uuid).
    let body = status_json_bytes(clean_cmd(&env).env("CI", "true").args([
        "telemetry",
        "status",
        "--json",
    ]));
    assert_eq!(
        body,
        r#"{"enabled":false,"source":"ci","endpoint":"https://gauge-telemetry.fly.dev","pending":0}"#,
    );
}

// ---------------------------------------------------------------------------
// last-flush stamp surfaced by `status --json` (Co-M1)
// ---------------------------------------------------------------------------

/// Write a `telemetry/last-flush` stamp under the isolated home with the exact
/// shape the flusher emits: `{"timestamp":...,"last_status":<u16|null>}`.
fn write_last_flush(env: &ToolEnv, body: &str) {
    let dir = env.tome_root().join("telemetry");
    std::fs::create_dir_all(&dir).expect("create telemetry dir");
    std::fs::write(dir.join("last-flush"), body).expect("write last-flush stamp");
}

#[test]
fn status_json_surfaces_last_flush_with_status() {
    let env = ToolEnv::new();
    // A successful-delivery stamp: a concrete 2xx status.
    write_last_flush(
        &env,
        "{\"timestamp\":\"2026-06-11T12:00:00.000Z\",\"last_status\":200}\n",
    );
    let body = status_json_bytes(clean_cmd(&env).args(["telemetry", "status", "--json"]));
    let v: serde_json::Value = serde_json::from_str(&body).expect("status --json is JSON");
    let lf = v.get("last_flush").expect("last_flush present");
    assert_eq!(
        lf.get("timestamp").and_then(|t| t.as_str()),
        Some("2026-06-11T12:00:00.000Z"),
    );
    assert_eq!(lf.get("last_status").and_then(|s| s.as_u64()), Some(200));
}

#[test]
fn status_json_surfaces_last_flush_null_status() {
    let env = ToolEnv::new();
    // A drain that ran but acknowledged nothing: `last_status` is JSON null.
    write_last_flush(
        &env,
        "{\"timestamp\":\"2026-06-11T12:00:00.000Z\",\"last_status\":null}\n",
    );
    let body = status_json_bytes(clean_cmd(&env).args(["telemetry", "status", "--json"]));
    let v: serde_json::Value = serde_json::from_str(&body).expect("status --json is JSON");
    let lf = v.get("last_flush").expect("last_flush present");
    // We PIN that a null status is EMITTED (not omitted) as JSON null.
    assert!(
        lf.get("last_status").map(|s| s.is_null()).unwrap_or(false),
        "last_status must be present and null: {lf}"
    );
}

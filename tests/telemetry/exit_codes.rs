//! Phase 10 / US1 — telemetry CLI exit codes + byte-stable `status --json`
//! pins, driven through the REAL `tome` binary over an isolated `$HOME`.
//!
//! The closed-set round-trip table (every `TomeError` variant → code → slug)
//! lives in `tests/index_query_misc/exit_codes.rs`; this file covers the two
//! telemetry surfaces that only manifest end-to-end: the malformed-config exit
//! 91 and the deterministic (uuid-free) `status --json` wire shapes.

use std::process::Command;

use crate::common::ToolEnv;

/// Every env var that can flip the telemetry enabled-state precedence — cleared
/// on every spawned command so the resolver reaches a deterministic state
/// regardless of the host/CI environment.
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

fn clean_cmd(env: &ToolEnv) -> Command {
    let mut cmd = env.cmd();
    for &k in TELEMETRY_ENV_VARS {
        cmd.env_remove(k);
    }
    cmd
}

/// Write a `telemetry/config.toml` under the isolated home.
fn write_config(env: &ToolEnv, body: &str) {
    let dir = env.tome_root().join("telemetry");
    std::fs::create_dir_all(&dir).expect("create telemetry dir");
    std::fs::write(dir.join("config.toml"), body).expect("write telemetry config");
}

// ---------------------------------------------------------------------------
// 91 — malformed telemetry config surfaces loudly on the foreground CLI
// ---------------------------------------------------------------------------

#[test]
fn malformed_config_is_exit_91() {
    let env = ToolEnv::new();
    // A wrong-typed value for `enabled` fails the strict (deny_unknown_fields,
    // typed) parse → TelemetryConfigInvalid (exit 91). CI + TOME_TELEMETRY are
    // cleared (via clean_cmd) so the resolver actually reaches the file.
    write_config(&env, "enabled = \"not a bool\"\n");

    let out = clean_cmd(&env)
        .args(["telemetry", "status"])
        .output()
        .expect("spawn tome");
    assert_eq!(
        out.status.code(),
        Some(91),
        "malformed config must be TelemetryConfigInvalid (91); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
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
fn status_json_fresh_default_is_byte_stable() {
    let env = ToolEnv::new();
    // No id minted (status is read-only) ⇒ no install_uuid; default endpoint
    // (TOME_TELEMETRY_ENDPOINT cleared); opt-out default-on.
    let body = status_json_bytes(clean_cmd(&env).args(["telemetry", "status", "--json"]));
    assert_eq!(
        body,
        r#"{"enabled":true,"source":"default","endpoint":"https://telemetry.tome-mcp.app/v1/events","pending":0}"#,
    );
}

#[test]
fn status_json_ci_disabled_is_byte_stable() {
    let env = ToolEnv::new();
    let body = status_json_bytes(clean_cmd(&env).env("CI", "true").args([
        "telemetry",
        "status",
        "--json",
    ]));
    assert_eq!(
        body,
        r#"{"enabled":false,"source":"ci","endpoint":"https://telemetry.tome-mcp.app/v1/events","pending":0}"#,
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

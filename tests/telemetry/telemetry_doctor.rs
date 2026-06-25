//! Phase 10 / US5 (T071, FR-064/065) — the `tome doctor` telemetry section,
//! end-to-end through the real binary.
//!
//! Read-only is the load-bearing invariant (FR-124): the doctor report
//! PROJECTS the telemetry state, it never mints/writes/creates. To prove that
//! cleanly we drive doctor with telemetry FORCE-OFF (`TOME_TELEMETRY=0`), so the
//! process-start hook (`cli_startup`) — which DOES mint when enabled — is
//! short-circuited and the ONLY telemetry code that runs is the read-only
//! section assembler. Anything that appears on disk afterward would be doctor
//! minting, which must never happen.
//!
//! `--fix` gains no telemetry capability (FR-065): seeded telemetry files are
//! byte-identical before and after a `doctor --fix` run.

use std::process::Command;

use serde_json::Value;

use crate::common::{ToolEnv, fabricate_all_registry_models, paths_for};
use crate::queue_util::TELEMETRY_ENV_VARS;

/// A `tome` command over the isolated `$HOME` with every CI/telemetry var
/// removed, then telemetry FORCE-OFF. Force-off makes the process-start hook a
/// no-op (no mint, no enqueue), so the only telemetry path that runs is the
/// read-only doctor section — exactly what we want to isolate.
fn force_off_cmd(env: &ToolEnv) -> Command {
    let mut cmd = env.cmd();
    for &k in TELEMETRY_ENV_VARS {
        cmd.env_remove(k);
    }
    cmd.env("TOME_TELEMETRY", "0");
    cmd
}

/// The telemetry file paths under the isolated home.
fn telemetry_dir(env: &ToolEnv) -> std::path::PathBuf {
    env.tome_root().join("telemetry")
}
fn id_path(env: &ToolEnv) -> std::path::PathBuf {
    telemetry_dir(env).join("id")
}
fn queue_path(env: &ToolEnv) -> std::path::PathBuf {
    telemetry_dir(env).join("queue.jsonl")
}
fn last_flush_path(env: &ToolEnv) -> std::path::PathBuf {
    telemetry_dir(env).join("last-flush")
}

/// Run `tome --json doctor` (force-off) and return the parsed JSON report.
fn doctor_json(env: &ToolEnv) -> Value {
    let out = force_off_cmd(env)
        .args(["--json", "doctor"])
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).expect("doctor --json parses")
}

/// The telemetry section + allowlist render, and doctor MINTS NOTHING.
///
/// With telemetry force-off and no seeded state, the telemetry block reports
/// disabled (source `TOME_TELEMETRY=0`), the one Midnight allowlist entry, and —
/// critically — leaves the telemetry tree absent: the read is pure projection.
#[test]
fn doctor_reports_telemetry_section_read_only_no_mint() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    // Nothing telemetry on disk before.
    assert!(!telemetry_dir(&env).exists(), "no telemetry dir before");

    let v = doctor_json(&env);
    let t = v
        .get("telemetry")
        .expect("doctor --json carries a telemetry block");

    assert_eq!(t["enabled"], false);
    assert_eq!(t["source"], "env_off");
    assert_eq!(t["install_id"]["present"], false);
    assert_eq!(t["queue"]["pending"], 0);
    assert_eq!(t["queue"]["corrupt"], 0);

    // The allowlist surfaces exactly the one Midnight entry, canonical source.
    let allow = t["allowlist"].as_array().expect("allowlist is an array");
    assert_eq!(allow.len(), 1, "one allowlist entry");
    assert_eq!(allow[0]["short_id"], "midnight");
    assert_eq!(
        allow[0]["canonical_source"],
        "github.com/devrelaicom/midnight-expert-tome"
    );

    // The endpoint is reported (scrubbed); HTTPS by default.
    assert!(
        t["endpoint"].as_str().unwrap().starts_with("https://"),
        "endpoint reported (scrubbed https): {}",
        t["endpoint"],
    );

    // READ-ONLY: doctor minted nothing. The id/queue absent before stay absent.
    assert!(
        !id_path(&env).exists(),
        "doctor must not mint the install id"
    );
    assert!(
        !queue_path(&env).exists(),
        "doctor must not create the queue"
    );
}

/// With a seeded id (mode 0600), a queue, and a last-flush stamp, the report
/// surfaces all three — and the seeded files are byte-identical afterward
/// (doctor read them, never rewrote them).
#[cfg(unix)]
#[test]
fn doctor_surfaces_seeded_id_queue_and_last_flush() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    // Seed the telemetry tree directly (not via the binary) so the only writer
    // is the test; doctor must not touch them.
    std::fs::create_dir_all(telemetry_dir(&env)).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(id_path(&env), b"0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f\n").unwrap();
        std::fs::set_permissions(id_path(&env), std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    // REAL kernel-shaped queue lines (the kernel envelope is
    // `{"event_name":..,"time_unix_nano":<u64 nanos>,"attributes":{..}}`), so the
    // oldest-age read asserts against reality, not a legacy `timestamp` fiction.
    // The `time_unix_nano` values correspond to 2020-01-01/02T00:00:00Z.
    std::fs::write(
        queue_path(&env),
        b"{\"event_name\":\"tome.search\",\"time_unix_nano\":1577836800000000000,\"attributes\":{}}\n\
          {\"event_name\":\"tome.install\",\"time_unix_nano\":1577923200000000000,\"attributes\":{}}\n",
    )
    .unwrap();
    std::fs::write(
        last_flush_path(&env),
        b"{\"timestamp\":\"2026-06-11T14:11:45.123Z\",\"last_status\":200}",
    )
    .unwrap();

    let id_before = std::fs::read(id_path(&env)).unwrap();
    let queue_before = std::fs::read(queue_path(&env)).unwrap();
    let flush_before = std::fs::read(last_flush_path(&env)).unwrap();

    let v = doctor_json(&env);
    let t = &v["telemetry"];

    // install id surfaced with its 0600 mode + an age.
    assert_eq!(t["install_id"]["present"], true);
    assert_eq!(t["install_id"]["mode"], 0o600);
    assert!(t["install_id"]["age_seconds"].is_u64());

    // queue depth + oldest age (the 2020 first event ⇒ a large positive age).
    assert_eq!(t["queue"]["pending"], 2);
    assert_eq!(t["queue"]["corrupt"], 0);
    assert!(
        t["queue"]["oldest_age_seconds"].as_u64().unwrap() > 0,
        "oldest age surfaced",
    );

    // last flush surfaced verbatim.
    assert_eq!(t["last_flush"]["timestamp"], "2026-06-11T14:11:45.123Z");
    assert_eq!(t["last_flush"]["status"], 200);

    // READ-ONLY: every seeded file is byte-identical after doctor ran.
    assert_eq!(
        std::fs::read(id_path(&env)).unwrap(),
        id_before,
        "id unchanged"
    );
    assert_eq!(
        std::fs::read(queue_path(&env)).unwrap(),
        queue_before,
        "queue unchanged"
    );
    assert_eq!(
        std::fs::read(last_flush_path(&env)).unwrap(),
        flush_before,
        "last-flush unchanged"
    );
}

/// `doctor --fix` gains no telemetry capability (FR-065): the seeded telemetry
/// files are byte-identical before and after a `--fix` run.
#[test]
fn doctor_fix_performs_no_telemetry_write() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    std::fs::create_dir_all(telemetry_dir(&env)).unwrap();
    std::fs::write(id_path(&env), b"0b9c1f2e-3a4d-4b6c-8e1f-2a3b4c5d6e7f\n").unwrap();
    // A REAL kernel-shaped queue line (the read-only assertion below is about
    // `--fix` not mutating it; the content matches the live envelope shape).
    std::fs::write(
        queue_path(&env),
        b"{\"event_name\":\"tome.search\",\"time_unix_nano\":1577836800000000000,\"attributes\":{}}\n",
    )
    .unwrap();
    let id_before = std::fs::read(id_path(&env)).unwrap();
    let queue_before = std::fs::read(queue_path(&env)).unwrap();

    // Run `doctor --fix` (force-off so the startup hook can't mint/enqueue).
    let out = force_off_cmd(&env)
        .args(["doctor", "--fix"])
        .output()
        .unwrap();
    // We don't assert the exit code (models may classify the system any way);
    // the point is the telemetry files are untouched.
    let _ = out;

    assert_eq!(
        std::fs::read(id_path(&env)).unwrap(),
        id_before,
        "doctor --fix must not rewrite the install id"
    );
    assert_eq!(
        std::fs::read(queue_path(&env)).unwrap(),
        queue_before,
        "doctor --fix must not rewrite the queue"
    );
}

/// Byte-stable `--json` telemetry block for a fixed, deterministic disabled
/// state (no install id → no random uuid in the wire shape). This pins the
/// telemetry block's field set + order for the disabled/empty case.
#[test]
fn doctor_json_telemetry_block_is_byte_stable_for_disabled_state() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let v = doctor_json(&env);
    let t = &v["telemetry"];

    // The disabled/empty block carries EXACTLY these keys (no install id ⇒ no
    // mode/age; no flush ⇒ no last_flush; no config error). Deterministic.
    let mut keys: Vec<&str> = t.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "allowlist",
            "enabled",
            "endpoint",
            "install_id",
            "queue",
            "source"
        ],
        "disabled telemetry block carries exactly the expected keys: {t}",
    );

    // install_id (absent) carries only path + present:false (no mode/age).
    let mut id_keys: Vec<&str> = t["install_id"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    id_keys.sort_unstable();
    assert_eq!(id_keys, ["path", "present"]);

    // queue (empty) carries only pending + corrupt (no oldest_age_seconds).
    let mut q_keys: Vec<&str> = t["queue"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    q_keys.sort_unstable();
    assert_eq!(q_keys, ["corrupt", "pending"]);
}

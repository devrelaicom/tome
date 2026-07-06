//! Issue #434 — the `tome doctor --verify` end-to-end MCP probe.
//!
//! Three layers:
//!
//! 1. **Protocol mechanics** against a scripted MCP server (a `/bin/sh`
//!    stand-in that answers `initialize` and `tools/list`): the probe
//!    completes the round-trip and counts tools. A REAL `tome mcp` success
//!    needs real model weights (preflight verifies checksums — the
//!    `#[ignore]`d release-gate territory), so the success path is proven at
//!    the protocol level here.
//! 2. **Timeout honesty** against a server that never responds (`/bin/cat`):
//!    the probe reports failure within its bound instead of hanging.
//! 3. **Real binary, real doctor**: `tome doctor --verify --json` in a seeded
//!    project spawns the actual `tome mcp` argv sync would register; on a
//!    modelless test install the preflight refusal is REPORTED (ok=false with
//!    the stderr tail) rather than hanging or crashing doctor — and the field
//!    is absent entirely without `--verify`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::common::{ToolEnv, paths_for, seed_workspace};

fn write_script(dir: &std::path::Path, body: &str) -> PathBuf {
    let path = dir.join("fake-mcp.sh");
    std::fs::write(&path, body).expect("write script");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod +x");
    }
    path
}

#[cfg(unix)]
#[test]
fn probe_completes_round_trip_against_scripted_server() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Line 1: initialize request → id-1 response. Lines 2+3: the initialized
    // notification and the tools/list request → id-2 response with 3 tools.
    let script = write_script(
        tmp.path(),
        r#"#!/bin/sh
read _init
printf '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{},"serverInfo":{"name":"fake","version":"0"}}}\n'
read _initialized
read _tools_list
printf '{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"a"},{"name":"b"},{"name":"c"}]}}\n'
"#,
    );

    let tools = tome::doctor::mcp_probe::probe_command(&script, &[], Duration::from_secs(5))
        .expect("round trip succeeds");
    assert_eq!(tools, 3, "tools/list count surfaced");
}

#[cfg(unix)]
#[test]
fn probe_times_out_against_unresponsive_server_without_hanging() {
    let tmp = tempfile::TempDir::new().unwrap();
    // Consumes stdin forever, never answers — the wedged-server case.
    let script = write_script(tmp.path(), "#!/bin/sh\nwhile read _line; do :; done\n");
    let started = Instant::now();
    let err = tome::doctor::mcp_probe::probe_command(&script, &[], Duration::from_millis(400))
        .expect_err("a silent server never answers");
    assert!(
        err.contains("timed out"),
        "failure names the timeout: {err}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the probe must return promptly after its bound, took {:?}",
        started.elapsed(),
    );
}

#[cfg(unix)]
#[test]
fn probe_reports_stderr_tail_when_server_exits() {
    let tmp = tempfile::TempDir::new().unwrap();
    let script = write_script(
        tmp.path(),
        "#!/bin/sh\necho 'boom: preflight refused' >&2\nexit 51\n",
    );
    let err = tome::doctor::mcp_probe::probe_command(&script, &[], Duration::from_secs(5))
        .expect_err("server died before responding");
    assert!(
        err.contains("exited before responding") || err.contains("timed out"),
        "failure names the early exit: {err}",
    );
    assert!(
        err.contains("boom: preflight refused"),
        "the stderr tail rides the failure report: {err}",
    );
}

/// The real doctor path: `--verify` in a seeded project probes the exact argv
/// sync registers, and a modelless install's preflight refusal is REPORTED,
/// not fatal. Without `--verify` the field never appears.
#[test]
fn doctor_verify_probes_and_reports_failure_on_modelless_install() {
    let _override_lock = crate::common::HARNESS_OVERRIDE_MUTEX
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    seed_workspace(&paths, "test-workspace");

    let project = env.home_path().join("project");
    std::fs::create_dir_all(project.join(".tome")).unwrap();
    std::fs::write(
        project.join(".tome/config.toml"),
        "workspace = \"test-workspace\"\nharnesses = [\"cursor\"]\n",
    )
    .unwrap();

    // Without --verify: no probe, no field (the wire pin stays unchanged).
    let plain = env
        .cmd()
        .current_dir(&project)
        .args(["--json", "doctor"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&plain.stdout).expect("json");
    assert!(
        v.get("mcp_probe").is_none(),
        "mcp_probe must be absent without --verify",
    );

    // With --verify: one row per effective harness, spawning the real binary.
    let out = env
        .cmd()
        .current_dir(&project)
        .args(["--json", "doctor", "--verify"])
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let probe = v["mcp_probe"].as_array().expect("mcp_probe under --verify");
    assert_eq!(probe.len(), 1, "one row per effective harness: {probe:?}");
    let row = &probe[0];
    assert_eq!(row["harness"], "cursor");
    assert_eq!(row["workspace"], "test-workspace");
    // A modelless test install cannot start the real server; the probe
    // REPORTS that (with the preflight stderr tail) instead of hanging.
    assert_eq!(row["ok"], false);
    let err = row["error"].as_str().expect("failure carries an error");
    assert!(!err.is_empty());
}

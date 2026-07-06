//! End-to-end MCP server probe for `tome doctor --verify` (issue #434).
//!
//! For each harness in the scope-effective list, spawn the EXACT `tome mcp`
//! command sync registers for it (the argv comes from
//! [`crate::harness::sync::expected_tome_entry`] — the same SSOT the writer
//! persists, never hand-assembled) and perform a real `initialize` →
//! `notifications/initialized` → `tools/list` round-trip over stdio, bounded
//! by a timeout. Reports ok (with the tool count) or failed (with the reason
//! plus a credential-scrubbed stderr tail).
//!
//! Only runs when verification is enabled — the `--verify` flag or
//! `[doctor] verify_by_default` in `~/.tome/config.toml`: it is network-free
//! but spawns a real subprocess per harness, so it is deliberately not part
//! of the default read-only report. Sync-only — `std::process` + reader threads for the
//! timeout, no `tokio` (the sync boundary holds; the async island stays
//! `src/mcp/`).

use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::doctor::report::McpProbeEntry;
use crate::settings::resolver::EffectiveHarnessList;
use crate::workspace::WorkspaceName;

/// The per-server round-trip budget. A healthy `tome mcp` answers
/// `initialize` + `tools/list` well inside this; a wedged one is killed and
/// reported rather than hanging doctor.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// How much trailing stderr to keep for a failure report.
const STDERR_TAIL_MAX: usize = 2048;

/// Probe every harness in the scope-effective list. One row per harness,
/// mirroring the `harness_mcp` report rows. `None`-equivalent (empty) when the
/// effective list is absent/empty — the caller omits the field.
pub(crate) fn probe_effective_harnesses(
    workspace_name: &WorkspaceName,
    effective: Option<&EffectiveHarnessList>,
) -> Vec<McpProbeEntry> {
    let Some(list) = effective else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for h in &list.harnesses {
        let entry = crate::harness::sync::expected_tome_entry(workspace_name, &h.name);
        let result = probe_command(Path::new(&entry.command), &entry.args, PROBE_TIMEOUT);
        out.push(McpProbeEntry {
            harness: h.name.clone(),
            workspace: workspace_name.as_str().to_string(),
            ok: result.is_ok(),
            tools: result.as_ref().ok().copied(),
            error: result.err(),
        });
    }
    out
}

/// Spawn `command args…` and drive one MCP stdio round-trip: `initialize`
/// (id 1) → `notifications/initialized` → `tools/list` (id 2). Returns the
/// advertised tool count on success; on failure, a reason string carrying the
/// credential-scrubbed stderr tail. The child is always killed + reaped —
/// a wedged server cannot outlive the probe.
///
/// `pub` (not `pub(crate)`) so the integration suite can drive the protocol
/// mechanics against a scripted server without spawning the full doctor.
pub fn probe_command(command: &Path, args: &[String], timeout: Duration) -> Result<u32, String> {
    let deadline = Instant::now() + timeout;

    let mut child = Command::new(command)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn failed: {e}"))?;

    // Drain stderr CONCURRENTLY from the start (the #318 pipe-buffer lesson:
    // a child blocked on a full, undrained stderr pipe stalls the round-trip
    // and turns a chatty-but-healthy server into a false timeout). Bounded:
    // only the trailing window is kept.
    let stderr_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let mut stderr_reader = None;
    if let Some(mut stderr) = child.stderr.take() {
        let sink = Arc::clone(&stderr_buf);
        stderr_reader = Some(std::thread::spawn(move || {
            let mut chunk = [0u8; 4096];
            while let Ok(n) = stderr.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let mut buf = sink.lock().unwrap_or_else(|e| e.into_inner());
                buf.extend_from_slice(&chunk[..n]);
                // Keep only the trailing window (plus slack for a UTF-8 cut).
                let excess = buf.len().saturating_sub(STDERR_TAIL_MAX * 2);
                if excess > 0 {
                    buf.drain(..excess);
                }
            }
        }));
    }

    let result = drive_round_trip(&mut child, deadline);

    // Kill first so `wait` cannot block on a healthy long-running server;
    // the reader threads end on the pipes' EOF. Join the stderr reader so
    // the tail below has the final flushed bytes (bounded: EOF follows the
    // kill).
    let _ = child.kill();
    let _ = child.wait();
    if let Some(handle) = stderr_reader {
        let _ = handle.join();
    }

    let stderr_tail = {
        let buf = stderr_buf.lock().unwrap_or_else(|e| e.into_inner());
        let text = String::from_utf8_lossy(&buf);
        let tail: String = text
            .chars()
            .skip(text.chars().count().saturating_sub(STDERR_TAIL_MAX))
            .collect();
        // Boundary rule: subprocess output passes through the scrub chokepoint
        // before it can reach any display path.
        String::from_utf8_lossy(&crate::catalog::git::scrub_credentials(
            tail.trim().as_bytes(),
        ))
        .into_owned()
    };

    result.map_err(|reason| {
        if stderr_tail.is_empty() {
            reason
        } else {
            format!("{reason}; stderr: {stderr_tail}")
        }
    })
}

/// The protocol half of [`probe_command`]: writes the three client messages
/// and awaits the two responses on a reader thread, honouring `deadline`.
fn drive_round_trip(child: &mut Child, deadline: Instant) -> Result<u32, String> {
    let mut stdin = child.stdin.take().ok_or("child stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("child stdout unavailable")?;

    // Reader thread: one line per message (MCP stdio framing). The channel
    // gives the main thread `recv_timeout` — the sync-safe timeout primitive.
    // The thread ends on EOF/error; it is detached deliberately (it holds only
    // the pipe, which closes when the child is killed by the caller).
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    // Issue #478: a stdin-write failure is NOT immediately fatal. A
    // fast-exiting server can close its stdin before our writes land (EPIPE)
    // while its EXIT is the real story — bailing on the write raced the
    // child's death and made the reported class flap between "write to server
    // stdin failed" and "exited before responding". Record the first write
    // failure and fall through to the response/exit detection: a dead child
    // classifies via the reader channel's disconnect ("server exited before
    // responding…"), a still-running server that merely closed stdin via the
    // timeout — one deterministic classification either way, with the write
    // failure appended as context.
    let mut write_failure: Option<String> = None;

    let init = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "tome-doctor", "version": env!("CARGO_PKG_VERSION")},
        },
    });
    send_line(&mut stdin, &init, &mut write_failure);
    let init_response =
        await_response(&rx, 1, deadline).map_err(|e| with_write_context(e, &write_failure))?;
    if init_response.get("result").is_none() {
        return Err(format!(
            "initialize returned an error: {}",
            init_response
                .get("error")
                .map(|e| e.to_string())
                .unwrap_or_else(|| "(no error body)".to_owned()),
        ));
    }

    let initialized = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized",
    });
    send_line(&mut stdin, &initialized, &mut write_failure);
    let tools_list = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
    });
    send_line(&mut stdin, &tools_list, &mut write_failure);

    let tools_response =
        await_response(&rx, 2, deadline).map_err(|e| with_write_context(e, &write_failure))?;
    let tools = tools_response
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(|t| t.as_array())
        .ok_or_else(|| {
            format!(
                "tools/list returned no tools array: {}",
                tools_response
                    .get("error")
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "(no error body)".to_owned()),
            )
        })?;
    Ok(u32::try_from(tools.len()).unwrap_or(u32::MAX))
}

/// [`write_line`] with issue-#478 fall-through semantics: the FIRST stdin
/// write failure is recorded, never propagated — classification stays with
/// the response/exit-detection machinery.
fn send_line(
    stdin: &mut impl Write,
    message: &serde_json::Value,
    write_failure: &mut Option<String>,
) {
    if let Err(e) = write_line(stdin, message)
        && write_failure.is_none()
    {
        *write_failure = Some(e);
    }
}

/// Append the recorded stdin-write failure (if any) to an await-side failure
/// reason, so the EPIPE context still surfaces without owning the
/// classification (issue #478).
fn with_write_context(reason: String, write_failure: &Option<String>) -> String {
    match write_failure {
        Some(w) => format!("{reason} ({w})"),
        None => reason,
    }
}

fn write_line(stdin: &mut impl Write, message: &serde_json::Value) -> Result<(), String> {
    let mut line = message.to_string();
    line.push('\n');
    stdin
        .write_all(line.as_bytes())
        .and_then(|()| stdin.flush())
        .map_err(|e| format!("write to server stdin failed: {e}"))
}

/// Read lines until one parses as a JSON-RPC message with the given `id`, or
/// the deadline passes / the stream ends. Non-matching lines (notifications,
/// stray output) are skipped.
fn await_response(
    rx: &mpsc::Receiver<String>,
    id: u64,
    deadline: Instant,
) -> Result<serde_json::Value, String> {
    loop {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| format!("timed out waiting for response id {id}"))?;
        match rx.recv_timeout(remaining) {
            Ok(line) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                // A RESPONSE carries `result` or `error` — never `method`.
                // The distinction matters against a misbehaving server that
                // echoes the request back (same `id`, but a request shape).
                let is_response = value.get("method").is_none()
                    && (value.get("result").is_some() || value.get("error").is_some());
                if is_response && value.get("id").and_then(|v| v.as_u64()) == Some(id) {
                    return Ok(value);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                return Err(format!("timed out waiting for response id {id}"));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(format!(
                    "server exited before responding to request id {id}"
                ));
            }
        }
    }
}

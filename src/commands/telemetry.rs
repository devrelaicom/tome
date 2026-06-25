//! `tome telemetry {status,inspect,on,off,reset,purge,flush}` — the user-facing
//! controls for the local-first telemetry subsystem, routed through the
//! `gauge-telemetry` kernel handle.
//!
//! Every subcommand here is a foreground, user-invoked command, so — unlike the
//! silent emit path — it surfaces config errors loudly (a malformed config →
//! exit 5 via [`config::resolve_enabled_with_source`]). Reports land on
//! **stdout**; the global `--json` flag (carried in `mode`) shapes them.
//! `status`/`inspect` are strictly read-only.
//!
//! The kernel owns the install id, the queue, and `reset`; this surface drives
//! the global [`crate::telemetry::handle`] for delivery (`flush`) and identity
//! reset, and edits `config.toml [telemetry] enabled` for the on/off switch.

use std::io::Write;

use serde::{Deserialize, Serialize};

use crate::cli::{TelemetryCommand, TelemetryFlushArgs, TelemetryResetArgs};
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::presentation::prompt;
use crate::telemetry::config::{self, Source};
use crate::util;
use crate::workspace::ResolvedScope;

/// Subcommand dispatcher invoked by `main.rs`.
pub fn run(cmd: TelemetryCommand, _scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    match cmd {
        TelemetryCommand::Status => status(&paths, mode),
        TelemetryCommand::Inspect => inspect_run(&paths, mode),
        TelemetryCommand::On => on(&paths, mode),
        TelemetryCommand::Off => off(&paths, mode),
        TelemetryCommand::Reset(args) => reset(&paths, args, mode),
        TelemetryCommand::Purge => purge(&paths, mode),
        TelemetryCommand::Flush(args) => flush_run(&paths, args, mode),
    }
}

// ---------------------------------------------------------------------------
// status — read-only report
// ---------------------------------------------------------------------------

/// The `tome telemetry status` record.
#[derive(Debug, Serialize)]
struct StatusReport {
    /// Whether telemetry is enabled for this install (resolved precedence).
    enabled: bool,
    /// Which precedence rule decided `enabled`.
    source: Source,
    /// The local install UUID, when present. Read-only: never minted by status.
    #[serde(skip_serializing_if = "Option::is_none")]
    install_uuid: Option<String>,
    /// The (scrubbed) collector endpoint events would be delivered to.
    endpoint: String,
    /// Queued, not-yet-delivered events (line count of the local JSONL queue).
    pending: u64,
    /// The last successful flush stamp, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_flush: Option<LastFlush>,
}

/// The kernel `last-flush` stamp shape (best-effort — the kernel writes it on a
/// successful drain). `status` only reads it.
#[derive(Debug, Serialize, Deserialize)]
struct LastFlush {
    timestamp: String,
    #[serde(rename = "last_status", default)]
    status: Option<u16>,
}

fn status(paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    // Resolve the enabled-state precedence. This is the ONLY fallible read here
    // — a malformed config surfaces loudly on the foreground CLI.
    let (enabled, source) = config::resolve_enabled_with_source(paths)?;

    let report = StatusReport {
        enabled,
        source,
        install_uuid: read_install_uuid(paths),
        endpoint: config::resolve_endpoint(paths),
        pending: pending_count(paths),
        last_flush: read_last_flush(paths),
    };

    match mode {
        Mode::Json => write_json(&report),
        Mode::Human => emit_status_human(&report),
    }
}

/// Read the install UUID from the kernel id file WITHOUT minting it. `status` is
/// read-only, so an absent/unreadable id is simply `None`. The id file is one
/// trimmed line; we surface it only when it has the v4 UUID shape (lowercase hex
/// `8-4-4-4-12` with the version/variant nibbles), so a garbage file reports
/// `None` rather than echoing junk.
fn read_install_uuid(paths: &Paths) -> Option<String> {
    let body = util::bounded_read_to_string(&paths.telemetry_id(), util::TOME_CONFIG_MAX).ok()?;
    let first = body.lines().next().unwrap_or("").trim();
    if looks_like_v4_uuid(first) {
        Some(first.to_string())
    } else {
        None
    }
}

/// A lenient v4-UUID shape check (lowercase-hex `8-4-4-4-12`, version nibble `4`,
/// variant nibble in `{8,9,a,b}`). Used only to decide whether to surface the
/// stored id in `status`; the authoritative id is the kernel's.
fn looks_like_v4_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 36 {
        return false;
    }
    for &i in &[8usize, 13, 18, 23] {
        if b[i] != b'-' {
            return false;
        }
    }
    for (i, &c) in b.iter().enumerate() {
        if matches!(i, 8 | 13 | 18 | 23) {
            continue;
        }
        if !(c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
            return false;
        }
    }
    b[14] == b'4' && matches!(b[19], b'8' | b'9' | b'a' | b'b')
}

/// Count pending events = non-blank lines in the kernel queue file. Read-only; a
/// missing queue or any read error ⇒ 0.
fn pending_count(paths: &Paths) -> u64 {
    std::fs::read_to_string(paths.telemetry_queue())
        .map(|s| s.lines().filter(|l| !l.trim().is_empty()).count() as u64)
        .unwrap_or(0)
}

/// Read the kernel `last-flush` stamp, if present. Best-effort and read-only.
fn read_last_flush(paths: &Paths) -> Option<LastFlush> {
    let path = paths.telemetry_last_flush();
    util::refuse_symlinked_component(&path).ok()?;
    let body = util::bounded_read_to_string(&path, util::TOME_CONFIG_MAX).ok()?;
    serde_json::from_str::<LastFlush>(&body).ok()
}

fn emit_status_human(report: &StatusReport) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    let on_off = if report.enabled {
        "enabled"
    } else {
        "disabled"
    };
    writeln!(
        out,
        "telemetry: {} ({})",
        on_off,
        source_label(report.source)
    )?;
    writeln!(
        out,
        "install:   {}",
        report.install_uuid.as_deref().unwrap_or("(none)")
    )?;
    writeln!(out, "endpoint:  {}", report.endpoint)?;
    writeln!(out, "pending:   {}", report.pending)?;
    match &report.last_flush {
        Some(lf) => match lf.status {
            Some(s) => writeln!(out, "last flush: {} (status {})", lf.timestamp, s)?,
            None => writeln!(out, "last flush: {} (no successful delivery)", lf.timestamp)?,
        },
        None => writeln!(out, "last flush: never")?,
    }
    Ok(())
}

fn source_label(source: Source) -> &'static str {
    match source {
        Source::EnvOn => "TOME_TELEMETRY=1",
        Source::EnvOff => "TOME_TELEMETRY=0",
        Source::Ci => "CI auto-off",
        Source::Config => "config file",
        Source::Default => "default",
    }
}

// ---------------------------------------------------------------------------
// inspect — read-only dump of the pending queue (NEVER sends, NEVER repairs)
// ---------------------------------------------------------------------------

/// The `tome telemetry inspect --json` record. `events` preserves queue (FIFO)
/// order and embeds the parsed JSON values verbatim. `corrupt` counts unparsable
/// lines that were skipped — inspect reports them but NEVER repairs the queue.
#[derive(Debug, Serialize)]
struct InspectReport {
    /// Total parsable pending events (the length of `events`).
    pending: u64,
    /// Unparsable lines skipped while reporting. Non-zero ⇒ exit 92.
    corrupt: usize,
    /// The parsed event values, oldest first, embedded as-is.
    events: Vec<serde_json::Value>,
}

/// `tome telemetry inspect` — pretty-print the pending queue without sending it.
///
/// Strictly read-only: reads the queue lines and classifies them, leaving the
/// file byte-identical. The report is emitted FIRST; then, if any line was
/// unparsable, we surface [`TomeError::TelemetryQueueCorrupt`] (exit 92) carrying
/// the SCRUBBED queue path. A clean queue exits 0.
fn inspect_run(paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    let (events, corrupt) = classify_queue_lines(paths);
    let pending = events.len() as u64;

    match mode {
        Mode::Json => {
            let report = InspectReport {
                pending,
                corrupt,
                events,
            };
            write_json(&report)?;
        }
        Mode::Human => emit_inspect_human(pending, corrupt, &events)?,
    }

    if corrupt > 0 {
        return Err(TomeError::TelemetryQueueCorrupt {
            path: scrubbed_queue_path(paths),
            count: corrupt,
        });
    }
    Ok(())
}

/// Read the kernel queue file and split each non-blank line into a parsed JSON
/// value or a corrupt count. Read-only; a missing/unreadable queue is `(empty, 0)`.
fn classify_queue_lines(paths: &Paths) -> (Vec<serde_json::Value>, usize) {
    let body = match std::fs::read_to_string(paths.telemetry_queue()) {
        Ok(b) => b,
        Err(_) => return (Vec::new(), 0),
    };
    let mut events = Vec::new();
    let mut corrupt = 0usize;
    for line in body.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(trimmed) {
            Ok(v) => events.push(v),
            Err(_) => corrupt += 1,
        }
    }
    (events, corrupt)
}

fn emit_inspect_human(
    pending: u64,
    corrupt: usize,
    events: &[serde_json::Value],
) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "pending: {pending}")?;
    for (i, ev) in events.iter().enumerate() {
        // The kernel envelope namespaces the event under an `event.name` field;
        // fall back to "(unknown)" so a value missing it still lists.
        let kind = ev
            .get("event")
            .and_then(|e| e.get("name"))
            .and_then(serde_json::Value::as_str)
            .or_else(|| ev.get("event_type").and_then(serde_json::Value::as_str))
            .unwrap_or("(unknown)");
        let compact = serde_json::to_string(ev).unwrap_or_else(|_| "<unrenderable>".to_string());
        writeln!(out, "  [{i}] {kind}: {compact}")?;
    }
    if corrupt > 0 {
        writeln!(
            out,
            "{corrupt} unparsable line(s) (left in place; the flusher self-heals on drain)"
        )?;
    }
    Ok(())
}

/// Scrub the queue path for inclusion in a [`TomeError::TelemetryQueueCorrupt`]
/// surface. A filesystem path can't carry URL credentials, but routing it through
/// the shared scrubber keeps "every telemetry-facing string is scrubbed" true.
fn scrubbed_queue_path(paths: &Paths) -> std::path::PathBuf {
    let queue = paths.telemetry_queue();
    let bytes = queue.to_string_lossy();
    let scrubbed = crate::catalog::git::scrub_credentials(bytes.as_bytes());
    std::path::PathBuf::from(String::from_utf8_lossy(&scrubbed).into_owned())
}

// ---------------------------------------------------------------------------
// on / off — flip the config switch
// ---------------------------------------------------------------------------

fn on(paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    // The kernel mints the install id lazily on its first emit; we only flip the
    // config switch here (a later command's emit mints, no eager mint needed).
    config::set_enabled(paths, true)?;
    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        writeln!(out, "Telemetry enabled.")?;
    }
    Ok(())
}

fn off(paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    // Off leaves the install UUID intact (a later `on` resumes the same id);
    // only `purge` deletes it.
    config::set_enabled(paths, false)?;
    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        writeln!(out, "Telemetry disabled.")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// reset — sever continuity (new UUID, cleared queue)
// ---------------------------------------------------------------------------

fn reset(paths: &Paths, args: TelemetryResetArgs, mode: Mode) -> Result<(), TomeError> {
    if !args.yes {
        // Confirm-or-refuse. `prompt::confirm` refuses up front on a non-TTY with
        // `NotATerminal` (exit 54), so a scripted reset MUST pass `--yes`.
        let proceed = prompt::confirm(
            "This severs telemetry continuity (new install UUID, queue cleared). Continue?",
            false,
        )?;
        if !proceed {
            if mode == Mode::Human {
                let mut err = std::io::stderr().lock();
                let _ = writeln!(err, "Aborted: reset declined.");
            }
            return Ok(());
        }
    }

    // The kernel `reset` mints a fresh install id and clears the queue.
    if let Some(h) = crate::telemetry::handle() {
        h.reset().map_err(TomeError::Io)?;
    }

    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        let fresh = read_install_uuid(paths).unwrap_or_else(|| "(unavailable)".to_string());
        writeln!(out, "Telemetry identity reset. New install UUID: {fresh}")?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// purge — delete all telemetry state and disable
// ---------------------------------------------------------------------------

fn purge(paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    // Disable in config first, then remove all telemetry state files (id + queue
    // + the Tome-owned stamps). A missing file is fine.
    config::set_enabled(paths, false)?;
    for p in [
        paths.telemetry_id(),
        paths.telemetry_queue(),
        paths.telemetry_last_version(),
        paths.telemetry_last_heartbeat(),
        paths.telemetry_last_flush(),
        paths.telemetry_last_flush_attempt(),
    ] {
        let _ = std::fs::remove_file(p);
    }
    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "Telemetry purged: identity deleted, queue cleared, telemetry disabled."
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// flush — FOREGROUND drain (and the detached child's `--quiet` entry point)
// ---------------------------------------------------------------------------

/// `tome telemetry flush [--quiet]` — drain the pending queue to the collector in
/// the FOREGROUND via the kernel handle's `run_flush` (this process IS the
/// detached child under `--quiet`). Best-effort: the kernel drain never fails the
/// caller, so this always exits 0 (the former exit-90 path is vestigial).
///
/// The `--quiet` child neither emits nor spawns: it is a `Telemetry` command, so
/// `main.rs` skips `cli_startup` (no mint/notice) AND skips `teardown_at_exit`
/// (no recursive flusher fork) — that gating is the fork-bomb guard.
fn flush_run(paths: &Paths, args: TelemetryFlushArgs, mode: Mode) -> Result<(), TomeError> {
    if let Some(h) = crate::telemetry::handle() {
        h.run_flush();
    }

    if args.quiet {
        // The detached child must be silent and always exit 0.
        return Ok(());
    }

    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        let pending = pending_count(paths);
        if pending == 0 {
            writeln!(out, "Telemetry flushed.")?;
        } else {
            writeln!(out, "Telemetry flush: {pending} event(s) still pending.")?;
        }
    }
    Ok(())
}

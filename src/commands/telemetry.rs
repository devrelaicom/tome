//! `tome telemetry {status,on,off,reset,purge}` — the user-facing controls for
//! the local-first telemetry subsystem (Phase 10, US1).
//!
//! Every subcommand here is a foreground, user-invoked command, so — unlike the
//! silent enqueue/flush path — it surfaces errors loudly (config parse → exit
//! 91, non-TTY reset → exit 54). Reports land on **stdout**; the global `--json`
//! flag (carried in `mode`) shapes them. `status` is strictly read-only: it must
//! never mint the install id or write any state.
//!
//! `inspect` (US2) is a read-only reporter: it pretty-prints the pending queue
//! WITHOUT sending it, leaves the queue file byte-identical, and exits 92
//! ([`TomeError::TelemetryQueueCorrupt`]) when unparsable lines exist (after the
//! report). `flush` (US3) drains the queue in the FOREGROUND: it reports the
//! outcome and exits 90 ([`TomeError::TelemetryEndpointUnreachable`]) on a
//! transport error, EXCEPT under `--quiet` (the detached child) which is silent
//! and always exits 0. The enum is the CLI's [`crate::cli::TelemetryCommand`].

use std::io::Write;

use serde::{Deserialize, Serialize};

use crate::cli::{TelemetryCommand, TelemetryFlushArgs, TelemetryResetArgs};
use crate::error::TomeError;
use crate::output::{Mode, write_json};
use crate::paths::Paths;
use crate::presentation::prompt;
use crate::telemetry::config::{self, Source};
use crate::telemetry::event::Uuid;
use crate::telemetry::{identity, transport};
use crate::util;
use crate::workspace::ResolvedScope;

/// Subcommand dispatcher invoked by `main.rs`.
///
/// `paths` is resolved here from the default `$HOME` layout (via
/// [`Paths::resolve`]) rather than read off `scope` — `ResolvedScope` carries
/// workspace identity, not a `Paths`, and telemetry state is per-install (not
/// per-workspace). This mirrors how `status`/`harness` obtain `Paths`.
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

/// The byte-stable `tome telemetry status` record (pin-tested). Field order and
/// the `skip_serializing_if` gates are load-bearing: a JSON consumer parses this
/// exact shape.
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

/// The `telemetry/last-flush` stamp shape — written by the flusher (US3,
/// `flush::stamp_last_flush`) as `{"timestamp":"<rfc3339>","last_status":<u16|null>}`.
///
/// `status` deserializes from the stamp's `last_status` key and is an `Option`
/// so a failed-drain `null` status (no batch acknowledged a 2xx) is
/// distinguished from a successful 2xx. `#[serde(default)]` lets a stamp that
/// omits the field (shouldn't happen — the writer always emits it) still parse.
#[derive(Debug, Serialize, Deserialize)]
struct LastFlush {
    timestamp: String,
    #[serde(rename = "last_status", default)]
    status: Option<u16>,
}

fn status(paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    // Resolve the enabled-state precedence. This is the ONLY fallible read here
    // — a malformed config surfaces as exit 91 (loud, on the foreground CLI).
    let (enabled, source) = config::resolve_enabled_with_source(paths)?;

    let report = StatusReport {
        enabled,
        source,
        install_uuid: read_install_uuid(paths),
        endpoint: transport::resolve_endpoint(),
        pending: pending_count(paths),
        last_flush: read_last_flush(paths),
    };

    match mode {
        Mode::Json => write_json(&report),
        Mode::Human => emit_status_human(&report),
    }
}

/// Read the install UUID from `telemetry/id` WITHOUT minting it. `status` is
/// read-only (FR), so an absent or corrupt id is simply `None` — we never call
/// `ensure_install_id` (which would mint). The id file is one trimmed line.
fn read_install_uuid(paths: &Paths) -> Option<String> {
    let body = util::bounded_read_to_string(&paths.telemetry_id(), util::TOME_CONFIG_MAX).ok()?;
    let first = body.lines().next().unwrap_or("").trim();
    Uuid::parse(first).map(|u| u.as_str().to_string())
}

/// Count queued events = non-blank lines in `telemetry/queue.jsonl`. Routes
/// through the queue module's SSOT [`queue::count_pending`] (read-only, missing
/// queue ⇒ 0, any error ⇒ 0) rather than re-counting lines inline, so the
/// "what counts as a pending line" rule lives in exactly one place. The `as u64`
/// keeps the byte-stable `pending` JSON field type unchanged.
fn pending_count(paths: &Paths) -> u64 {
    crate::telemetry::queue::count_pending(paths) as u64
}

/// Read the `telemetry/last-flush` stamp, if present.
///
/// Best-effort and read-only (like every `status` read): an absent file, an
/// unreadable/over-cap file, or an unparsable body all degrade to `None` — a
/// `status` report never fails on the stamp. The flusher (US3) writes the stamp
/// as `{"timestamp":...,"last_status":<u16|null>}`; we parse exactly that shape.
fn read_last_flush(paths: &Paths) -> Option<LastFlush> {
    let path = paths.telemetry_last_flush();
    // Sec-L1: read/write containment parity — the flusher writes the stamp via the
    // shared atomic (symlink-refusing) writer; refuse a symlinked component on the
    // read too. A hostile stamp degrades to `None` (absent), never propagated.
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
            // A `null` status means the drain ran but no batch was acknowledged
            // (empty queue, or a transport error before any 2xx).
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

/// The byte-stable `tome telemetry inspect --json` record (pin-tested). Field
/// order is load-bearing. `events` preserves queue (FIFO) order and embeds the
/// parsed JSON values verbatim. `corrupt` is the count of unparsable lines that
/// were skipped — inspect reports them but NEVER repairs the queue (the flusher
/// self-heals on drain).
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
/// Strictly read-only: routes through [`queue::classify_lines`] /
/// [`queue::read_lines`], which never mutate the file, so the queue is
/// byte-identical afterwards. The report (human or JSON) is emitted FIRST; then,
/// if any line was unparsable, we surface [`TomeError::TelemetryQueueCorrupt`]
/// (exit 92) carrying the SCRUBBED queue path. A clean queue exits 0.
fn inspect_run(paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    // Read-only classification: parsed values + a count of unparsable lines.
    // `classify_lines` (and the `read_lines` it calls) only read the file.
    let (events, corrupt) = crate::telemetry::queue::classify_lines(paths)?;
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

    // The report has already been printed. If the queue held unparsable lines,
    // surface exit 92 — but NEVER mutate the queue (no repair); the flusher
    // self-heals on its next drain. The path is scrubbed like every other
    // telemetry-facing string.
    if corrupt > 0 {
        return Err(TomeError::TelemetryQueueCorrupt {
            path: scrubbed_queue_path(paths),
            count: corrupt,
        });
    }
    Ok(())
}

fn emit_inspect_human(
    pending: u64,
    corrupt: usize,
    events: &[serde_json::Value],
) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    writeln!(out, "pending: {pending}")?;
    for (i, ev) in events.iter().enumerate() {
        // Surface the event_type when present (the wire field is `event_type`);
        // fall back to "(unknown)" so a value missing it still lists.
        let kind = ev
            .get("event_type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("(unknown)");
        // A compact, one-line rendering of the value (no pretty-print: keep it
        // to a single line per event).
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
/// surface. A filesystem path can't carry URL credentials, but routing it
/// through the shared scrubber keeps "every telemetry-facing string is scrubbed"
/// true by construction — every path string routes through
/// [`crate::catalog::git::scrub_credentials`].
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
    config::set_enabled(paths, true)?;
    // Mint the install id if absent so an identity exists for the funnel join
    // key. `ensure_install_id` is idempotent (no-op when present).
    identity::ensure_install_id(paths)?;
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
        // Confirm-or-refuse. `prompt::confirm` refuses up front on a non-TTY
        // with `NotATerminal` (exit 54) — the same pattern as `models remove` —
        // so a scripted reset MUST pass `--yes`. A Ctrl-C maps to `Interrupted`.
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

    // `identity::reset` acquires the flush lock first, then mints a fresh id and
    // clears the queue. Returns the new UUID.
    let fresh = identity::reset(paths)?;
    if mode == Mode::Human {
        let mut out = std::io::stdout().lock();
        writeln!(
            out,
            "Telemetry identity reset. New install UUID: {}",
            fresh.as_str()
        )?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// purge — delete all telemetry state and disable
// ---------------------------------------------------------------------------

fn purge(paths: &Paths, mode: Mode) -> Result<(), TomeError> {
    // Unconditional (no confirm): purge deletes the id, clears the queue, and
    // sets enabled=false. It acquires the flush lock first.
    identity::purge(paths)?;
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

/// `tome telemetry flush [--quiet]` — drain the pending queue to the collector
/// in the FOREGROUND (this is NOT the spawn site; it calls
/// [`crate::telemetry::flush::run`] directly).
///
/// Two modes, both routing through the ONE shared drain:
/// - **default (loud)**: report the outcome on stdout and exit 0 on a clean
///   drain; surface [`TelemetryEndpointUnreachable`](TomeError::TelemetryEndpointUnreachable)
///   (exit 90, scrubbed endpoint) on a transport error, and propagate any other
///   error.
/// - **`--quiet`** (the spawned detached child, FR-020): discard the `Result`
///   entirely — NO stdout/stderr, ALWAYS exit 0. A transport failure is invisible
///   to the background child (it leaves the queue intact to retry next drain).
///
/// The `--quiet` child neither enqueues nor spawns: it is a `Telemetry` command,
/// so `main.rs` skips `cli_startup` (no mint/notice) AND skips `teardown_at_exit`
/// (no recursive flusher fork) — that gating is the fork-bomb guard.
fn flush_run(paths: &Paths, args: TelemetryFlushArgs, mode: Mode) -> Result<(), TomeError> {
    let result = crate::telemetry::flush::run(paths);

    if args.quiet {
        // FR-020: the detached child must be silent and always exit 0. Swallow
        // the outcome wholesale — a transport failure stays in the queue for the
        // next drain and is never surfaced.
        return Ok(());
    }

    match result {
        Ok(()) => {
            if mode == Mode::Human {
                let mut out = std::io::stdout().lock();
                // A best-effort, concise outcome: the pending count after the
                // drain distinguishes "sent everything" from "nothing queued".
                let pending = crate::telemetry::queue::count_pending(paths);
                if pending == 0 {
                    writeln!(out, "Telemetry flushed.")?;
                } else {
                    writeln!(out, "Telemetry flush: {pending} event(s) still pending.")?;
                }
            }
            Ok(())
        }
        // A transport/non-https/unreachable error surfaces as exit 90 with the
        // SCRUBBED endpoint (the error variant already carries the scrubbed form).
        // Any other error propagates unchanged.
        Err(e) => Err(e),
    }
}

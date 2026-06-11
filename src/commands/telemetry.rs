//! `tome telemetry {status,on,off,reset,purge}` — the user-facing controls for
//! the local-first telemetry subsystem (Phase 10, US1).
//!
//! Every subcommand here is a foreground, user-invoked command, so — unlike the
//! silent enqueue/flush path — it surfaces errors loudly (config parse → exit
//! 91, non-TTY reset → exit 54). Reports land on **stdout**; the global `--json`
//! flag (carried in `mode`) shapes them. `status` is strictly read-only: it must
//! never mint the install id or write any state.
//!
//! `inspect` and `flush` are deliberately absent — they land in later slices
//! (US2 / US3). The enum is the CLI's [`crate::cli::TelemetryCommand`]; new
//! variants are added there when those slices land.

use std::io::Write;

use serde::Serialize;

use crate::cli::{TelemetryCommand, TelemetryResetArgs};
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
        TelemetryCommand::On => on(&paths, mode),
        TelemetryCommand::Off => off(&paths, mode),
        TelemetryCommand::Reset(args) => reset(&paths, args, mode),
        TelemetryCommand::Purge => purge(&paths, mode),
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

/// The `telemetry/last-flush` stamp shape. US3 defines + writes it; until then a
/// status read always sees it absent and reports `None`.
#[derive(Debug, Serialize)]
struct LastFlush {
    timestamp: String,
    status: u16,
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

/// Count queued events = lines in `telemetry/queue.jsonl`. A missing queue is 0
/// (nothing pending). The read is bounded; an unreadable/over-cap queue degrades
/// to 0 rather than erroring a read-only report.
fn pending_count(paths: &Paths) -> u64 {
    match util::bounded_read_to_string(&paths.telemetry_queue(), util::HARNESS_RULES_MAX) {
        Ok(body) => body.lines().filter(|l| !l.trim().is_empty()).count() as u64,
        Err(_) => 0,
    }
}

/// Read the `telemetry/last-flush` stamp, if present.
///
// US3 fills last-flush: the stamp format is not yet defined/written, so there is
// no on-disk producer. Until then an absent file is the only case → `None`.
fn read_last_flush(_paths: &Paths) -> Option<LastFlush> {
    None
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
        Some(lf) => writeln!(out, "last flush: {} (status {})", lf.timestamp, lf.status)?,
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

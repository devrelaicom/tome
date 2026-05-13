//! Output mode + a thin formatter abstraction that keeps individual commands
//! tidy. `Mode::Json` shapes stdout records; `Mode::Human` writes a friendly
//! line. Colour is auto-disabled by `anstream` when stdout is not a TTY or
//! when `NO_COLOR`/`CLICOLOR=0` is set (FR-020).

use std::io::{IsTerminal, Write};

use serde::Serialize;

use crate::error::TomeError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    Human,
    Json,
}

impl Mode {
    pub fn from_flag(json: bool) -> Self {
        if json { Self::Json } else { Self::Human }
    }
}

/// Whether stdout is connected to a real terminal. Used by interactive
/// commands (e.g. `tome catalog remove`) — non-TTY without `--force` is a
/// usage error per FR-021.
pub fn stdout_is_tty() -> bool {
    std::io::stdout().is_terminal()
}

pub fn stdin_is_tty() -> bool {
    std::io::stdin().is_terminal()
}

/// Serialise `value` as one JSON record per line on stdout (NDJSON). Caller
/// chooses whether to invoke this in a loop (`list`) or once (`show`).
pub fn write_json<T: Serialize + ?Sized>(value: &T) -> Result<(), TomeError> {
    let mut out = std::io::stdout().lock();
    serde_json::to_writer(&mut out, value)
        .map_err(|e| TomeError::Internal(anyhow::Error::new(e)))?;
    writeln!(out)?;
    Ok(())
}

/// `--json` error record schema (matches data-model.md §6). Always writes to
/// stderr — errors never appear on stdout.
#[derive(Debug, Serialize)]
pub struct ErrorRecord<'a> {
    pub category: &'a str,
    pub exit_code: i32,
    pub message: String,
}

impl<'a> ErrorRecord<'a> {
    pub fn from_error(err: &'a TomeError) -> Self {
        Self {
            category: err.category(),
            exit_code: err.exit_code(),
            message: format!("{}", err),
        }
    }
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope<'a> {
    error: ErrorRecord<'a>,
}

pub fn write_error(mode: Mode, err: &TomeError) {
    let mut err_out = std::io::stderr().lock();
    match mode {
        Mode::Human => {
            // Plain text; `anstream` would add colour here if a stylist were
            // attached. For now keep it dependency-light — the `--help` polish
            // pass in Phase 6 will revisit styling.
            let _ = writeln!(err_out, "error: {}", err);
        }
        Mode::Json => {
            let env = ErrorEnvelope {
                error: ErrorRecord::from_error(err),
            };
            let _ = serde_json::to_writer(&mut err_out, &env);
            let _ = writeln!(err_out);
        }
    }
}

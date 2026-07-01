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
///
/// #296: `retryable` + `remediation` are appended (never reorder the existing
/// fields — the envelope is byte-pinned). Both derive from the closed
/// [`ErrorCategory`](crate::error::ErrorCategory) SSOT — the SAME accessors the
/// MCP tool `data` payload uses — so an agent branches on structured data rather
/// than regexing the English `message`. `retryable` is always present;
/// `remediation` is omitted entirely when the category has no single fix
/// command (`skip_serializing_if`), so error records that never had a fix keep
/// their exact prior shape apart from the always-present `retryable` bool.
#[derive(Debug, Serialize)]
pub struct ErrorRecord<'a> {
    pub category: &'a str,
    pub exit_code: i32,
    pub message: String,
    /// Whether retrying the same operation unchanged could plausibly succeed
    /// (transient/contended failures — see [`ErrorCategory::retryable`]).
    pub retryable: bool,
    /// The coarse `tome` command that fixes this failure class, if one exists
    /// (see [`ErrorCategory::remediation`]). Omitted when `None`. Never carries
    /// a credential or instance-specific secret (it is a `&'static str`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

impl<'a> ErrorRecord<'a> {
    pub fn from_error(err: &'a TomeError) -> Self {
        let category = err.category();
        Self {
            category: category.as_str(),
            exit_code: err.exit_code(),
            message: format!("{}", err),
            retryable: category.retryable(),
            remediation: category.remediation().map(str::to_owned),
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

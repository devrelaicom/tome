//! Output mode + a thin formatter abstraction that keeps individual commands
//! tidy. `Mode::Json` shapes stdout records; `Mode::Human` writes a friendly
//! line. Colour is auto-disabled by `anstream` when stdout is not a TTY or
//! when `NO_COLOR`/`CLICOLOR=0` is set (FR-020).

use std::io::{IsTerminal, Write};

use serde::Serialize;

use crate::error::TomeError;
use crate::presentation::colour;

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
            let _ = writeln!(err_out, "{}", render_human_error(err));
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

/// Render the human-mode error body (no trailing newline).
///
/// `TomeError` `Display` values embed continuation lines (`hint:`, `Bound:`,
/// `Enabled:`, `available:`) after a `\n`, so the message that reaches here is
/// often multi-line. We style the leading `error:` token red and dim each
/// continuation line so the failure reads as the failure and the guidance reads
/// as secondary.
///
/// Every style is applied through [`colour`], which is a no-op when colour is
/// disabled (`NO_COLOR` / piped / non-TTY / `--no-color`). The indent is *also*
/// gated on [`colour::is_enabled`], so with colour off the output is byte-for-
/// byte the historical `error: {err}` (plus any embedded continuation lines) —
/// no escape codes and no extra whitespace leak into piped stderr.
fn render_human_error(err: &TomeError) -> String {
    assemble_human_error(
        &err.to_string(),
        colour::is_enabled(),
        colour::error,
        colour::dim,
    )
}

/// Pure assembly of the human error body, argument-driven so both the styled
/// and plain branches are unit-testable without touching the process-global
/// colour `OnceLock`. `style_prefix` styles the leading `error:` token;
/// `style_hint` styles each continuation line. When `styled` is false the
/// callers pass identity-style closures (colour helpers are no-ops when colour
/// is disabled) and no indent is added, so the result is byte-identical to the
/// historical `error: {display}` shape.
fn assemble_human_error(
    display: &str,
    styled: bool,
    style_prefix: impl Fn(&str) -> String,
    style_hint: impl Fn(&str) -> String,
) -> String {
    let mut lines = display.split('\n');
    let first = lines.next().unwrap_or("");
    let mut out = format!("{} {first}", style_prefix("error:"));

    for line in lines {
        out.push('\n');
        if styled {
            // Two-space indent so continuation guidance (hint:/Bound:/...) is
            // visually offset from the `error:` prefix; only emitted when colour
            // is on so the plain (piped) output stays byte-identical to the
            // historical shape.
            out.push_str("  ");
        }
        out.push_str(&style_hint(line));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Safety property: with colour disabled the human error body is exactly
    /// `error: {display}` — no escape codes, no indentation — for both single-
    /// line and hint-bearing multi-line `Display` values. `render_human_error`
    /// reads the process-global colour gate, which settles on `false` in the
    /// Cargo test harness (stdout is not a TTY), so this exercises the real
    /// production path.
    #[test]
    fn human_error_disabled_is_byte_identical_to_plain_prefix() {
        // Single-line variant: no continuation lines.
        let usage = TomeError::Usage("bad flag".to_owned());
        assert_eq!(render_human_error(&usage), "error: invalid usage: bad flag");

        // Multi-line variant: an embedded `hint:` continuation line.
        let not_found = TomeError::CatalogNotFound("foo".to_owned());
        let display = not_found.to_string();
        assert_eq!(render_human_error(&not_found), format!("error: {display}"));

        // Belt-and-braces: the rendered form carries no ANSI escape byte and
        // introduces no leading indentation on the continuation line when the
        // colour gate is off.
        let rendered = render_human_error(&not_found);
        assert!(
            !rendered.contains('\u{1b}'),
            "no escape codes must leak when colour is disabled: {rendered:?}"
        );
        for line in rendered.split('\n').skip(1) {
            assert!(
                !line.starts_with(' '),
                "continuation lines must not be indented when colour is disabled: {line:?}"
            );
        }
    }

    /// Pure-assembly, plain branch: identity styling + `styled = false` must
    /// reproduce the historical shape exactly (documents the disabled contract
    /// independently of the global gate).
    #[test]
    fn assemble_plain_reproduces_historical_shape() {
        let id = |s: &str| s.to_owned();

        assert_eq!(
            assemble_human_error("bad flag", false, id, id),
            "error: bad flag"
        );
        assert_eq!(
            assemble_human_error("boom\nhint: try harder", false, id, id),
            "error: boom\nhint: try harder"
        );
        // Multiple continuation lines are each preserved verbatim.
        assert_eq!(
            assemble_human_error("boom\nhint: a\nBound: b", false, id, id),
            "error: boom\nhint: a\nBound: b"
        );
    }

    /// Pure-assembly, styled branch: the `error:` prefix carries the prefix
    /// style, and each continuation line is indented two spaces and carries the
    /// hint style. Uses sentinel-wrapping style closures so the assertion is
    /// deterministic without depending on the terminal or `owo-colors`.
    #[test]
    fn assemble_styled_styles_prefix_and_indents_hints() {
        let prefix = |s: &str| format!("<red>{s}</red>");
        let hint = |s: &str| format!("<dim>{s}</dim>");

        // Single line: prefix styled, no continuation.
        assert_eq!(
            assemble_human_error("boom", true, prefix, hint),
            "<red>error:</red> boom"
        );

        // Multi-line: prefix styled; hint line indented + hint-styled.
        assert_eq!(
            assemble_human_error("boom\nhint: try harder", true, prefix, hint),
            "<red>error:</red> boom\n  <dim>hint: try harder</dim>"
        );

        // Every continuation line (hint:/Bound:/...) is indented + styled.
        assert_eq!(
            assemble_human_error("boom\nhint: a\nBound: b", true, prefix, hint),
            "<red>error:</red> boom\n  <dim>hint: a</dim>\n  <dim>Bound: b</dim>"
        );
    }
}

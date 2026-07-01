//! Interactive prompt wrappers. Each function refuses up front when a
//! terminal isn't available on **both** stdin and stdout, returning
//! [`TomeError::NotATerminal`] (exit 54) per FR-051. This matters more than
//! just stdin-is-tty because `inquire` writes the prompt and reads its echo
//! on stdout; piping stdout (e.g. `tome plugin | tee out.log`) would
//! otherwise produce mangled prompts and dangerously sticky answers.
//!
//! For non-interactive callers, every prompt has a non-interactive
//! equivalent exposed elsewhere (FR-052) — `--force`, an explicit selector
//! flag, etc. This module's `Err(NotATerminal)` is the signpost that points
//! callers there.
//!
//! The global `--non-interactive` flag (and the `TOME_NONINTERACTIVE` env var)
//! auto-confirm every prompt-bearing command. Rather than sprinkle the check
//! through each command, the decision lands in one place: [`non_interactive`].
//! Confirmation-gated commands read it alongside their per-command skip flag
//! (`--force` / `--yes`), so any of the three independently suppresses the
//! prompt. See the module `set_non_interactive` / `non_interactive` pair,
//! which mirrors `presentation::colour::{set_disabled, is_enabled}`.

use std::sync::OnceLock;

use inquire::{Confirm, MultiSelect, Select};

use crate::error::TomeError;
use crate::output;

/// Set by the CLI when `--non-interactive` is passed. Forwarded from `main.rs`
/// before dispatch, mirroring `colour::set_disabled`. `None` until set.
static NON_INTERACTIVE: OnceLock<bool> = OnceLock::new();

/// Forward the global `--non-interactive` flag from the CLI parser. Idempotent:
/// only the first call wins, so a later dispatch can't flip the decision.
pub fn set_non_interactive(enabled: bool) {
    let _ = NON_INTERACTIVE.set(enabled);
}

/// Whether the caller wants every prompt auto-confirmed. True when the global
/// `--non-interactive` flag was passed OR the `TOME_NONINTERACTIVE` env var is
/// truthy (set, non-empty, and not one of `0`/`false`/`no`/`off`,
/// case-insensitive — the shared [`crate::util::env_truthy`] convention, the
/// same one `telemetry::config`'s CI detection uses). The env var is read live
/// so a caller that only sets the environment (never the flag) is honoured; a
/// persistently-exported `TOME_NONINTERACTIVE=1` therefore also auto-confirms
/// the otherwise-interactive `tome plugin` TUI prompts — intended per the
/// "env auto-confirms every prompt" semantics.
///
/// Confirmation-gated commands combine this with their per-command skip flag:
/// `if !args.force && !prompt::non_interactive() { … prompt … }`. Any of the
/// three suppresses the prompt.
pub fn non_interactive() -> bool {
    if *NON_INTERACTIVE.get().unwrap_or(&false) {
        return true;
    }
    crate::util::env_truthy("TOME_NONINTERACTIVE")
}

/// Hard-require both ends of the user interaction to be a terminal. Used at
/// the entry of every prompt function below and at the entry of the
/// `tome plugin` interactive flow (FR-051).
pub fn require_terminal() -> Result<(), TomeError> {
    if output::stdin_is_tty() && output::stdout_is_tty() {
        Ok(())
    } else {
        Err(TomeError::NotATerminal)
    }
}

/// Pick exactly one item from `options`. Returns `Err(NotATerminal)` if the
/// process is not attached to a terminal.
pub fn select<T: std::fmt::Display>(message: &str, options: Vec<T>) -> Result<T, TomeError> {
    require_terminal()?;
    Select::new(message, options)
        .prompt()
        .map_err(prompt_error_to_tome)
}

/// Pick any number of items from `options`.
pub fn multiselect<T: std::fmt::Display>(
    message: &str,
    options: Vec<T>,
) -> Result<Vec<T>, TomeError> {
    require_terminal()?;
    MultiSelect::new(message, options)
        .prompt()
        .map_err(prompt_error_to_tome)
}

/// Ask a yes/no question with `default` as the pre-selected answer.
pub fn confirm(message: &str, default: bool) -> Result<bool, TomeError> {
    require_terminal()?;
    Confirm::new(message)
        .with_default(default)
        .prompt()
        .map_err(prompt_error_to_tome)
}

fn prompt_error_to_tome(err: inquire::InquireError) -> TomeError {
    use inquire::InquireError::*;
    match err {
        // Ctrl-C or Ctrl-D: surface as Interrupted so the standard signal
        // path applies (exit 8). This matches the constitution's "scriptable
        // by default" expectation that interactive cancellation is a clean
        // exit rather than an internal error.
        OperationCanceled | OperationInterrupted => TomeError::Interrupted,
        NotTTY => TomeError::NotATerminal,
        IO(e) => TomeError::Io(e),
        other => TomeError::Internal(anyhow::anyhow!("prompt failed: {other:?}")),
    }
}

/// Suspend any active progress bar / spinner over `f`, restoring it after.
/// `inquire` repaints stdout/stderr and races with `indicatif`'s ticker; the
/// `indicatif` recipe is to wrap an interactive read in `pb.suspend(|| …)`.
pub fn suspend_progress<R>(pb: &indicatif::ProgressBar, f: impl FnOnce() -> R) -> R {
    pb.suspend(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_terminal_errors_under_cargo_test() {
        // The Cargo test harness redirects stdio; stdin and stdout are not
        // terminals. require_terminal() must therefore return NotATerminal.
        let r = require_terminal();
        assert!(
            matches!(r, Err(TomeError::NotATerminal)),
            "require_terminal() under cargo test: {r:?}",
        );
    }

    #[test]
    fn select_short_circuits_in_non_tty_context() {
        let r = select("pick one", vec!["a", "b"]);
        assert!(matches!(r, Err(TomeError::NotATerminal)));
    }

    #[test]
    fn multiselect_short_circuits_in_non_tty_context() {
        let r = multiselect("pick many", vec!["a", "b"]);
        assert!(matches!(r, Err(TomeError::NotATerminal)));
    }

    #[test]
    fn confirm_short_circuits_in_non_tty_context() {
        let r = confirm("are you sure?", false);
        assert!(matches!(r, Err(TomeError::NotATerminal)));
    }

    // The env-var half of `non_interactive()` is the shared
    // `crate::util::env_truthy`; its truthy/falsey token parse is covered
    // lock-free (no process-env mutation) in `crate::util::env`'s own tests.
    // `NON_INTERACTIVE` is a process-global OnceLock (like `colour::ENABLED`),
    // so the flag half + the composed CLI behaviour are covered by the
    // binary-driven integration tests (`catalog_remove` / telemetry `identity`).
}

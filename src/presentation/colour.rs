//! Colour gating for human output.
//!
//! Colour is enabled when **all three** of the following are true:
//! 1. stdout is connected to a terminal,
//! 2. the `NO_COLOR` environment variable is not set (per <https://no-color.org>), and
//! 3. the caller has not passed `--no-color` (forwarded via [`set_disabled`]).
//!
//! The decision is computed once at startup via [`init`] and read by any code
//! that wants to colour a fragment, so the result is consistent across the
//! whole command's output.
//!
//! `owo-colors` is the underlying crate; this module never imports it
//! transitively. Code that wants `.green()` / `.red()` etc. should use the
//! [`ColouredExt`] helpers below, which return a styled `String` or a plain
//! `String` depending on the gate.

use std::sync::OnceLock;

use owo_colors::OwoColorize;

use crate::output;

/// Configured at startup. `None` until [`init`] is called.
static ENABLED: OnceLock<bool> = OnceLock::new();
/// Set by the CLI when `--no-color` is passed. Overrides every other signal.
static FORCE_DISABLED: OnceLock<bool> = OnceLock::new();

/// Forward the `--no-color` flag from the CLI parser. Idempotent. Must be
/// called before [`init`] for the flag to take effect.
pub fn set_disabled(disabled: bool) {
    let _ = FORCE_DISABLED.set(disabled);
}

/// Compute and cache the colour-enabled decision. Idempotent — subsequent
/// calls return the cached value.
pub fn init() -> bool {
    *ENABLED.get_or_init(|| {
        if *FORCE_DISABLED.get().unwrap_or(&false) {
            return false;
        }
        if std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty()) {
            return false;
        }
        output::stdout_is_tty()
    })
}

/// Whether colour is enabled. Cheap to call after [`init`].
pub fn is_enabled() -> bool {
    ENABLED.get().copied().unwrap_or(false)
}

/// Render `text` in the project's "success" colour iff colour is enabled.
pub fn success(text: &str) -> String {
    if is_enabled() {
        text.green().to_string()
    } else {
        text.to_string()
    }
}

/// Render `text` in the project's "warning" colour iff colour is enabled.
pub fn warning(text: &str) -> String {
    if is_enabled() {
        text.yellow().to_string()
    } else {
        text.to_string()
    }
}

/// Render `text` in the project's "error" colour iff colour is enabled.
pub fn error(text: &str) -> String {
    if is_enabled() {
        text.red().to_string()
    } else {
        text.to_string()
    }
}

/// Render `text` in the project's "hint" colour iff colour is enabled.
pub fn hint(text: &str) -> String {
    if is_enabled() {
        text.cyan().to_string()
    } else {
        text.to_string()
    }
}

/// Render `text` in bold iff colour is enabled.
pub fn bold(text: &str) -> String {
    if is_enabled() {
        text.bold().to_string()
    } else {
        text.to_string()
    }
}

/// Render `text` dimmed iff colour is enabled. Used for paths and metadata.
pub fn dim(text: &str) -> String {
    if is_enabled() {
        text.dimmed().to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ENABLED is a global OnceLock, so we cannot reliably re-initialise it
    // mid-process. These tests therefore exercise only the public predicate
    // forms; the actual gating logic is covered by visual inspection plus
    // the integration tests that pipe `tome` to a file (FR-046).

    #[test]
    fn colour_helpers_return_plain_string_in_non_tty_context() {
        // In a Cargo test harness stdout is not a TTY, so init() should
        // settle on `false`.
        init();
        assert!(!is_enabled());

        // Every helper must return its input verbatim when colour is off,
        // not an ANSI-wrapped variant.
        let s = "hello";
        assert_eq!(success(s), s);
        assert_eq!(warning(s), s);
        assert_eq!(error(s), s);
        assert_eq!(hint(s), s);
        assert_eq!(bold(s), s);
        assert_eq!(dim(s), s);
    }

    // NOTE: a unit test that exercises the NO_COLOR env-var branch of init()
    // is intentionally omitted here. Env mutation inside the lib test binary
    // interacted poorly with the pre-push hook environment (see retro/P2.md).
    // That branch is covered by integration tests in later phases that
    // invoke `tome` with NO_COLOR set in the test process's env, asserting
    // that `--json` output is ANSI-free and human output is colour-free.
}

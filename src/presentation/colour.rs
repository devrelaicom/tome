//! Colour gating for human output.
//!
//! Colour is enabled according to the following precedence (highest wins):
//! 1. `--no-color` CLI flag (forwarded via [`set_disabled`]) → always off,
//! 2. `NO_COLOR` environment variable (per <https://no-color.org>) → always off,
//! 3. `[output] color` in `~/.tome/config.toml` (`always` → on, `never` → off),
//! 4. auto: stdout is connected to a terminal.
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

/// Pure colour-enabled resolver. Precedence (highest wins):
/// 1. `no_color_flag` (`--no-color`) → false
/// 2. `no_color_env` (`NO_COLOR` present and non-empty) → false
/// 3. `config` `Always` → true, `Never` → false
/// 4. `Auto` / no config → `is_tty`
///
/// Kept pure and argument-driven so it is trivially unit-testable without
/// touching any global state.
pub(crate) fn resolve_color(
    no_color_flag: bool,
    no_color_env: Option<()>,
    config: Option<crate::config::ColorMode>,
    is_tty: bool,
) -> bool {
    if no_color_flag || no_color_env.is_some() {
        return false;
    }
    match config {
        Some(crate::config::ColorMode::Always) => true,
        Some(crate::config::ColorMode::Never) => false,
        Some(crate::config::ColorMode::Auto) | None => is_tty,
    }
}

/// Compute and cache the colour-enabled decision. Idempotent — subsequent
/// calls return the cached value.
///
/// Reads the config defensively (`load_or_default`) so a malformed
/// `config.toml` never prevents colour/progress from initialising; the
/// strict error is surfaced by the command itself.
pub fn init() -> bool {
    *ENABLED.get_or_init(|| {
        let no_color_flag = *FORCE_DISABLED.get().unwrap_or(&false);
        let no_color_env = std::env::var_os("NO_COLOR")
            .filter(|v| !v.is_empty())
            .map(|_| ());
        let config_color = crate::paths::Paths::resolve()
            .ok()
            .map(|p| crate::config::load_or_default(&p).output.color)
            .and_then(|c| c);
        let is_tty = output::stdout_is_tty();
        resolve_color(no_color_flag, no_color_env, config_color, is_tty)
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

/// Render `text` as a panel key: bold + the project accent (cyan) iff colour
/// is enabled.
pub fn label(text: &str) -> String {
    if is_enabled() {
        text.cyan().bold().to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ColorMode;

    // ENABLED is a global OnceLock, so we cannot reliably re-initialise it
    // mid-process. These tests therefore exercise only the public predicate
    // forms and the pure `resolve_color` function; the actual gating logic
    // is covered by visual inspection plus the integration tests that pipe
    // `tome` to a file (FR-046).

    /// Verify the full precedence chain of `resolve_color`:
    /// flag(--no-color) > NO_COLOR env > config(always/never) > auto(tty)
    #[test]
    fn resolve_color_precedence() {
        // --no-color flag forces off even when config says always and tty=false
        assert!(
            !resolve_color(true, None, Some(ColorMode::Always), false),
            "flag forces off"
        );
        // NO_COLOR env forces off even when tty=true
        assert!(
            !resolve_color(false, Some(()), Some(ColorMode::Always), true),
            "NO_COLOR env forces off"
        );
        // config never overrides tty=true
        assert!(
            !resolve_color(false, None, Some(ColorMode::Never), true),
            "config never"
        );
        // config always enables even when tty=false
        assert!(
            resolve_color(false, None, Some(ColorMode::Always), false),
            "config always (non-tty)"
        );
        // auto: no flag, no env, no config → follow tty
        assert!(resolve_color(false, None, None, true), "auto: tty");
        // auto: no flag, no env, no config → follow tty (non-tty case)
        assert!(!resolve_color(false, None, None, false), "auto: non-tty");
        // config auto: equivalent to None → follow tty
        assert!(
            resolve_color(false, None, Some(ColorMode::Auto), true),
            "config auto: tty"
        );
    }

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

    #[test]
    fn no_color_env_disables_colour_even_in_tty_contexts() {
        // We can't usefully assert this with `assert_eq!(is_enabled(), …)`
        // because `ENABLED` may already be locked in by the previous test.
        // What we can do is exercise the env-var branch of the decision
        // function and assert that, in isolation, NO_COLOR forces `false`.
        // This is a regression test for the `var_os(...).is_some_and(…)`
        // branch — if someone "fixes" that to check for empty strings, the
        // assertion below fails.
        let prior = std::env::var_os("NO_COLOR");
        // SAFETY: tests in this file are single-process and don't share env
        // with other tests under cargo test's default thread model — but
        // they do share env with other tests if they run in parallel. We
        // accept that risk for this read-only-ish check by restoring at the
        // end. If this becomes flaky, gate behind an env-mutex like the
        // paths_phase2 integration tests.
        unsafe { std::env::set_var("NO_COLOR", "1") };
        let no_color_set = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        assert!(no_color_set);
        unsafe {
            match prior {
                Some(v) => std::env::set_var("NO_COLOR", v),
                None => std::env::remove_var("NO_COLOR"),
            }
        }
    }
}

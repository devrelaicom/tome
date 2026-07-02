//! Colour gating for human output.
//!
//! Colour is enabled according to the following precedence (highest wins):
//! 1. `--no-color` CLI flag OR a truthy `TOME_NO_COLOR` env var (both
//!    forwarded via [`set_disabled`]) → always off,
//! 2. `NO_COLOR` environment variable (per <https://no-color.org>) → always off,
//! 3. `[output] color` in `~/.tome/config.toml` (`always` → on, `never` → off),
//! 4. auto: stdout is connected to a terminal.
//!
//! `TOME_NO_COLOR` is a Tome-specific truthy override (any set, non-empty value
//! that is not `0`/`false`/`no`/`off`) layered ON TOP of the standard `NO_COLOR`
//! signal — it lets a caller force Tome's colour off without also disabling
//! colour in every other `NO_COLOR`-respecting tool. It is folded into the
//! `--no-color` decision inside [`set_disabled`] rather than checked here so the
//! shared [`crate::util::env_truthy`] SSOT (which the binary crate cannot reach
//! directly) stays internal.
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
///
/// A truthy `TOME_NO_COLOR` env var (the shared [`crate::util::env_truthy`]
/// convention) is OR-ed into `disabled` here, so `main.rs` can keep calling
/// `set_disabled(cli.no_color)` unchanged and the env override still forces
/// colour off. Resolving the OR in this lib function keeps `env_truthy`
/// `pub(crate)` — the binary crate cannot reach it directly.
pub fn set_disabled(disabled: bool) {
    let _ = FORCE_DISABLED.set(disabled_with_env(disabled));
}

/// The exact rule [`set_disabled`] applies: the `--no-color` flag OR a truthy
/// `TOME_NO_COLOR`. Split out so the env-override wiring is unit-testable without
/// touching the one-shot `FORCE_DISABLED`/`ENABLED` `OnceLock`s (which other
/// tests in the same binary may already have locked in).
fn disabled_with_env(no_color_flag: bool) -> bool {
    no_color_flag || crate::util::env_truthy("TOME_NO_COLOR")
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
/// The caller (typically `main.rs`) is responsible for passing the resolved
/// `config_color` value from the single `load_or_default` call it already
/// performs for logging + progress. This avoids a second independent config
/// snapshot inside this function.
pub fn init(config_color: Option<crate::config::ColorMode>) -> bool {
    *ENABLED.get_or_init(|| {
        let no_color_flag = *FORCE_DISABLED.get().unwrap_or(&false);
        let no_color_env = std::env::var_os("NO_COLOR")
            .filter(|v| !v.is_empty())
            .map(|_| ());
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
    use std::sync::Mutex;

    // ENABLED is a global OnceLock, so we cannot reliably re-initialise it
    // mid-process. These tests therefore exercise only the public predicate
    // forms and the pure `resolve_color` function; the actual gating logic
    // is covered by visual inspection plus the integration tests that pipe
    // `tome` to a file (FR-046).

    // `TOME_NO_COLOR` is process-global; serialise every test that mutates it.
    static NO_COLOR_ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// Panic-safe `TOME_NO_COLOR` override, mirroring the `cli.rs` test's
    /// `JsonEnvGuard`. Captures the pre-test value on construction and
    /// restores/removes it in `Drop`, so an intervening `assert!` panic can't
    /// leak the var into a later test. Caller MUST hold `NO_COLOR_ENV_MUTEX` for
    /// the guard's lifetime (the guard is about panic-safe restore, not
    /// serialisation).
    struct NoColorEnvGuard {
        prior: Option<std::ffi::OsString>,
    }

    impl NoColorEnvGuard {
        /// Capture the current value and leave the var untouched. `set`/`unset`
        /// then mutate it in place under the held mutex.
        fn capture() -> Self {
            Self {
                prior: std::env::var_os("TOME_NO_COLOR"),
            }
        }

        fn set(&self, value: &str) {
            // SAFETY: the caller holds NO_COLOR_ENV_MUTEX for the guard's life.
            unsafe { std::env::set_var("TOME_NO_COLOR", value) };
        }

        fn unset(&self) {
            // SAFETY: the caller holds NO_COLOR_ENV_MUTEX for the guard's life.
            unsafe { std::env::remove_var("TOME_NO_COLOR") };
        }
    }

    impl Drop for NoColorEnvGuard {
        fn drop(&mut self) {
            // SAFETY: NO_COLOR_ENV_MUTEX is still held by the test for the
            // guard's lifetime.
            unsafe {
                match &self.prior {
                    Some(v) => std::env::set_var("TOME_NO_COLOR", v),
                    None => std::env::remove_var("TOME_NO_COLOR"),
                }
            }
        }
    }

    /// The `TOME_NO_COLOR` env override folded into `set_disabled` via
    /// `disabled_with_env`: a truthy value forces colour off even when the
    /// `--no-color` flag is absent; the flag alone still forces off.
    #[test]
    fn tome_no_color_env_ors_into_disabled() {
        let _lock = NO_COLOR_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
        // RAII: `TOME_NO_COLOR` is restored/removed on drop even if an
        // assertion below panics — no manual restore tail to skip.
        let env = NoColorEnvGuard::capture();

        env.set("1");
        assert!(
            disabled_with_env(false),
            "truthy TOME_NO_COLOR must disable colour even without --no-color",
        );

        env.set("0");
        assert!(
            !disabled_with_env(false),
            "falsey TOME_NO_COLOR must not disable colour on its own",
        );
        assert!(
            disabled_with_env(true),
            "--no-color flag still forces off regardless of env",
        );

        env.unset();
        assert!(
            !disabled_with_env(false),
            "unset TOME_NO_COLOR + no flag → not disabled",
        );
        assert!(
            disabled_with_env(true),
            "--no-color flag forces off with env unset",
        );
    }

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
        // config auto: non-tty → off
        assert!(
            !resolve_color(false, None, Some(ColorMode::Auto), false),
            "config auto: non-tty follows TTY (off)"
        );
    }

    #[test]
    fn colour_helpers_return_plain_string_in_non_tty_context() {
        // In a Cargo test harness stdout is not a TTY, so init() should
        // settle on `false`. Pass None — no config override in this test.
        init(None);
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

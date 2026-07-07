//! `indicatif` wrappers for long-running and indeterminate operations.
//!
//! Three rules from the spec drive this module:
//!
//! 1. **FR-042** — long-running operations (> ~2 s expected: model download,
//!    batch embedding, reindex) show a *progress bar* on a connected stderr.
//! 2. **FR-043** — operations whose duration is non-trivial but not
//!    predictable in advance (git operations, model loading, DB init) show a
//!    *spinner* on a connected stderr.
//! 3. **FR-046** — both are suppressed automatically when stderr is not a
//!    terminal. We use indicatif's `ProgressDrawTarget::hidden()` for that;
//!    the `ProgressBar` API stays the same, only the rendering goes away.
//!
//! Progress is on **stderr**, not stdout, so `--json` and command output on
//! stdout stay byte-stable even when a progress bar is rendering live.
//!
//! `[output] progress = false` in `~/.tome/config.toml` suppresses progress
//! bars and spinners even on a connected TTY. `true` or absent delegates back
//! to the TTY check (the default auto behaviour).
//!
//! `--json` (or a truthy `TOME_JSON`) suppresses progress unconditionally
//! (#480): a structured-output consumer asked for machine output, so a live
//! stderr bar on a TTY is noise it never wanted. The override beats a config
//! `progress = true` and is resolved once at the [`init_progress`] call in
//! `main.rs` — the same single-snapshot boundary as colour and logging.

use std::io::IsTerminal;
use std::sync::OnceLock;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

/// Cached progress-enabled decision. Computed once at startup via
/// [`init_progress`]; `None` until then (auto-detect at each call site).
static PROGRESS_ENABLED: OnceLock<bool> = OnceLock::new();

/// Resolve and cache the progress-enabled decision from config.
///
/// Call this once at CLI startup (after config is available) on the non-MCP
/// path. The MCP server never shows progress — do not call this there.
///
/// `config_progress`:
/// - `Some(false)` → suppress even on a TTY,
/// - `Some(true)` or `None` → auto (follow stderr TTY).
///
/// `json_mode` (#480): `true` when the invocation asked for structured
/// output (`--json` / truthy `TOME_JSON`) — suppresses progress
/// unconditionally, beating a config `progress = true`.
pub fn init_progress(config_progress: Option<bool>, json_mode: bool) {
    let _ = PROGRESS_ENABLED.set(resolve_progress(
        config_progress,
        stderr_is_tty(),
        json_mode,
    ));
}

/// Pure progress-enabled resolver. Separated from global state for
/// unit-testability.
///
/// - `json` → always hidden (#480: machine output implies a quiet stderr),
/// - `Some(false)` → always hidden,
/// - `Some(true)` | `None` → honour `is_tty`.
pub(crate) fn resolve_progress(config_progress: Option<bool>, is_tty: bool, json: bool) -> bool {
    if json {
        return false;
    }
    match config_progress {
        Some(false) => false,
        Some(true) | None => is_tty,
    }
}

/// Whether stderr is a real terminal. Mirrors [`crate::output::stdout_is_tty`]
/// but on the diagnostic stream where progress rendering happens.
pub fn stderr_is_tty() -> bool {
    std::io::stderr().is_terminal()
}

fn target() -> ProgressDrawTarget {
    // If init_progress has been called, use the cached decision.
    // If not (e.g. in tests or library use), fall back to the TTY check.
    let show = PROGRESS_ENABLED
        .get()
        .copied()
        .unwrap_or_else(stderr_is_tty);
    if show {
        ProgressDrawTarget::stderr()
    } else {
        ProgressDrawTarget::hidden()
    }
}

/// A determinate progress bar with a known total. Use for model downloads
/// (bytes/total/speed), embedding generation (skills/total), and reindex
/// operations. The returned bar renders on stderr when stderr is a TTY and
/// is a no-op handle otherwise.
pub fn bar(total: u64, message: impl Into<String>) -> ProgressBar {
    let pb = ProgressBar::with_draw_target(Some(total), target());
    pb.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold} [{bar:40.cyan/blue}] {pos}/{len} ({eta_precise})",
        )
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("##-"),
    );
    pb.set_prefix(message.into());
    pb
}

/// A bytes-aware determinate progress bar. Use for network downloads where
/// the total byte count is known up front.
pub fn byte_bar(total_bytes: u64, message: impl Into<String>) -> ProgressBar {
    let pb = ProgressBar::with_draw_target(Some(total_bytes), target());
    pb.set_style(
        ProgressStyle::with_template(
            "{prefix:.bold} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta_precise})",
        )
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("##-"),
    );
    pb.set_prefix(message.into());
    pb
}

/// An indeterminate spinner. Use for opaque or short-but-unknown-length work
/// (git fetch, model load, DB open).
pub fn spinner(message: impl Into<String>) -> ProgressBar {
    let pb = ProgressBar::with_draw_target(None, target());
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {prefix}")
            .unwrap_or_else(|_| ProgressStyle::default_spinner()),
    );
    pb.set_prefix(message.into());
    pb.enable_steady_tick(Duration::from_millis(120));
    pb
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the progress suppression logic independent of global state.
    #[test]
    fn resolve_progress_suppression() {
        // config false → always hidden regardless of tty
        assert!(
            !resolve_progress(Some(false), true, false),
            "config false on tty"
        );
        assert!(
            !resolve_progress(Some(false), false, false),
            "config false off tty"
        );
        // config true → follow tty
        assert!(
            resolve_progress(Some(true), true, false),
            "config true on tty"
        );
        assert!(
            !resolve_progress(Some(true), false, false),
            "config true off tty"
        );
        // no config → follow tty
        assert!(resolve_progress(None, true, false), "auto on tty");
        assert!(!resolve_progress(None, false, false), "auto off tty");
    }

    /// #480: `--json` implies a quiet stderr — the override beats every other
    /// combination, including an explicit config `progress = true` on a TTY.
    #[test]
    fn json_mode_suppresses_progress_unconditionally() {
        assert!(
            !resolve_progress(Some(true), true, true),
            "json beats config true on tty"
        );
        assert!(
            !resolve_progress(None, true, true),
            "json beats auto on tty"
        );
        assert!(
            !resolve_progress(Some(false), true, true),
            "json + config false stays hidden"
        );
        assert!(!resolve_progress(None, false, true), "json off tty hidden");
    }

    #[test]
    fn bar_with_zero_total_does_not_panic() {
        let pb = bar(0, "noop");
        pb.inc(1); // > total should saturate rather than panic
        pb.finish_and_clear();
    }

    #[test]
    fn byte_bar_constructs_and_increments() {
        let pb = byte_bar(1024, "downloading model");
        pb.inc(512);
        pb.finish_and_clear();
    }

    #[test]
    fn spinner_constructs_and_finishes() {
        let pb = spinner("loading");
        pb.finish_and_clear();
    }

    #[test]
    fn hidden_target_is_used_when_stderr_is_not_a_tty() {
        // The test harness redirects stderr, so stderr_is_tty() is false and
        // we should be using the hidden draw target. We cannot inspect the
        // target directly, but a bar built this way must accept rapid updates
        // and finish cleanly without rendering.
        let pb = bar(100, "test");
        for _ in 0..1000 {
            pb.inc(1);
        }
        pb.finish_and_clear();
        // Reaching here without panicking is the assertion.
    }
}

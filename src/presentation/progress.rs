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

use std::io::IsTerminal;
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

/// Whether stderr is a real terminal. Mirrors [`crate::output::stdout_is_tty`]
/// but on the diagnostic stream where progress rendering happens.
pub fn stderr_is_tty() -> bool {
    std::io::stderr().is_terminal()
}

fn target() -> ProgressDrawTarget {
    if stderr_is_tty() {
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

// NOTE: unit tests for this module are intentionally deferred to integration.
// indicatif's draw-target threading and steady-tick spawns interact with the
// stdin/stderr that `git push` passes to the pre-push hook in a way that hangs
// the test runner reproducibly when invoked through the hook chain (the
// underlying issue is tracked in `specs/002-phase-2-plugins-index/retro/P2.md`).
//
// The module's correctness is covered downstream by:
//   - `tests/plugin_enable.rs` — exercises the byte bar via the embedding
//     pipeline once the plugin lifecycle lands;
//   - `tests/models_download.rs` — exercises the byte bar with a local
//     fixture HTTP server;
//   - `tests/catalog_update_reindex.rs` — exercises the determinate bar.
//
// The TTY-detection branch (`target()` returns hidden when stderr is not a
// terminal) is also indirectly verified by SC-007 (queries piped to a file
// produce deterministic, terminal-independent output).

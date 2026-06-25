//! The first-run telemetry opt-out notice (FR-013/014/015).
//!
//! CLI-only, printed to **stderr exactly once** — on the first run that mints the
//! install id. The "exactly once" guarantee rides on the kernel's atomic id mint:
//! [`crate::telemetry::init`] records `MINTED_THIS_RUN` when it builds an enabled
//! handle for an install whose id file was absent, and
//! [`crate::telemetry::cli_startup`] prints this notice only on that observation.
//!
//! The wording is deliberately forward-looking ("will be seen on the next run",
//! not "already sent"): at the moment we print, nothing has been delivered yet
//! (FR-015 — no over-claim). The MCP surface never calls this (a server has no
//! stderr a human reads).
//!
//! Content (FR-013): the single line discloses BOTH telemetry streams — the
//! anonymous usage data AND the named usage of plugins from allowlisted catalogs
//! (currently Midnight) shared with that catalog's publisher — plus the opt-out
//! mechanism and a pointer to `tome telemetry --help` for the full detail.

/// Print the one-line opt-out notice to stderr.
///
/// Plain text only (no color) — it goes to stderr regardless of `NO_COLOR`/TTY,
/// so it never pollutes `--json` stdout and never needs styling. Best-effort: a
/// failed write to stderr is not actionable.
pub fn print_first_run_notice() {
    eprintln!(
        "Tome collects anonymous usage telemetry, plus named usage of plugins from \
         allowlisted catalogs (currently Midnight) shared with that catalog's publisher, \
         to help improve the project. It's opt-out — run `tome telemetry off` (or set \
         TOME_TELEMETRY=0). See `tome telemetry --help`."
    );
}

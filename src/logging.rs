//! `tracing-subscriber` wiring. Diagnostic output goes to stderr only and is
//! orthogonal to `--json` (FR-019b) — `--json` shapes the command's primary
//! output on stdout, while logs remain human-readable on stderr.
//!
//! Verbosity sources, in precedence order: `-v`/`-vv` on the CLI, then
//! `TOME_LOG`, then `RUST_LOG`, then the default (`warn`).

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

#[derive(Debug, Clone, Copy, Default)]
pub enum Verbosity {
    #[default]
    Default,
    Verbose,
    Debug,
}

impl Verbosity {
    pub fn from_count(count: u8) -> Self {
        match count {
            0 => Self::Default,
            1 => Self::Verbose,
            _ => Self::Debug,
        }
    }

    fn default_directive(self) -> &'static str {
        match self {
            Self::Default => "warn",
            Self::Verbose => "info",
            Self::Debug => "debug",
        }
    }
}

/// Initialise the global tracing subscriber. Safe to call once at startup.
pub fn init(verbosity: Verbosity) {
    let filter = EnvFilter::try_from_env("TOME_LOG")
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new(verbosity.default_directive()));

    let fmt_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .without_time();

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .try_init();
}

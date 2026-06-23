//! `tracing-subscriber` wiring. Diagnostic output goes to stderr only and is
//! orthogonal to `--json` (FR-019b) — `--json` shapes the command's primary
//! output on stdout, while logs remain human-readable on stderr.
//!
//! Verbosity sources, in precedence order: `-v`/`-vv` on the CLI, then
//! `TOME_LOG`, then `RUST_LOG`, then `[logging] level` in `config.toml`,
//! then the built-in default (`warn`).

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

    /// True when an explicit `-v`/`-vv` flag was passed (i.e. not the
    /// default). Used by [`resolve_directive`] to determine whether the
    /// flag takes precedence over env and config.
    pub(crate) fn is_explicit(self) -> bool {
        !matches!(self, Self::Default)
    }

    fn default_directive(self) -> &'static str {
        match self {
            Self::Default => "warn",
            Self::Verbose => "info",
            Self::Debug => "debug",
        }
    }
}

/// Pure precedence resolver for the effective `EnvFilter` directive.
///
/// Precedence (highest to lowest):
/// 1. `-v`/`-vv` flag — only when `verbosity` is `Some(v)` and `v.is_explicit()`
///    (i.e. a non-default [`Verbosity`]). Pass `None` for callers that have no
///    verbosity flag (e.g. the MCP server).
/// 2. `TOME_LOG` env var (`tome_log`, non-empty)
/// 3. `RUST_LOG` env var (`rust_log`, non-empty)
/// 4. `[logging] level` in `config.toml` (`config_level`)
/// 5. `default` — callers supply the right default: `"warn"` for the CLI,
///    `"info"` for the MCP server.
///
/// `tome_log` / `rust_log` are the raw env values (pass `None` when unset) so
/// this function is pure and unit-testable without touching the real environment.
pub(crate) fn resolve_directive(
    verbosity: Option<Verbosity>,
    config_level: Option<crate::config::LogLevel>,
    tome_log: Option<String>,
    rust_log: Option<String>,
    default: &str,
) -> String {
    if let Some(v) = verbosity
        && v.is_explicit()
    {
        return v.default_directive().to_string();
    }
    if let Some(d) = tome_log.filter(|s| !s.is_empty()) {
        return d;
    }
    if let Some(d) = rust_log.filter(|s| !s.is_empty()) {
        return d;
    }
    if let Some(level) = config_level {
        return level.as_directive().to_string();
    }
    default.to_string()
}

/// Initialise the global tracing subscriber. Safe to call once at startup.
///
/// `config_level` is the `[logging] level` value from `~/.tome/config.toml`,
/// loaded defensively via `config::load_or_default` so a malformed config
/// never prevents logging from starting. The env vars (`TOME_LOG`, `RUST_LOG`)
/// and the `-v`/`-vv` flag take precedence over this value.
pub fn init(verbosity: Verbosity, config_level: Option<crate::config::LogLevel>) {
    let directive = resolve_directive(
        Some(verbosity),
        config_level,
        std::env::var("TOME_LOG").ok(),
        std::env::var("RUST_LOG").ok(),
        "warn",
    );

    let filter = EnvFilter::new(directive);

    let fmt_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .without_time();

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LogLevel;

    #[test]
    fn config_level_used_when_no_flag_no_env() {
        // verbosity default (no -v), no TOME_LOG/RUST_LOG → config level wins.
        let directive = resolve_directive(
            Some(Verbosity::default()),
            Some(LogLevel::Info),
            None,
            None,
            "warn",
        );
        assert_eq!(directive, "info");
    }

    #[test]
    fn flag_beats_config() {
        let directive = resolve_directive(
            Some(Verbosity::from_count(1)),
            Some(LogLevel::Error),
            None,
            None,
            "warn",
        );
        assert_eq!(directive, "info"); // -v = info, beats config error
    }

    #[test]
    fn env_beats_config() {
        let directive = resolve_directive(
            Some(Verbosity::default()),
            Some(LogLevel::Error),
            Some("debug".into()),
            None,
            "warn",
        );
        assert_eq!(directive, "debug"); // TOME_LOG=debug beats config
    }

    #[test]
    fn default_when_nothing_set() {
        assert_eq!(
            resolve_directive(Some(Verbosity::default()), None, None, None, "warn"),
            "warn"
        );
    }

    #[test]
    fn rust_log_beats_config() {
        let directive = resolve_directive(
            Some(Verbosity::default()),
            Some(LogLevel::Warn),
            None,
            Some("trace".into()),
            "warn",
        );
        assert_eq!(directive, "trace");
    }

    #[test]
    fn tome_log_beats_rust_log() {
        let directive = resolve_directive(
            Some(Verbosity::default()),
            None,
            Some("info".into()),
            Some("trace".into()),
            "warn",
        );
        assert_eq!(directive, "info");
    }

    #[test]
    fn flag_beats_tome_log() {
        let directive = resolve_directive(
            Some(Verbosity::from_count(1)),
            None,
            Some("error".into()),
            None,
            "warn",
        );
        assert_eq!(directive, "info"); // -v = info
    }

    #[test]
    fn double_v_gives_debug() {
        let directive = resolve_directive(Some(Verbosity::from_count(2)), None, None, None, "warn");
        assert_eq!(directive, "debug");
    }

    #[test]
    fn empty_tome_log_falls_through_to_config() {
        let directive = resolve_directive(
            Some(Verbosity::default()),
            Some(LogLevel::Debug),
            Some(String::new()),
            None,
            "warn",
        );
        assert_eq!(directive, "debug");
    }

    // MCP-shape tests: verbosity=None, default="info"

    #[test]
    fn mcp_default_is_info() {
        // No verbosity flag, no env, no config → MCP default "info".
        let directive = resolve_directive(None, None, None, None, "info");
        assert_eq!(directive, "info");
    }

    #[test]
    fn mcp_config_beats_default() {
        // No verbosity flag, no env, config=debug → "debug" (config beats "info" default).
        let directive = resolve_directive(None, Some(LogLevel::Debug), None, None, "info");
        assert_eq!(directive, "debug");
    }

    #[test]
    fn empty_rust_log_falls_through_to_config() {
        // RUST_LOG="" with config_level=debug, no TOME_LOG, no explicit verbosity → "debug".
        let directive = resolve_directive(
            Some(Verbosity::default()),
            Some(LogLevel::Debug),
            None,
            Some(String::new()),
            "warn",
        );
        assert_eq!(directive, "debug");
    }
}

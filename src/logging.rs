//! `tracing-subscriber` wiring. Diagnostic output goes to stderr only and is
//! orthogonal to `--json` (FR-019b) — `--json` shapes the command's primary
//! output on stdout, while logs remain human-readable on stderr.
//!
//! Verbosity sources, in precedence order: `-v`/`-vv` on the CLI, then
//! `TOME_LOG`, then `RUST_LOG`, then `[logging] level` in `config.toml`,
//! then the built-in default ([`DEFAULT_CLI_DIRECTIVE`] — `warn` with the
//! benign llama.cpp/ggml chatter demoted to `error`).

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// The default CLI `EnvFilter` directive.
///
/// A bare `"warn"` lets through a benign llama.cpp chatter line that the
/// bundled summariser triggers on every run (issue #501):
///
/// ```text
/// WARN n_ctx_seq (4096) < n_ctx_train (32768) -- the full capacity of the
/// model will not be utilized module="llama.cpp::llama_context"
/// ```
///
/// llama.cpp / ggml C-side logs are routed through `tracing` under the target
/// `llama-cpp-2` (the string literal in `llama_cpp_2`'s `send_logs_to_tracing`
/// `Metadata`; the `module="llama.cpp::…"` text is a *field*, not the target).
/// Demoting that one target to `error` in the default directive hides the
/// benign WARN/INFO noise while still surfacing genuine llama.cpp errors.
///
/// This lives in the *default* directive only: an explicit `-v`/`-vv` flag, a
/// `TOME_LOG` / `RUST_LOG` value, or a `[logging] level` in `config.toml` all
/// take precedence (see [`resolve_directive`]) and restore full verbosity.
pub(crate) const DEFAULT_CLI_DIRECTIVE: &str = "warn,llama-cpp-2=error";

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
/// 5. `default` — callers supply the right default: [`DEFAULT_CLI_DIRECTIVE`]
///    for the CLI, `"info"` for the MCP server.
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
        DEFAULT_CLI_DIRECTIVE,
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
    fn cli_default_directive_demotes_llama_target() {
        // Issue #501: at the default log level, the bundled summariser's
        // benign llama.cpp `n_ctx` WARN must be suppressed. The default CLI
        // directive keeps `warn` globally but pins the `llama-cpp-2` tracing
        // target — the one every llama.cpp/ggml C-side log is routed under —
        // to `error`, so only genuine llama.cpp errors surface.
        assert_eq!(DEFAULT_CLI_DIRECTIVE, "warn,llama-cpp-2=error");

        // With no flag / env / config override, the resolver returns the CLI
        // default unchanged (the demotion is part of the default itself).
        let directive = resolve_directive(
            Some(Verbosity::default()),
            None,
            None,
            None,
            DEFAULT_CLI_DIRECTIVE,
        );
        assert_eq!(directive, DEFAULT_CLI_DIRECTIVE);

        // The composed directive parses into a valid `EnvFilter` (a malformed
        // target would panic on `EnvFilter::new`, e.g. via bad `-` handling).
        let _ = EnvFilter::new(directive);
    }

    #[test]
    fn cli_default_directive_filters_llama_target_semantically() {
        // Assert the *semantics*, not just the string: build the real
        // `EnvFilter` from the default directive and confirm the hyphenated
        // `llama-cpp-2` target actually resolves as intended. The hyphen is
        // unusual for `EnvFilter`, so this guards against a silent regression
        // if `tracing-subscriber`'s directive matching changes on upgrade.
        use tracing::Level;
        use tracing_subscriber::layer::SubscriberExt;

        let subscriber = tracing_subscriber::registry().with(EnvFilter::new(DEFAULT_CLI_DIRECTIVE));

        tracing::subscriber::with_default(subscriber, || {
            // The benign llama.cpp chatter (WARN/INFO) on the `llama-cpp-2`
            // target is suppressed — this is the whole point of issue #501.
            assert!(
                !tracing::enabled!(target: "llama-cpp-2", Level::WARN),
                "llama-cpp-2 WARN must be filtered out at the default level"
            );
            assert!(
                !tracing::enabled!(target: "llama-cpp-2", Level::INFO),
                "llama-cpp-2 INFO must be filtered out at the default level"
            );

            // A genuine llama.cpp ERROR still surfaces.
            assert!(
                tracing::enabled!(target: "llama-cpp-2", Level::ERROR),
                "llama-cpp-2 ERROR must still pass the default filter"
            );

            // Unrelated targets keep the global `warn` default: their WARN
            // passes and their INFO does not.
            assert!(
                tracing::enabled!(target: "tome", Level::WARN),
                "non-llama WARN must still pass at the default level"
            );
            assert!(
                !tracing::enabled!(target: "tome", Level::INFO),
                "non-llama INFO must be filtered out at the default level"
            );
        });
    }

    #[test]
    fn explicit_verbosity_overrides_llama_demotion() {
        // `-vv` must restore full verbosity for the llama.cpp target too: the
        // flag branch wins outright, so the demotion in the default drops out.
        let directive = resolve_directive(
            Some(Verbosity::from_count(2)),
            None,
            None,
            None,
            DEFAULT_CLI_DIRECTIVE,
        );
        assert_eq!(directive, "debug");
        assert!(!directive.contains("llama-cpp-2"));
    }

    #[test]
    fn env_override_replaces_llama_demotion() {
        // An explicit `TOME_LOG` / `RUST_LOG` fully replaces the default (and
        // therefore its llama demotion) — the user asked for verbatim control.
        for env in [
            resolve_directive(
                Some(Verbosity::default()),
                None,
                Some("trace".into()),
                None,
                DEFAULT_CLI_DIRECTIVE,
            ),
            resolve_directive(
                Some(Verbosity::default()),
                None,
                None,
                Some("trace".into()),
                DEFAULT_CLI_DIRECTIVE,
            ),
        ] {
            assert_eq!(env, "trace");
            assert!(!env.contains("llama-cpp-2"));
        }
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

//! Shared environment-variable boolean parsing.
//!
//! Two consumers now interpret a boolean-valued env var with the identical
//! rule: `telemetry::config::is_ci` (the CI auto-disable gate, e.g. `CI=1`) and
//! `presentation::prompt::non_interactive` (`TOME_NONINTERACTIVE=1`). Rather than
//! duplicate the parse at the second consumer, it is promoted here as the single
//! source of truth (the SSOT-at-the-second-consumer pattern) so the truthy set
//! can never drift between the two surfaces.
//!
//! Sync-only, like the rest of `src/util/`.

/// Truthy-presence for a boolean-valued token: non-empty (after trimming) and
/// NOT one of the explicit falsey tokens `0`/`false`/`no`/`off`
/// (case-insensitive). This is the core rule; [`env_truthy`] applies it to an
/// env var's value.
pub(crate) fn truthy(value: &str) -> bool {
    let v = value.trim();
    !v.is_empty()
        && !matches!(
            v.to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off"
        )
}

/// Whether the env var `name` is set to a truthy value per [`truthy`]. An unset
/// var (or one holding non-UTF-8 bytes, which `std::env::var` reports as absent)
/// is false.
pub(crate) fn env_truthy(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| truthy(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthy_accepts_set_non_empty_non_falsey() {
        for v in ["1", "true", "TRUE", "yes", "on", "anything", " 1 "] {
            assert!(truthy(v), "{v:?} should be truthy");
        }
    }

    #[test]
    fn truthy_rejects_empty_and_falsey_tokens() {
        for v in ["", "  ", "0", "false", "FALSE", "no", "NO", "off", "OFF"] {
            assert!(!truthy(v), "{v:?} should be falsey");
        }
    }

    #[test]
    fn env_truthy_reads_the_var() {
        // Unset → false (unique key, no other test observes it).
        assert!(!env_truthy("TOME_TEST_UTIL_ENV_DEFINITELY_UNSET"));
    }
}

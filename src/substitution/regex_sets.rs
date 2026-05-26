//! Compiled-regex cache slots for the substitution stages.
//!
//! Compiled lazily on first use via `OnceLock::get_or_init` (US2/US3).
//!
//! ## Single-sweep design (US2.d B2 fix)
//!
//! Stages 1 + 2 are scanned in a SINGLE regex pass via [`combined_regex`].
//! Each match's resolved value is emitted directly into the output buffer
//! and the loop never re-scans the substituted text. This is the
//! structural enforcement of the no-rescan invariant (NFR-007 / FR-051):
//! a Stage-1 built-in resolving to `${TOME_ENV_LEAKED}` literal cannot
//! exfiltrate the operator's `TOME_ENV_LEAKED` host env var, because the
//! resolved value never re-enters the scanner.
//!
//! Filename note: this module is `regex_sets` rather than `regex` so it
//! doesn't shadow the [`regex`] crate inside the `substitution` module
//! tree, which would force every reference to the crate to use the
//! awkward `::regex::Regex` absolute path.

use regex::Regex;
use std::sync::OnceLock;

/// Compiled regex for the unified Stage 1 + Stage 2 sweep. Populated by
/// [`combined_regex`] on first call (US2.d B2 fix).
pub(super) static COMBINED_RE: OnceLock<Regex> = OnceLock::new();

/// Compiled regex for the `$ARGUMENTS` / `$N` / `$NAME` arguments
/// stage. Populated by US3.
#[allow(dead_code)]
pub(super) static ARGUMENTS_RE: OnceLock<Regex> = OnceLock::new();

/// Return the lazy-compiled regex for the unified Stage 1 + Stage 2
/// pattern.
///
/// Pattern: `\$\{TOME_(?:ENV_([A-Z0-9_]+)|([A-Z0-9_]+))(?::-(.*?))?\}`
///
/// - Capture group 1 — when set, the suffix after `TOME_ENV_`. The
///   caller resolves via `std::env::var(format!("TOME_ENV_{name}"))`.
/// - Capture group 2 — when set, a Stage 1 built-in name (e.g.
///   `SKILL_NAME`, `PLUGIN_VERSION`, `WORKSPACE_DATA`).
/// - Capture group 3 — optional `:-default` value (applies to whichever
///   branch matched).
///
/// The two alternatives are LEFT-MOST-FIRST per the regex crate
/// semantics: a body containing `${TOME_ENV_FOO}` always takes the env
/// branch (group 1 set, group 2 unset), not the built-ins branch. This
/// avoids the previous double-pass surface where a Stage 1 built-in
/// resolving to `${TOME_ENV_X}` would later have its text re-scanned by
/// Stage 2 — the structural fix for the no-rescan invariant
/// (NFR-007 / FR-051) and the exfiltration vector closed by US2.d B2.
///
/// Per FR-033 + NFR-005, the `TOME_` namespace prefix is mandatory:
/// references outside (`${GITHUB_TOKEN}`, `${PATH}`, …) MUST NOT match.
///
/// The pattern is a constant — `Regex::new` cannot fail at runtime, so
/// the unreachable case is `expect`ed with a clear message rather than
/// propagated as `Result`.
pub(crate) fn combined_regex() -> &'static Regex {
    COMBINED_RE.get_or_init(|| {
        Regex::new(r"\$\{TOME_(?:ENV_([A-Z0-9_]+)|([A-Z0-9_]+))(?::-(.*?))?\}")
            .expect("COMBINED_RE must compile (constant pattern)")
    })
}

#[cfg(test)]
mod tests {
    use super::combined_regex;

    #[test]
    fn env_branch_captures_group_1_only() {
        let re = combined_regex();
        let caps = re
            .captures("${TOME_ENV_FOO}")
            .expect("env reference matches");
        assert_eq!(caps.get(1).map(|m| m.as_str()), Some("FOO"));
        assert_eq!(caps.get(2), None);
        assert_eq!(caps.get(3), None);
    }

    #[test]
    fn builtin_branch_captures_group_2_only() {
        let re = combined_regex();
        let caps = re
            .captures("${TOME_SKILL_NAME}")
            .expect("built-in reference matches");
        assert_eq!(caps.get(1), None);
        assert_eq!(caps.get(2).map(|m| m.as_str()), Some("SKILL_NAME"));
        assert_eq!(caps.get(3), None);
    }

    #[test]
    fn env_branch_with_default_captures_group_1_and_3() {
        let re = combined_regex();
        let caps = re
            .captures("${TOME_ENV_FOO:-fallback}")
            .expect("env+default matches");
        assert_eq!(caps.get(1).map(|m| m.as_str()), Some("FOO"));
        assert_eq!(caps.get(2), None);
        assert_eq!(caps.get(3).map(|m| m.as_str()), Some("fallback"));
    }

    #[test]
    fn builtin_branch_with_default_captures_group_2_and_3() {
        let re = combined_regex();
        let caps = re
            .captures("${TOME_SKILL_NAME:-fallback}")
            .expect("built-in+default matches");
        assert_eq!(caps.get(1), None);
        assert_eq!(caps.get(2).map(|m| m.as_str()), Some("SKILL_NAME"));
        assert_eq!(caps.get(3).map(|m| m.as_str()), Some("fallback"));
    }

    #[test]
    fn non_namespace_reference_does_not_match() {
        let re = combined_regex();
        assert!(re.captures("${GITHUB_TOKEN}").is_none());
        assert!(re.captures("${PATH}").is_none());
    }

    #[test]
    fn alternation_is_left_most_first_env_wins() {
        // `ENV_FOO` could in principle be parsed as a built-in `ENV_FOO`
        // name (it matches `[A-Z0-9_]+`). The leftmost alternation
        // guarantees the env branch wins so we never silently expose
        // `${TOME_ENV_*}` references as Stage-1 unknown built-ins.
        let re = combined_regex();
        let caps = re.captures("${TOME_ENV_FOO}").unwrap();
        assert!(caps.get(1).is_some(), "env branch must win on ENV_ prefix");
        assert!(caps.get(2).is_none());
    }
}

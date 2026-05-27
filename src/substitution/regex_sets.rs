//! Compiled-regex cache slots for the substitution stages.
//!
//! Compiled lazily on first use via `OnceLock::get_or_init`.
//!
//! ## Single-sweep design (US2.d B2 fix + US3 extension)
//!
//! Stages 1, 2, AND 3 are scanned in a SINGLE regex pass via
//! [`combined_regex`]. Each match's resolved value is emitted directly
//! into the output buffer and the loop never re-scans the substituted
//! text. This is the structural enforcement of the no-rescan invariant
//! (NFR-007 / FR-051):
//!
//! - A Stage-1 built-in resolving to `${TOME_ENV_LEAKED}` literal cannot
//!   exfiltrate the operator's `TOME_ENV_LEAKED` host env var, because
//!   the resolved value never re-enters the scanner.
//! - A Stage-3 caller argument resolving to `${TOME_ENV_LEAKED}` cannot
//!   leak either, for the same structural reason.
//! - A Stage-1 built-in resolving to `$0` cannot be hijacked by a
//!   subsequent Stage-3 positional substitution. A hostile plugin
//!   setting `"plugin_name": "$0"` (so any `${TOME_PLUGIN_NAME}` body
//!   resolves to `$0`) cannot trigger argument substitution on the
//!   value it produced.
//!
//! Filename note: this module is `regex_sets` rather than `regex` so it
//! doesn't shadow the [`regex`] crate inside the `substitution` module
//! tree, which would force every reference to the crate to use the
//! awkward `::regex::Regex` absolute path.

use regex::Regex;
use std::sync::OnceLock;

/// Compiled regex for the unified Stage 1 + Stage 2 + Stage 3 sweep.
/// Populated by [`combined_regex`] on first call.
///
/// Stage 3 was originally planned as a separate `ARGUMENTS_RE` slot in
/// F3; US3.a folded it into this unified regex per Option A from the
/// US3 brief (structural enforcement of the no-rescan invariant). The
/// separate slot was removed when Stage 3 landed — leaving it as dead
/// code would invite future drift back into the double-pass design.
pub(super) static COMBINED_RE: OnceLock<Regex> = OnceLock::new();

// Capture-group indices for the unified pattern. Kept as named
// constants so the dispatch in `render()` reads as code, not magic
// numbers. Group 0 is always the full match.
//
// Stage 1 + 2 share groups 1–3 (left-hand alternative); Stage 3 lives
// in groups 4–6 (right-hand alternatives). Exactly one of groups 1, 2,
// 4, 5, 6 is populated on any successful match — OR the match's literal
// text is `$ARGUMENTS` (the bare-keyword alternative has no capture
// group, so the dispatcher checks `m.as_str()` for that one shape).

/// `${TOME_ENV_<NAME>}` env reference — captures NAME (suffix after
/// `TOME_ENV_`).
pub(super) const ENV_NAME_GROUP: usize = 1;

/// `${TOME_<BUILTIN>}` reference — captures BUILTIN (e.g. `SKILL_NAME`,
/// `PLUGIN_VERSION`).
pub(super) const BUILTIN_NAME_GROUP: usize = 2;

/// `:-default` value, applies to whichever stage-1/2 branch matched.
pub(super) const DEFAULT_GROUP: usize = 3;

/// `$ARGUMENTS[N]` — captures N as a digit run.
pub(super) const ARG_INDEX_GROUP: usize = 4;

/// `$N` — captures N as a digit run.
pub(super) const POSITIONAL_GROUP: usize = 5;

/// `$<name>` — captures the name (lowercase identifier).
pub(super) const NAMED_GROUP: usize = 6;

/// Return the lazy-compiled regex for the unified single-sweep pattern.
///
/// Pattern:
/// `\$\{TOME_(?:ENV_([A-Z0-9_]+)|([A-Z0-9_]+))(?::-(.*?))?\}|\$ARGUMENTS\[(\d+)\]|\$ARGUMENTS|\$(\d+)|\$([a-z_][a-z0-9_]*)`
///
/// Branches (LEFT-most-first per the `regex` crate's leftmost-first
/// alternation semantics):
///
/// 1. `${TOME_ENV_<NAME>}[:-default]` — Stage 2 (env passthrough).
///    Group [`ENV_NAME_GROUP`] = NAME; group [`DEFAULT_GROUP`] = default.
/// 2. `${TOME_<BUILTIN>}[:-default]` — Stage 1 (built-in).
///    Group [`BUILTIN_NAME_GROUP`] = BUILTIN; group [`DEFAULT_GROUP`] = default.
/// 3. `$ARGUMENTS[N]` — Stage 3 (indexed positional).
///    Group [`ARG_INDEX_GROUP`] = N.
/// 4. `$ARGUMENTS` (bare) — Stage 3 (whole-positional join).
///    No capture group; dispatcher matches on `m.as_str() == "$ARGUMENTS"`.
///    Listed AFTER `$ARGUMENTS[N]` so the leftmost-first rule lets the
///    longer alternative win when both could match.
/// 5. `$N` — Stage 3 (bare positional).
///    Group [`POSITIONAL_GROUP`] = N.
/// 6. `$<name>` — Stage 3 (named).
///    Group [`NAMED_GROUP`] = name.
///
/// Per FR-033 + NFR-005, the `TOME_` namespace prefix is mandatory for
/// stages 1 + 2: references outside (`${GITHUB_TOKEN}`, `${PATH}`, …)
/// MUST NOT match those branches.
///
/// Stage 3's `$<name>` branch is case-sensitive lowercase per FR-040
/// (`[a-z_][a-z0-9_]*`): this prevents `$PATH`-style env references
/// from accidentally matching Stage 3.
///
/// The pattern is a constant — `Regex::new` cannot fail at runtime, so
/// the unreachable case is `expect`ed with a clear message rather than
/// propagated as `Result`.
pub(crate) fn combined_regex() -> &'static Regex {
    COMBINED_RE.get_or_init(|| {
        Regex::new(
            r"\$\{TOME_(?:ENV_([A-Z0-9_]+)|([A-Z0-9_]+))(?::-(.*?))?\}|\$ARGUMENTS\[(\d+)\]|\$ARGUMENTS|\$(\d+)|\$([a-z_][a-z0-9_]*)",
        )
        .expect("COMBINED_RE must compile (constant pattern)")
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ARG_INDEX_GROUP, BUILTIN_NAME_GROUP, DEFAULT_GROUP, ENV_NAME_GROUP, NAMED_GROUP,
        POSITIONAL_GROUP, combined_regex,
    };

    // --- Stage 1 + 2 (kept from US2.d) -----------------------------------

    #[test]
    fn env_branch_captures_group_1_only() {
        let re = combined_regex();
        let caps = re
            .captures("${TOME_ENV_FOO}")
            .expect("env reference matches");
        assert_eq!(caps.get(ENV_NAME_GROUP).map(|m| m.as_str()), Some("FOO"));
        assert_eq!(caps.get(BUILTIN_NAME_GROUP), None);
        assert_eq!(caps.get(DEFAULT_GROUP), None);
    }

    #[test]
    fn builtin_branch_captures_group_2_only() {
        let re = combined_regex();
        let caps = re
            .captures("${TOME_SKILL_NAME}")
            .expect("built-in reference matches");
        assert_eq!(caps.get(ENV_NAME_GROUP), None);
        assert_eq!(
            caps.get(BUILTIN_NAME_GROUP).map(|m| m.as_str()),
            Some("SKILL_NAME")
        );
        assert_eq!(caps.get(DEFAULT_GROUP), None);
    }

    #[test]
    fn env_branch_with_default_captures_group_1_and_3() {
        let re = combined_regex();
        let caps = re
            .captures("${TOME_ENV_FOO:-fallback}")
            .expect("env+default matches");
        assert_eq!(caps.get(ENV_NAME_GROUP).map(|m| m.as_str()), Some("FOO"));
        assert_eq!(caps.get(BUILTIN_NAME_GROUP), None);
        assert_eq!(
            caps.get(DEFAULT_GROUP).map(|m| m.as_str()),
            Some("fallback")
        );
    }

    #[test]
    fn builtin_branch_with_default_captures_group_2_and_3() {
        let re = combined_regex();
        let caps = re
            .captures("${TOME_SKILL_NAME:-fallback}")
            .expect("built-in+default matches");
        assert_eq!(caps.get(ENV_NAME_GROUP), None);
        assert_eq!(
            caps.get(BUILTIN_NAME_GROUP).map(|m| m.as_str()),
            Some("SKILL_NAME")
        );
        assert_eq!(
            caps.get(DEFAULT_GROUP).map(|m| m.as_str()),
            Some("fallback")
        );
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
        assert!(
            caps.get(ENV_NAME_GROUP).is_some(),
            "env branch must win on ENV_ prefix"
        );
        assert!(caps.get(BUILTIN_NAME_GROUP).is_none());
    }

    // --- Stage 3 (new in US3.a) ------------------------------------------

    #[test]
    fn argument_index_captures_group_4_only() {
        let re = combined_regex();
        let caps = re.captures("$ARGUMENTS[3]").expect("$ARGUMENTS[N] matches");
        assert_eq!(caps.get(ENV_NAME_GROUP), None);
        assert_eq!(caps.get(BUILTIN_NAME_GROUP), None);
        assert_eq!(caps.get(DEFAULT_GROUP), None);
        assert_eq!(caps.get(ARG_INDEX_GROUP).map(|m| m.as_str()), Some("3"));
        assert_eq!(caps.get(POSITIONAL_GROUP), None);
        assert_eq!(caps.get(NAMED_GROUP), None);
    }

    #[test]
    fn positional_dollar_n_captures_group_5_only() {
        let re = combined_regex();
        let caps = re.captures("$5").expect("$N matches");
        assert_eq!(caps.get(ENV_NAME_GROUP), None);
        assert_eq!(caps.get(BUILTIN_NAME_GROUP), None);
        assert_eq!(caps.get(ARG_INDEX_GROUP), None);
        assert_eq!(caps.get(POSITIONAL_GROUP).map(|m| m.as_str()), Some("5"));
        assert_eq!(caps.get(NAMED_GROUP), None);
    }

    #[test]
    fn named_dollar_word_captures_group_6_only() {
        let re = combined_regex();
        let caps = re.captures("$foo").expect("$<name> matches");
        assert_eq!(caps.get(ENV_NAME_GROUP), None);
        assert_eq!(caps.get(BUILTIN_NAME_GROUP), None);
        assert_eq!(caps.get(ARG_INDEX_GROUP), None);
        assert_eq!(caps.get(POSITIONAL_GROUP), None);
        assert_eq!(caps.get(NAMED_GROUP).map(|m| m.as_str()), Some("foo"));
    }

    #[test]
    fn bare_arguments_has_no_capture_group_set() {
        // The bare-`$ARGUMENTS` alternative has no capture group; the
        // dispatcher distinguishes it from other Stage-3 forms by
        // checking `m.as_str() == "$ARGUMENTS"`.
        let re = combined_regex();
        let caps = re.captures("$ARGUMENTS").expect("$ARGUMENTS matches");
        assert_eq!(caps.get(0).map(|m| m.as_str()), Some("$ARGUMENTS"));
        assert_eq!(caps.get(ENV_NAME_GROUP), None);
        assert_eq!(caps.get(BUILTIN_NAME_GROUP), None);
        assert_eq!(caps.get(ARG_INDEX_GROUP), None);
        assert_eq!(caps.get(POSITIONAL_GROUP), None);
        assert_eq!(caps.get(NAMED_GROUP), None);
    }

    #[test]
    fn arguments_index_wins_over_bare_arguments() {
        // Critical leftmost-first ordering: `$ARGUMENTS[3]` must match
        // the indexed alternative (group 4 set), not bare-$ARGUMENTS
        // followed by literal `[3]`.
        let re = combined_regex();
        let caps = re.captures("$ARGUMENTS[3]").unwrap();
        assert_eq!(caps.get(0).map(|m| m.as_str()), Some("$ARGUMENTS[3]"));
        assert_eq!(caps.get(ARG_INDEX_GROUP).map(|m| m.as_str()), Some("3"));
    }

    #[test]
    fn named_lowercase_only_does_not_match_uppercase() {
        // `$PATH` MUST NOT match the named-$<name> alternative — keeps
        // Stage 3 from accidentally swallowing uppercase env-style
        // refs.
        let re = combined_regex();
        assert!(re.captures("$PATH").is_none());
        assert!(re.captures("$HOME").is_none());
        // Mixed-case starts-lowercase: pattern is greedy on
        // `[a-z0-9_]*`, so `$pathName` matches `$path` and stops at
        // `N`. Document this so the dispatcher's handling of trailing
        // text is unambiguous.
        let caps = re.captures("$pathName").unwrap();
        assert_eq!(caps.get(NAMED_GROUP).map(|m| m.as_str()), Some("path"));
        assert_eq!(caps.get(0).map(|m| m.as_str()), Some("$path"));
        // Pure-lowercase named ref is fine.
        assert!(re.captures("$path").is_some());
    }

    #[test]
    fn underscore_prefix_named_ref_matches() {
        // `[a-z_]` allows the identifier to start with `_`.
        let re = combined_regex();
        let caps = re.captures("$_foo").unwrap();
        assert_eq!(caps.get(NAMED_GROUP).map(|m| m.as_str()), Some("_foo"));
    }

    #[test]
    fn iter_returns_every_arg_match_in_order() {
        // Sanity-check the full unified pattern over a mixed body —
        // verifies the dispatcher loop will see each match in source
        // order and that resolved values can't accidentally collide.
        let re = combined_regex();
        let body = "${TOME_SKILL_NAME} $ARGUMENTS[0] $1 $foo bare $ARGUMENTS";
        let matches: Vec<&str> = re.find_iter(body).map(|m| m.as_str()).collect();
        assert_eq!(
            matches,
            vec![
                "${TOME_SKILL_NAME}",
                "$ARGUMENTS[0]",
                "$1",
                "$foo",
                "$ARGUMENTS",
            ]
        );
    }
}

//! Compiled-regex cache slots for the three substitution stages.
//!
//! Compiled lazily on first use via `OnceLock::get_or_init` (US2/US3).
//! F3 ships the slots uncompiled — the production pipeline still
//! returns the body unchanged at this stage.
//!
//! Filename note: this module is `regex_sets` rather than `regex` so it
//! doesn't shadow the [`regex`] crate inside the `substitution` module
//! tree, which would force every reference to the crate to use the
//! awkward `::regex::Regex` absolute path.

use regex::Regex;
use std::sync::OnceLock;

/// Compiled regex for the `${TOME_*}` built-ins stage. Populated by
/// [`builtin_regex`] on first call per US2.a.
pub(super) static BUILTINS_RE: OnceLock<Regex> = OnceLock::new();

/// Compiled regex for the `${TOME_ENV_*}` env-passthrough stage.
/// Populated by US2.b.
#[allow(dead_code)]
pub(super) static ENV_RE: OnceLock<Regex> = OnceLock::new();

/// Compiled regex for the `$ARGUMENTS` / `$N` / `$NAME` arguments
/// stage. Populated by US3.
#[allow(dead_code)]
pub(super) static ARGUMENTS_RE: OnceLock<Regex> = OnceLock::new();

/// Return the lazy-compiled regex for the Stage 1 built-ins pattern.
///
/// Pattern: `\$\{TOME_([A-Z0-9_]+)(?::-(.*?))?\}` per
/// `contracts/substitution-engine.md` § Stage 1. Capture group 1 is the
/// variable name (uppercase ASCII + digits + underscores); capture
/// group 2 is the optional `:-default` value.
///
/// The pattern is a constant — `Regex::new` cannot fail at runtime, so
/// the unreachable case is `expect`ed with a clear message rather than
/// propagated as `Result`.
pub(super) fn builtin_regex() -> &'static Regex {
    BUILTINS_RE.get_or_init(|| {
        Regex::new(r"\$\{TOME_([A-Z0-9_]+)(?::-(.*?))?\}")
            .expect("BUILTIN_REGEX must compile (constant pattern)")
    })
}

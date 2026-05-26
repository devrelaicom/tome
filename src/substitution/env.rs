//! Env-passthrough `${TOME_ENV_*}` substitution.
//!
//! Phase 5 / US2.b — Stage 2 of the substitution pipeline. Implements
//! the host-env passthrough per `contracts/substitution-engine.md`
//! § Stage 2 and FR-030–FR-033.
//!
//! Lookup key construction: the regex captures the suffix after
//! `TOME_ENV_`; the full host-env lookup key is reconstructed by
//! [`std::env::var`] as `format!("TOME_ENV_{name}")` so the namespace
//! prefix is preserved on the host side. This means a plugin author
//! writes `${TOME_ENV_GITHUB_TOKEN}` and operators export
//! `TOME_ENV_GITHUB_TOKEN=…` — no untyped `${GITHUB_TOKEN}` references
//! are ever matched (FR-033 + NFR-005).
//!
//! Default-value behaviour matrix (contract § Stage 2):
//!
//! | Host env state | Reference form | Resolved value |
//! |---|---|---|
//! | Set | `${TOME_ENV_FOO}` | Host value |
//! | Set | `${TOME_ENV_FOO:-default}` | Host value (default ignored) |
//! | Unset | `${TOME_ENV_FOO}` | Empty string + `tracing::debug!` |
//! | Unset | `${TOME_ENV_FOO:-default}` | `default` |
//!
//! Fast-path: bodies with no matches short-circuit to `Cow::Borrowed`
//! so the caller can pass the slice straight into Stage 3 without a
//! copy.

use std::borrow::Cow;

use super::regex_sets;

/// Apply Stage 2 env-passthrough substitution.
///
/// Performs a single sweep over `body` rewriting every
/// `${TOME_ENV_<NAME>}` (with optional `:-default`) reference per the
/// matrix above. Returns `Cow::Borrowed(body)` when no matches are
/// found (NFR-007: substituted values are not re-scanned anyway, but
/// the fast-path avoids allocating an owned string when the body is
/// entirely free of Stage 2 references).
pub(super) fn apply_env(body: &str) -> Cow<'_, str> {
    let re = regex_sets::env_regex();
    if !re.is_match(body) {
        return Cow::Borrowed(body);
    }
    re.replace_all(body, |caps: &regex::Captures<'_>| {
        // capture 1 is `[A-Z0-9_]+` per the constant pattern; it is
        // always present on a successful match. The unwrap-with-empty
        // is purely defensive against a future regex change.
        let name = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        let default = caps.get(2).map(|m| m.as_str());
        let key = format!("TOME_ENV_{name}");
        match std::env::var(&key) {
            Ok(value) => value,
            Err(_) => match default {
                Some(d) => d.to_string(),
                None => {
                    tracing::debug!(
                        name = name,
                        key = key.as_str(),
                        "TOME_ENV_ reference with no host value and no default; resolving to empty string"
                    );
                    String::new()
                }
            },
        }
    })
    // `replace_all` returns `Cow<'_, str>` whose lifetime is tied to
    // `body`; propagate verbatim so the borrowed-fast-path inside the
    // regex engine is preserved when every replacement happens to be a
    // direct identity (not the common case, but cheap to keep).
}

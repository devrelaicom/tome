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
//! ## Single-sweep design (US2.d B2 fix)
//!
//! Per-match resolution is invoked from the unified Stage 1 + Stage 2
//! sweep in [`super::render`]. The resolved value is emitted directly
//! into the output buffer and never re-enters the scanner — this is the
//! structural enforcement of the no-rescan invariant (NFR-007 /
//! FR-051) and the fix for the exfiltration vector documented in US2.d
//! B2 (a hostile plugin's `"version": "${TOME_ENV_GITHUB_TOKEN}"`
//! cannot leak the operator's env var).

/// Resolve one `${TOME_ENV_<NAME>}` reference to its host-env value.
///
/// Pure function: caller supplies the captured `name` (suffix after
/// `TOME_ENV_`) and the optional `:-default` value; returns the
/// resolved string per the behaviour matrix in the module docs. Never
/// fails — when both the host env is unset and no default is supplied,
/// resolves to the empty string with a `tracing::debug!` event.
///
/// The returned `String` is emitted verbatim into the output buffer by
/// the caller and is NOT re-scanned (NFR-007).
pub(super) fn resolve_env(name: &str, default: Option<&str>) -> String {
    let key = format!("TOME_ENV_{name}");
    match std::env::var(&key) {
        Ok(value) => value,
        Err(_) => match default {
            Some(d) => d.to_owned(),
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
}

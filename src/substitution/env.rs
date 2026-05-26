//! Env-passthrough `{{$VAR}}` substitution.
//!
//! F3 stub: returns body borrowed (no-op). US2 wires the production
//! pattern per `contracts/substitution-engine.md` §Env passthrough.

use std::borrow::Cow;

/// Apply env-passthrough `{{$VAR}}` substitution.
///
/// F3 stub: returns body borrowed (no-op). The `#[allow(dead_code)]`
/// lifts in US2 when `render()` wires the stage.
#[allow(dead_code)]
pub(super) fn apply_env(body: &str) -> Cow<'_, str> {
    Cow::Borrowed(body)
}

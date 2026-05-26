//! Argument substitution per Claude Code semantics.
//!
//! Handles `$ARGUMENTS`, `$1` / `$2` / …, and `$NAME` references in
//! command/skill bodies. F3 stub: returns body unchanged + `false`
//! (nothing replaced). US3 wires the production pattern per
//! `contracts/substitution-engine.md` §Arguments.

use super::ArgumentValues;

/// Apply argument substitution per Claude Code semantics.
///
/// Returns `(rendered_body, any_replaced)`. The boolean signals to the
/// outer pipeline whether the optional `ARGUMENTS: …` tail should be
/// appended (only when caller-supplied args went unconsumed by the body).
///
/// F3 stub: returns body unchanged and `false`. The
/// `#[allow(dead_code)]` lifts in US3 when `render()` wires the stage.
#[allow(dead_code)]
pub(super) fn apply_arguments(
    body: &str,
    _args: &ArgumentValues,
    _declared: &[String],
) -> (String, bool) {
    (body.to_string(), false)
}

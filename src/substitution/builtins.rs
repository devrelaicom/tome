//! Built-in `{{TOME_*}}` placeholder substitution.
//!
//! F3 stub: returns body unchanged. US2 wires the production pattern
//! per `contracts/substitution-engine.md` §Built-ins.

use super::{SubstitutionContext, SubstitutionError};

/// Apply built-in `{{TOME_*}}` placeholder substitution.
///
/// F3 stub: returns body unchanged. The `#[allow(dead_code)]` lifts in
/// US2 when `render()` wires the stage.
#[allow(dead_code)]
pub(super) fn apply_builtins(
    body: &str,
    _ctx: &SubstitutionContext,
) -> Result<String, SubstitutionError> {
    Ok(body.to_string())
}

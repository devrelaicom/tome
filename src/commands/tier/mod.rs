//! `tome tier {set,list,clear}` — per-workspace skill/command routing tiers.
//!
//! Tiers live on the `workspace_skills.tier` column (schema v5). `set` and
//! `clear` perform an UPDATE under the advisory `index.lock` (FR-040); `list`
//! is a read-only projection. After a successful `set`/`clear`, the workspace's
//! `RULES.md` (and every bound project's mirror) is regenerated so the routing
//! directive reflects the new tier immediately.

use crate::cli::TierCommand;
use crate::error::TomeError;
use crate::output::Mode;
use crate::workspace::ResolvedScope;

mod clear;
mod list;
pub(crate) mod set;

pub fn run(cmd: TierCommand, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    match cmd {
        TierCommand::Set(args) => set::run(args, scope, mode),
        TierCommand::List(args) => list::run(args, scope, mode),
        TierCommand::Clear(args) => clear::run(args, scope, mode),
    }
}

/// Shared `<plugin>/<name>` parse.
pub(crate) fn split_id(id: &str) -> Result<(&str, &str), TomeError> {
    match id.split_once('/') {
        Some((p, n)) if !p.is_empty() && !n.is_empty() => Ok((p, n)),
        _ => Err(TomeError::Usage(format!(
            "invalid entry id `{id}` (expected `<plugin>/<name>`)"
        ))),
    }
}

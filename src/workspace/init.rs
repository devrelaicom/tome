//! `tome workspace init` — neutered for Phase 4 / Slice F2a.
//!
//! Phase 3's atomic `.tome/` creation is obsolete: Phase 4 splits the
//! Phase 3 marker into two artefacts:
//!
//! - `<root>/workspaces/<name>/{settings.toml, RULES.md}` — the
//!   workspace-layer state, owned by `tome workspace add` (US2).
//! - `<project>/.tome/config.toml` — a thin binding pointer carrying
//!   `workspace = "<name>"`, owned by `tome workspace use` (US1).
//!
//! The legacy `tome workspace init` command name is retained as a
//! `#[ignore]` placeholder so the harness compiles; the real
//! replacement commands land in slices US1 and US2. F2a delivers only
//! the path reshape — the lifecycle rewrite follows.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::TomeError;
use crate::paths::Paths;

/// Pre-Phase-4 outcome record. Retained as a type so the CLI command
/// dispatcher in `src/commands/workspace/` continues to compile until
/// US1/US2 rewrite the command surface.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct InitOutcome {
    pub workspace: PathBuf,
    pub catalogs: u32,
    pub inherited: bool,
    pub config_path: PathBuf,
    pub index_bootstrapped: bool,
}

/// Returns a clear error pointing at the F11/US1 rewrite. The original
/// implementation depended on per-workspace `.tome/index.db` and
/// `.tome/config.toml` paths that no longer exist in the Phase 4
/// layout. Callers that need the new behaviour will live under
/// `tome workspace add` / `tome workspace use` (US2 / US1).
pub fn init(
    _target_root: &Path,
    _inherit_global: bool,
    _force: bool,
    _paths: &Paths,
) -> Result<InitOutcome, TomeError> {
    Err(TomeError::Internal(anyhow::anyhow!(
        "`tome workspace init` is replaced in Phase 4 by `tome workspace add` (US2) + `tome workspace use` (US1); see slices F2a → F10 → US1 → US2"
    )))
}

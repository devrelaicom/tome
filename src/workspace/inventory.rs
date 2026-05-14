//! Opt-in best-effort workspace inventory.
//!
//! `${state_dir}/workspaces.txt` is a newline-delimited list of absolute
//! workspace root paths that `tome workspace init` has run against
//! (under one of the historical bootstrap modes). Used by the catalog
//! reference-counting algorithm (US3 / contracts/catalog-extensions-p3.md)
//! to enumerate every scope that might reference a given catalog URL
//! before deleting an on-disk clone.
//!
//! The registry is intentionally opt-in and intentionally best-effort:
//!
//! - Missing file → empty list. This is the steady-state condition for
//!   users who haven't created a workspace yet.
//! - Malformed line (not an absolute path, missing dir) → ignored. The
//!   inventory is a *hint*, not a source of truth. Phase 3 doctor
//!   reports orphan clones; the inventory just shortens the search.
//! - Concurrent writers — none yet. Phase 3 only writes via
//!   `tome workspace init`, and the file is rewritten atomically (slice
//!   F3 contract).
//!
//! Phase 3 introduces only the reader. The writer (`append_if_registry_exists`)
//! lands with `tome workspace init` (US2 / slice US2.b).

use std::path::{Path, PathBuf};

/// Read the inventory at `path` (typically `paths.workspace_registry`).
///
/// Returns an empty `Vec` when the file is missing or unreadable — by
/// design (see module-level docs).
pub fn read_registry(path: &Path) -> Vec<PathBuf> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .collect()
}

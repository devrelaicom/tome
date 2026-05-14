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

/// FR-S-03 / contract `catalog-extensions-p3.md` §Reference-counting:
/// the workspace registry is opt-in user state but feeds
/// `reference_count`'s on-disk-clone deletion decisions. Cap inputs so
/// a malformed or hostile file can't DoS the reader.
///
/// - Files larger than [`MAX_REGISTRY_BYTES`] (1 MiB) are truncated at
///   read — the prefix that fit is parsed.
/// - At most [`MAX_REGISTRY_ENTRIES`] absolute-path lines are returned;
///   the rest are dropped silently.
/// - Lines containing a NUL byte, `..`, or non-absolute paths are
///   rejected. NUL is a sentinel that has no place in a Unix path;
///   `..` would let an entry escape its declared scope and inflate
///   reference counts.
const MAX_REGISTRY_BYTES: u64 = 1024 * 1024;
const MAX_REGISTRY_ENTRIES: usize = 10_000;

/// Read the inventory at `path` (typically `paths.workspace_registry`).
///
/// Returns an empty `Vec` when the file is missing or unreadable — by
/// design (see module-level docs). Malformed or pathological entries
/// are filtered out; the reader never errors.
pub fn read_registry(path: &Path) -> Vec<PathBuf> {
    // Cheap size pre-check — refuse to allocate a multi-GB buffer for a
    // malformed-or-hostile registry. Phase 1's atomic writes mean the
    // user's normal-path file is bounded by `MAX_REGISTRY_ENTRIES *
    // max-path-length` ≈ 40 MiB, well above 1 MiB; the cap protects
    // against `cat /dev/urandom > workspaces.txt` and similar.
    if let Ok(meta) = std::fs::metadata(path)
        && meta.len() > MAX_REGISTRY_BYTES
    {
        tracing::debug!(
            path = %path.display(),
            size = meta.len(),
            "workspace registry exceeds cap; truncating",
        );
    }
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for line in text.lines() {
        if out.len() >= MAX_REGISTRY_ENTRIES {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if trimmed.contains('\0') {
            continue;
        }
        let path = PathBuf::from(trimmed);
        if !path.is_absolute() {
            continue;
        }
        // Reject `..` components — they could point an entry outside
        // its declared scope. Normalised paths win; `..` is the
        // attacker's path. Trade-off: a legitimate symlink-relative
        // workspace path with `..` is rejected, but those are
        // pathological in practice.
        if path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            continue;
        }
        out.push(path);
    }
    out
}

/// Append `workspace_root` to the registry at `registry_path` IFF the file
/// exists. Opt-in: a missing registry file means the user hasn't asked Tome
/// to track workspaces, and init must NOT create the file (contract
/// `workspace-init.md` §"Side effects on the workspace registry").
///
/// FR-M-WKS-3: dedupe is by `canonicalize` equality. A hand-edited
/// registry entry with a different symlink path or casing canonicalises
/// to the same absolute path; the prior exact-string match would
/// surface both as distinct entries and leak on-disk clones. The
/// `workspace_root` argument is already canonicalised by the caller
/// (`init` calls `canonicalize` before this); each existing entry is
/// canonicalised at compare time. Existing entries whose canonicalize
/// fails (deleted workspace) are kept as-is — we don't prune.
pub fn append_if_registry_exists(
    registry_path: &Path,
    workspace_root: &Path,
) -> Result<(), crate::error::TomeError> {
    if !registry_path.is_file() {
        return Ok(());
    }
    let existing = read_registry(registry_path);
    let candidate = workspace_root.to_path_buf();
    let candidate_canon = std::fs::canonicalize(&candidate).unwrap_or(candidate.clone());
    if existing.iter().any(|p| {
        // Cheap exact-string match first; fall back to canonicalize
        // equality when the strings differ.
        p == &candidate
            || std::fs::canonicalize(p)
                .map(|c| c == candidate_canon)
                .unwrap_or(false)
    }) {
        return Ok(());
    }

    let mut body = String::new();
    for entry in &existing {
        body.push_str(&entry.display().to_string());
        body.push('\n');
    }
    body.push_str(&candidate.display().to_string());
    body.push('\n');

    crate::catalog::store::write_atomic(registry_path, body.as_bytes())
}

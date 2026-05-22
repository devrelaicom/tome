//! Per-subsystem check functions used by `tome doctor`'s assembly path.
//! Each function is pure compute over `(paths, scope, …)`; they share
//! the read-only-DB convention with `tome status` (`PRAGMA
//! integrity_check`, no advisory lock).
//!
//! Models / index / drift are delegated to `commands::status`'s
//! already-`pub` helpers so the two surfaces stay consistent — doctor's
//! checks must report the same health values status would for the
//! overlapping subsystems.
//!
//! New checks live here:
//! - `check_catalogs` enumerates the resolved scope's catalogs and
//!   classifies each on-disk clone.
//! - `harness_detect::probe` (sibling module) handles the harness list.

use std::path::Path;

use crate::catalog::manifest::CatalogManifest;
use crate::commands::plugin::registry_seeds;
use crate::doctor::report::{CatalogCacheHealth, CatalogCacheState};
use crate::error::TomeError;
use crate::index::{self, OpenOptions, workspace_catalogs};
use crate::paths::Paths;
use crate::workspace::Scope;

/// Enumerate every catalog in the resolved scope's enrolments (via
/// `workspace_catalogs`) and classify the on-disk clone:
///
/// - Missing → cache directory not on disk.
/// - NotARepo → directory exists but lacks `.git/`.
/// - ManifestInvalid → directory + `.git/` present but `tome-catalog.toml`
///   is missing or unparsable.
/// - Ok → everything parses.
///
/// `tome catalog show <name>` is the corresponding read-only inspect
/// surface; doctor's check is intentionally lighter (existence + parse
/// only, no validation of plugin sources).
///
/// Returns an empty `Vec` when the central DB is absent or the scope's
/// workspace has no enrolments.
pub fn check_catalogs(paths: &Paths, scope: &Scope) -> Result<Vec<CatalogCacheHealth>, TomeError> {
    let workspace_name = scope.name().as_str();

    let enrolments = if paths.index_db.is_file() {
        let (embedder_seed, reranker_seed, summariser_seed) = registry_seeds();
        let conn = index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: embedder_seed,
                reranker: reranker_seed,
                summariser: summariser_seed,
            },
        )?;
        workspace_catalogs::list_for_workspace(&conn, workspace_name).unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut out = Vec::with_capacity(enrolments.len());

    // Step 1: classify every catalog the resolved workspace enrols.
    let mut referenced_paths: std::collections::HashSet<std::path::PathBuf> =
        std::collections::HashSet::new();
    for e in &enrolments {
        let cache_path = paths.cache_dir_for(&e.url);
        referenced_paths.insert(cache_path.clone());
        let state = classify_clone(&cache_path);
        out.push(CatalogCacheHealth {
            name: e.catalog_name.clone(),
            url: e.url.clone(),
            cache_path,
            state,
        });
    }

    // Step 2: enumerate on-disk clones at `paths.catalogs_dir` and
    // surface any directory NOT referenced by the resolved config as
    // `Orphan`. Per `catalog-extensions-p3.md` §"Doctor reporting"
    // bullet 4: cache exists but no config references it. The URL is
    // unknown at the doctor level (we'd need to parse the manifest to
    // recover the original source URL); leaving it empty keeps the
    // JSON wire shape simple — the user only needs the cache path
    // to act on it.
    if paths.catalogs_dir.is_dir() {
        let entries = match std::fs::read_dir(&paths.catalogs_dir) {
            Ok(it) => it,
            Err(_) => return Ok(out),
        };
        for de in entries.flatten() {
            let p = de.path();
            if !p.is_dir() {
                continue;
            }
            if referenced_paths.contains(&p) {
                continue;
            }
            // Only orphans we can confidently classify (a directory
            // with `.git/` + parsable manifest is a real abandoned
            // catalog clone). A half-broken directory shows up as
            // `Missing` / `NotARepo` / `ManifestInvalid` on the
            // referenced-catalog path; unreferenced half-broken dirs
            // are unactionable noise and we skip them.
            let manifest = p.join("tome-catalog.toml");
            if !p.join(".git").is_dir() || !manifest.is_file() {
                continue;
            }
            // Unknown URL (we don't re-parse just to recover the
            // source); the user has the path which is what they need
            // to remove it.
            out.push(CatalogCacheHealth {
                name: p
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "<unknown>".to_owned()),
                url: String::new(),
                cache_path: p,
                state: crate::doctor::report::CatalogCacheState::Orphan,
            });
        }
    }

    Ok(out)
}

/// Phase 4 / F2a: the Phase 3 opt-in `workspaces.txt` registry is gone.
/// Workspace bindings now live in the central database's
/// `workspace_projects` table (F11). The function is retained as a
/// `present: false, tracked: 0` stub so the doctor JSON envelope shape
/// stays unchanged until F11 promotes a richer per-binding report.
pub fn check_workspace_registry(_paths: &Paths) -> crate::doctor::report::WorkspaceRegistryStatus {
    crate::doctor::report::WorkspaceRegistryStatus {
        present: false,
        tracked: 0,
    }
}

/// Classify a single clone path. Pure FS reads — no network, no git
/// shell-out.
fn classify_clone(path: &Path) -> CatalogCacheState {
    if !path.exists() {
        return CatalogCacheState::Missing;
    }
    if !path.is_dir() {
        // A file at the cache path is degenerate but not impossible
        // (manual filesystem editing). Treat as Missing — the rebuild
        // path is the same.
        return CatalogCacheState::Missing;
    }
    let git_dir = path.join(".git");
    if !git_dir.exists() {
        return CatalogCacheState::NotARepo;
    }
    let manifest_path = path.join("tome-catalog.toml");
    let Ok(bytes) = std::fs::read(&manifest_path) else {
        return CatalogCacheState::ManifestInvalid;
    };
    // We deliberately use lenient parsing — doctor only reports whether
    // the manifest is readable, not whether every plugin entry is
    // resolvable. `tome catalog show` is the surface for the deeper
    // validation.
    if CatalogManifest::parse_and_validate(&manifest_path, path, &bytes).is_err() {
        return CatalogCacheState::ManifestInvalid;
    }
    CatalogCacheState::Ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::workspace_catalogs;
    use crate::workspace::Scope;
    use tempfile::TempDir;

    fn fixture_paths(tmp: &Path) -> Paths {
        Paths::from_root(tmp.to_path_buf())
    }

    /// Seed one enrolment into the central DB for the privileged
    /// `global` workspace. Bootstrap happens via `index::open`; the
    /// per-test cache_path is the URL-hashed dir under `catalogs/` by
    /// construction of `paths.cache_dir_for(url)`.
    fn seed_enrolment(paths: &Paths, catalog: &str, url: &str) {
        let (e, r, s) = registry_seeds();
        let conn = index::open(
            &paths.index_db,
            &OpenOptions {
                embedder: e,
                reranker: r,
                summariser: s,
            },
        )
        .unwrap();
        workspace_catalogs::insert(&conn, "global", catalog, url, "main").unwrap();
    }

    #[test]
    fn check_catalogs_returns_empty_when_no_config() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let out =
            check_catalogs(&paths, &Scope(crate::workspace::WorkspaceName::global())).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn check_catalogs_reports_missing_for_absent_clone() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let url = "https://example.invalid/missing";
        seed_enrolment(&paths, "lost", url);

        let out =
            check_catalogs(&paths, &Scope(crate::workspace::WorkspaceName::global())).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].state, CatalogCacheState::Missing);
        assert_eq!(out[0].name, "lost");
    }

    #[test]
    fn check_catalogs_reports_not_a_repo_for_dir_without_git() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let url = "https://example.invalid/nogit";
        seed_enrolment(&paths, "nogit", url);
        std::fs::create_dir_all(paths.cache_dir_for(url)).unwrap();

        let out =
            check_catalogs(&paths, &Scope(crate::workspace::WorkspaceName::global())).unwrap();
        assert_eq!(out[0].state, CatalogCacheState::NotARepo);
    }

    #[test]
    fn check_catalogs_reports_manifest_invalid_when_manifest_missing() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let url = "https://example.invalid/nomanifest";
        seed_enrolment(&paths, "nomanifest", url);
        std::fs::create_dir_all(paths.cache_dir_for(url).join(".git")).unwrap();

        let out =
            check_catalogs(&paths, &Scope(crate::workspace::WorkspaceName::global())).unwrap();
        assert_eq!(out[0].state, CatalogCacheState::ManifestInvalid);
    }
}

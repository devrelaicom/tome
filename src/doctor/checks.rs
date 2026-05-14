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
use crate::catalog::store as catalog_store;
use crate::doctor::report::{CatalogCacheHealth, CatalogCacheState};
use crate::error::TomeError;
use crate::paths::Paths;
use crate::workspace::Scope;

/// Enumerate every catalog in the resolved scope's config and classify
/// the on-disk clone:
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
/// Returns an empty `Vec` when the config doesn't exist or has no
/// catalogs.
pub fn check_catalogs(paths: &Paths, scope: &Scope) -> Result<Vec<CatalogCacheHealth>, TomeError> {
    let config_path = paths.config_file_for(scope);
    if !config_path.is_file() {
        return Ok(Vec::new());
    }
    let config = catalog_store::load(&config_path)?;

    let mut out = Vec::with_capacity(config.catalogs.len());
    for entry in config.catalogs.values() {
        let cache_path = entry.path.clone();
        let state = classify_clone(&cache_path);
        out.push(CatalogCacheHealth {
            name: entry.name.clone(),
            url: entry.url.clone(),
            cache_path,
            state,
        });
    }
    Ok(out)
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
    use crate::config::{CatalogEntry, Config};
    use crate::workspace::Scope;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use tempfile::TempDir;
    use time::OffsetDateTime;

    fn fixture_paths(tmp: &Path) -> Paths {
        let state = tmp.join("state");
        Paths {
            config_dir: tmp.join("config"),
            config_file: tmp.join("config/config.toml"),
            data_dir: tmp.join("data"),
            catalogs_dir: tmp.join("data/catalogs"),
            index_db: tmp.join("data/index.db"),
            index_lock: tmp.join("data/index.lock"),
            models_dir: tmp.join("data/models"),
            state_dir: state.clone(),
            mcp_log: state.join("mcp.log"),
            mcp_log_prev: state.join("mcp.log.1"),
            workspace_registry: state.join("workspaces.txt"),
        }
    }

    fn write_config_with_one_catalog(paths: &Paths, name: &str, cache_path: PathBuf) {
        let mut catalogs = BTreeMap::new();
        catalogs.insert(
            name.to_owned(),
            CatalogEntry {
                name: name.to_owned(),
                url: format!("file://{}", cache_path.display()),
                ref_: "main".into(),
                path: cache_path,
                last_synced: OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
            },
        );
        let cfg = Config { catalogs };
        std::fs::create_dir_all(&paths.config_dir).unwrap();
        std::fs::write(&paths.config_file, toml::to_string_pretty(&cfg).unwrap()).unwrap();
    }

    #[test]
    fn check_catalogs_returns_empty_when_no_config() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let out = check_catalogs(&paths, &Scope::Global).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn check_catalogs_reports_missing_for_absent_clone() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let missing = paths.catalogs_dir.join("does-not-exist");
        write_config_with_one_catalog(&paths, "lost", missing);

        let out = check_catalogs(&paths, &Scope::Global).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].state, CatalogCacheState::Missing);
        assert_eq!(out[0].name, "lost");
    }

    #[test]
    fn check_catalogs_reports_not_a_repo_for_dir_without_git() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let cache = paths.catalogs_dir.join("nogit");
        std::fs::create_dir_all(&cache).unwrap();
        write_config_with_one_catalog(&paths, "nogit", cache);

        let out = check_catalogs(&paths, &Scope::Global).unwrap();
        assert_eq!(out[0].state, CatalogCacheState::NotARepo);
    }

    #[test]
    fn check_catalogs_reports_manifest_invalid_when_manifest_missing() {
        let tmp = TempDir::new().unwrap();
        let paths = fixture_paths(tmp.path());
        let cache = paths.catalogs_dir.join("nomanifest");
        std::fs::create_dir_all(cache.join(".git")).unwrap();
        write_config_with_one_catalog(&paths, "nomanifest", cache);

        let out = check_catalogs(&paths, &Scope::Global).unwrap();
        assert_eq!(out[0].state, CatalogCacheState::ManifestInvalid);
    }
}

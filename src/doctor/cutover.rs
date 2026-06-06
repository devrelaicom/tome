//! Phase 8 cutover doctor surface: detect + repair pre-cutover artifacts.
//!
//! Two read-only checks plus one repair, kept self-contained so the
//! intricate `report`/`fixes` machinery stays untouched:
//!
//! - **Legacy model manifests** ([`legacy_model_manifests`]): a model dir with
//!   a pre-cutover `manifest.json` and no native `manifest.toml`. Reported
//!   read-only; migrated by `doctor --fix` ([`migrate_model_manifests`]) —
//!   read the JSON, write the TOML atomically, delete the JSON. **No
//!   re-download** (the bytes are identical). This is the only cutover repair
//!   `--fix` performs (manifest-cutover.md §doctor).
//! - **Unconverted plugins** ([`unconverted_plugins`]): an enrolled catalog's
//!   plugin dir carrying a legacy `.claude-plugin/plugin.json` but no
//!   `tome-plugin.toml`. Reported read-only; **never auto-fixed** — converting
//!   a source repo is an authoring action the user must run (`tome plugin
//!   convert`), not a doctor repair (FR-124 discipline).

use std::path::{Path, PathBuf};

use crate::catalog::manifest::read_catalog_manifest;
use crate::embedding::registry::{MODEL_REGISTRY, ModelManifest};
use crate::error::TomeError;
use crate::paths::Paths;
use crate::plugin::manifest::{manifest_path_for, tome_manifest_path_for};

/// Registry model names whose on-disk manifest is still the pre-cutover
/// `manifest.json` (native `manifest.toml` absent). Read-only.
pub fn legacy_model_manifests(paths: &Paths) -> Vec<String> {
    MODEL_REGISTRY
        .iter()
        .filter(|e| {
            let toml_present = paths
                .model_manifest(e.name)
                .map(|p| p.is_file())
                .unwrap_or(false);
            let json_present = paths
                .model_manifest_legacy(e.name)
                .map(|p| p.is_file())
                .unwrap_or(false);
            json_present && !toml_present
        })
        .map(|e| e.name.to_owned())
        .collect()
}

/// Migrate every legacy model `manifest.json` to the native `manifest.toml`:
/// read → write TOML (atomic temp-file replace) → delete the JSON. No
/// re-download. Returns the migrated model names. Idempotent: a model that
/// already has `manifest.toml` is skipped.
pub fn migrate_model_manifests(paths: &Paths) -> Result<Vec<String>, TomeError> {
    let mut migrated = Vec::new();
    for entry in MODEL_REGISTRY {
        let toml_path = paths.model_manifest(entry.name)?;
        let json_path = paths.model_manifest_legacy(entry.name)?;
        if toml_path.is_file() || !json_path.is_file() {
            continue;
        }
        // Parse the legacy JSON (ModelManifest is Deserialize for both forms).
        let bytes = std::fs::read(&json_path).map_err(TomeError::Io)?;
        let manifest = serde_json::from_slice::<ModelManifest>(&bytes).map_err(|e| {
            TomeError::ModelRegistrationParseError {
                file: json_path.clone(),
                message: e.to_string(),
            }
        })?;
        // Write the native TOML atomically, then delete the JSON.
        let body = manifest.to_toml(&toml_path)?;
        let dir = toml_path.parent().ok_or_else(|| {
            TomeError::Internal(anyhow::anyhow!(
                "model manifest path {} has no parent",
                toml_path.display()
            ))
        })?;
        let tmp = tempfile::NamedTempFile::new_in(dir).map_err(TomeError::Io)?;
        std::fs::write(tmp.path(), &body).map_err(TomeError::Io)?;
        tmp.as_file().sync_all().map_err(TomeError::Io)?;
        tmp.persist(&toml_path)
            .map_err(|e| TomeError::Io(e.error))?;
        std::fs::remove_file(&json_path).map_err(TomeError::Io)?;
        migrated.push(entry.name.to_owned());
    }
    Ok(migrated)
}

/// Plugin directories under the given catalog cache paths that are still
/// unconverted: a legacy `.claude-plugin/plugin.json` present, no
/// `tome-plugin.toml`. Returns the plugin directories (display paths). The
/// caller supplies the enrolled catalog cache roots (the doctor already
/// resolves these for its catalog-cache checks). Read-only — no auto-fix.
pub fn unconverted_plugins(catalog_cache_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for cache_root in catalog_cache_roots {
        // Prefer the catalog manifest's declared plugin sources; fall back to
        // a shallow scan of immediate subdirectories for a catalog whose
        // manifest is itself unreadable.
        let plugin_dirs = catalog_plugin_dirs(cache_root);
        for plugin_dir in plugin_dirs {
            if is_unconverted(&plugin_dir) {
                out.push(plugin_dir);
            }
        }
    }
    out.sort();
    out
}

/// True iff `plugin_dir` carries a legacy `.claude-plugin/plugin.json` but no
/// native `tome-plugin.toml`.
fn is_unconverted(plugin_dir: &Path) -> bool {
    manifest_path_for(plugin_dir).is_file() && !tome_manifest_path_for(plugin_dir).is_file()
}

/// Resolve a catalog cache root to its plugin directories — via the catalog
/// manifest's `plugins[].source` when readable, else a shallow subdir scan.
fn catalog_plugin_dirs(cache_root: &Path) -> Vec<PathBuf> {
    if let Some(manifest) = read_catalog_manifest(cache_root) {
        return manifest
            .plugins
            .iter()
            .map(|p| cache_root.join(&p.source))
            .collect();
    }
    // Fallback: immediate subdirectories (a catalog whose manifest is absent
    // or unreadable still surfaces its unconverted plugins honestly).
    let Ok(entries) = std::fs::read_dir(cache_root) else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::registry::lookup;

    fn write_legacy_model_json(paths: &Paths, name: &str) {
        let entry = lookup(name).expect("registry entry");
        let dir = paths.models_dir.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = ModelManifest {
            name: entry.name.to_owned(),
            version: entry.version.to_owned(),
            kind: entry.kind,
            source_url: entry.source_url.to_owned(),
            sha256: entry.sha256.to_owned(),
            size_bytes: entry.size_bytes,
            licence: entry.licence.to_owned(),
            files: entry.files.iter().map(|s| (*s).to_owned()).collect(),
            installed_at: time::OffsetDateTime::UNIX_EPOCH,
        };
        let body = serde_json::to_vec_pretty(&manifest).unwrap();
        std::fs::write(dir.join("manifest.json"), body).unwrap();
    }

    fn test_paths(root: &Path) -> Paths {
        // Minimal Paths rooted at `root`; only models_dir + model_manifest are
        // exercised here.
        Paths::from_root(root.to_path_buf())
    }

    #[test]
    fn detects_and_migrates_legacy_model_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = test_paths(tmp.path());
        let name = MODEL_REGISTRY[0].name;
        write_legacy_model_json(&paths, name);

        // Read-only detection.
        assert!(legacy_model_manifests(&paths).contains(&name.to_owned()));

        // Migrate.
        let migrated = migrate_model_manifests(&paths).expect("migrate");
        assert!(migrated.contains(&name.to_owned()));

        // Post-state: toml present, json gone, and it re-parses.
        let toml_path = paths.model_manifest(name).unwrap();
        let json_path = paths.model_manifest_legacy(name).unwrap();
        assert!(toml_path.is_file(), "manifest.toml must exist");
        assert!(!json_path.is_file(), "manifest.json must be deleted");
        let bytes = std::fs::read(&toml_path).unwrap();
        let m = ModelManifest::from_toml_slice(&toml_path, &bytes).expect("re-parse toml");
        assert_eq!(m.name, name);

        // Idempotent: a second migrate finds nothing.
        assert!(legacy_model_manifests(&paths).is_empty());
        assert!(migrate_model_manifests(&paths).unwrap().is_empty());
    }

    #[test]
    fn unconverted_plugin_detection() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cat");
        // A legacy plugin (plugin.json only) and a converted one (both files).
        let legacy = cache.join("legacy-plugin");
        std::fs::create_dir_all(legacy.join(".claude-plugin")).unwrap();
        std::fs::write(
            legacy.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"legacy-plugin","version":"1.0.0"}"#,
        )
        .unwrap();
        let converted = cache.join("converted-plugin");
        std::fs::create_dir_all(converted.join(".claude-plugin")).unwrap();
        std::fs::write(
            converted.join(".claude-plugin").join("plugin.json"),
            r#"{"name":"converted-plugin","version":"1.0.0"}"#,
        )
        .unwrap();
        std::fs::write(
            converted.join("tome-plugin.toml"),
            "name = \"converted-plugin\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let found = unconverted_plugins(std::slice::from_ref(&cache));
        assert!(found.contains(&legacy), "legacy plugin must be flagged");
        assert!(
            !found.contains(&converted),
            "converted plugin must not be flagged"
        );
    }
}

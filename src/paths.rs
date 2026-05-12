//! XDG-aware path resolution. We honour `XDG_CONFIG_HOME` and `XDG_DATA_HOME`
//! across both macOS and Linux (rather than `directories`'s macOS-specific
//! `~/Library/...` fallback) — the spec is explicit about the XDG layout and
//! tests rely on it being controllable via the env vars. `cache_dir_for(url)`
//! content-addresses each catalog's cache by sha256(url) (FR-015).
//!
//! Phase 2 adds the index database, an advisory write lock, and the models
//! directory; all live under `data_dir` per spec FR-021 (data dir, NOT cache
//! dir — OS cache cleaners must never sweep them away). FR-021 explicitly
//! requires reusing the Phase 1 resolver rather than introducing a parallel
//! path-resolution rule.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::error::TomeError;

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
    pub catalogs_dir: PathBuf,
    // Phase 2 — all under `data_dir`, not `cache_dir` (FR-021).
    pub index_db: PathBuf,
    pub index_lock: PathBuf,
    pub models_dir: PathBuf,
}

impl Paths {
    pub fn resolve() -> Result<Self, TomeError> {
        let home = std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            TomeError::Io(std::io::Error::other(
                "HOME is not set — cannot resolve config and data directories",
            ))
        })?;
        let xdg_config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        let xdg_data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"));
        let config_dir = xdg_config.join("tome");
        let config_file = config_dir.join("config.toml");
        let data_dir = xdg_data.join("tome");
        let catalogs_dir = data_dir.join("catalogs");
        let index_db = data_dir.join("index.db");
        let index_lock = data_dir.join("index.lock");
        let models_dir = data_dir.join("models");
        Ok(Self {
            config_dir,
            config_file,
            data_dir,
            catalogs_dir,
            index_db,
            index_lock,
            models_dir,
        })
    }

    pub fn cache_dir_for(&self, url: &str) -> PathBuf {
        let mut h = Sha256::new();
        h.update(url.as_bytes());
        self.catalogs_dir.join(hex::encode(h.finalize()))
    }

    /// On-disk root for a named model. The directory contains the model
    /// artefact(s) plus a Tome-owned `manifest.json` (see `ModelManifest`).
    /// Reject empty / traversing / absolute names at the boundary so callers
    /// can rely on the returned path staying inside `models_dir`.
    pub fn model_path(&self, name: &str) -> Result<PathBuf, TomeError> {
        if name.is_empty() {
            return Err(TomeError::Usage("model name is empty".into()));
        }
        if name.contains('/') || name.contains('\\') || name == "." || name == ".." {
            return Err(TomeError::Usage(format!(
                "model name `{name}` contains a path separator or traversal",
            )));
        }
        if Path::new(name).is_absolute() {
            return Err(TomeError::Usage(format!(
                "model name `{name}` is an absolute path",
            )));
        }
        Ok(self.models_dir.join(name))
    }

    /// Per-model manifest path (the JSON file Tome writes after a verified
    /// download).
    pub fn model_manifest(&self, name: &str) -> Result<PathBuf, TomeError> {
        Ok(self.model_path(name)?.join("manifest.json"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> Paths {
        let data = PathBuf::from("/tmp/d");
        Paths {
            config_dir: PathBuf::from("/tmp/c"),
            config_file: PathBuf::from("/tmp/c/config.toml"),
            data_dir: data.clone(),
            catalogs_dir: data.join("catalogs"),
            index_db: data.join("index.db"),
            index_lock: data.join("index.lock"),
            models_dir: data.join("models"),
        }
    }

    #[test]
    fn cache_dir_is_deterministic_per_url() {
        let p = fixture();
        let a = p.cache_dir_for("https://github.com/owner/repo");
        let b = p.cache_dir_for("https://github.com/owner/repo");
        assert_eq!(a, b);
        let c = p.cache_dir_for("https://github.com/owner/other");
        assert_ne!(a, c);
        // sha256 hex is 64 chars
        assert_eq!(a.file_name().unwrap().to_str().unwrap().len(), 64);
    }

    #[test]
    fn phase_2_paths_are_under_data_dir() {
        // FR-021: index, lock, and models live under the per-user data dir
        // (not cache dir), so OS cache cleaners cannot silently wipe them.
        let p = fixture();
        assert!(p.index_db.starts_with(&p.data_dir));
        assert!(p.index_lock.starts_with(&p.data_dir));
        assert!(p.models_dir.starts_with(&p.data_dir));
    }

    #[test]
    fn model_path_accepts_simple_name() {
        let p = fixture();
        let got = p.model_path("bge-small-en-v1.5").unwrap();
        assert_eq!(got, p.models_dir.join("bge-small-en-v1.5"));
    }

    #[test]
    fn model_path_rejects_traversal_and_separators() {
        let p = fixture();
        for bad in ["", ".", "..", "../etc", "a/b", "a\\b", "/abs/path"] {
            assert!(
                p.model_path(bad).is_err(),
                "model_path({bad:?}) should have errored",
            );
        }
    }

    #[test]
    fn model_manifest_lives_inside_model_dir() {
        let p = fixture();
        let m = p.model_manifest("bge-reranker-base").unwrap();
        assert_eq!(
            m,
            p.models_dir.join("bge-reranker-base").join("manifest.json"),
        );
    }
}

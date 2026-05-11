//! XDG-aware path resolution. We honour `XDG_CONFIG_HOME` and `XDG_DATA_HOME`
//! across both macOS and Linux (rather than `directories`'s macOS-specific
//! `~/Library/...` fallback) — the spec is explicit about the XDG layout and
//! tests rely on it being controllable via the env vars. `cache_dir_for(url)`
//! content-addresses each catalog's cache by sha256(url) (FR-015).

use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::error::TomeError;

#[derive(Debug, Clone)]
pub struct Paths {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub data_dir: PathBuf,
    pub catalogs_dir: PathBuf,
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
        Ok(Self {
            config_dir,
            config_file,
            data_dir,
            catalogs_dir,
        })
    }

    pub fn cache_dir_for(&self, url: &str) -> PathBuf {
        let mut h = Sha256::new();
        h.update(url.as_bytes());
        self.catalogs_dir.join(hex::encode(h.finalize()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_dir_is_deterministic_per_url() {
        let p = Paths {
            config_dir: PathBuf::from("/tmp/c"),
            config_file: PathBuf::from("/tmp/c/config.toml"),
            data_dir: PathBuf::from("/tmp/d"),
            catalogs_dir: PathBuf::from("/tmp/d/catalogs"),
        };
        let a = p.cache_dir_for("https://github.com/owner/repo");
        let b = p.cache_dir_for("https://github.com/owner/repo");
        assert_eq!(a, b);
        let c = p.cache_dir_for("https://github.com/owner/other");
        assert_ne!(a, c);
        // sha256 hex is 64 chars
        assert_eq!(a.file_name().unwrap().to_str().unwrap().len(), 64);
    }
}

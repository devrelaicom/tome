//! XDG-aware path resolution. `directories` picks the right base on macOS and
//! Linux; `cache_dir_for(url)` content-addresses each catalog's cache by
//! sha256(url) so identical URLs land in the same directory (FR-015).

use std::path::PathBuf;

use directories::ProjectDirs;
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
        // qualifier="", organization="", application="tome" gives us the
        // unscoped XDG layout the spec asks for on Linux and the same on macOS
        // (where `directories` maps to the equivalent Application Support
        // location only when `qualifier` is non-empty — here we stay XDG).
        let dirs = ProjectDirs::from("", "", "tome").ok_or_else(|| {
            TomeError::Io(std::io::Error::other(
                "could not determine an XDG-compatible home directory",
            ))
        })?;
        let config_dir = dirs.config_dir().to_path_buf();
        let config_file = config_dir.join("config.toml");
        let data_dir = dirs.data_dir().to_path_buf();
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

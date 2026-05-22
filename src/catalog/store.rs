//! Atomic registry persistence. Writes go through `tempfile::NamedTempFile`
//! in the same directory as the target file, then rename — POSIX-atomic on a
//! single filesystem (FR-017b).
//!
//! On Unix the persisted file is chmod 0o600 — `config.toml` holds catalog
//! source URLs, which are not secrets today but can carry user-supplied
//! tokens via the user-typed `tome catalog add` source. The umask-default
//! 0644 would let any local user read those URLs.

use std::io::Write;
use std::path::Path;

use tempfile::NamedTempFile;

use crate::config::Config;
use crate::error::TomeError;
use crate::paths::Paths;
use crate::workspace::{Scope, WorkspaceName};

pub fn load(config_file: &Path) -> Result<Config, TomeError> {
    match std::fs::read_to_string(config_file) {
        Ok(text) => {
            let parsed: Config = toml::from_str(&text).map_err(|e| {
                TomeError::Internal(anyhow::anyhow!(
                    "config file `{}` is not valid: {}",
                    config_file.display(),
                    e
                ))
            })?;
            Ok(parsed)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(TomeError::Io(e)),
    }
}

pub fn save(config_file: &Path, config: &Config) -> Result<(), TomeError> {
    let parent = config_file
        .parent()
        .ok_or_else(|| TomeError::Io(std::io::Error::other("config path has no parent")))?;
    std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    let text =
        toml::to_string_pretty(config).map_err(|e| TomeError::Internal(anyhow::Error::new(e)))?;
    write_atomic(config_file, text.as_bytes())
}

/// Enumerate every scope whose `config.toml` currently references the
/// catalog URL. Used by `tome catalog remove` to decide whether the
/// on-disk clone at `paths.cache_dir_for(url)` is still needed after the
/// resolved scope drops it.
///
/// Phase 4 / F2a transitional behaviour: with the XDG-separated paths
/// gone and the `workspaces.txt` opt-in registry deleted, this function
/// only walks the central global `config.toml`. The Phase 3 per-workspace
/// `.tome/config.toml` enumeration is dropped here; F11 reintroduces
/// proper workspace-aware reference counting via the `workspace_catalogs`
/// junction table once the central DB schema is in place.
///
/// URL equality is exact-string match. Callers must pass the same
/// scrubbed-and-resolved URL the catalog was registered with.
///
/// ## Concurrency / TOCTOU
///
/// The reference-count read is **not** taken under any lock. The TOCTOU
/// profile is unchanged from Phase 3: see the Phase 3 retro notes on
/// `catalog::store::reference_count` for the full discussion.
pub fn reference_count(url: &str, paths: &Paths) -> Vec<Scope> {
    let mut refs = Vec::new();
    if let Ok(global) = load(&paths.global_config_file)
        && global.catalogs.values().any(|e| e.url == url)
    {
        // F11 rewires this onto the workspace_catalogs junction.
        refs.push(Scope(WorkspaceName::global()));
    }
    refs
}

/// Write `bytes` to `target` atomically: write to a same-directory temp file,
/// fsync, set chmod 0o600 (Unix only), then rename. The rename is the only
/// step visible to readers.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), TomeError> {
    let parent = target
        .parent()
        .ok_or_else(|| TomeError::Io(std::io::Error::other("target path has no parent")))?;
    std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    let mut tmp = NamedTempFile::new_in(parent).map_err(TomeError::Io)?;
    tmp.write_all(bytes).map_err(TomeError::Io)?;
    tmp.as_file().sync_all().map_err(TomeError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(tmp.path(), perms).map_err(TomeError::Io)?;
    }
    tmp.persist(target).map_err(|e| TomeError::Io(e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn write_atomic_creates_file_with_exact_bytes() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("a.toml");
        write_atomic(&target, b"hello").unwrap();
        let read = std::fs::read(&target).unwrap();
        assert_eq!(read, b"hello");
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("config.toml");
        let cfg = Config::default();
        save(&file, &cfg).unwrap();
        let back = load(&file).unwrap();
        assert_eq!(cfg, back);
    }

    #[test]
    fn load_missing_file_returns_empty_config() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("does-not-exist.toml");
        let cfg = load(&file).unwrap();
        assert!(cfg.catalogs.is_empty());
    }
}

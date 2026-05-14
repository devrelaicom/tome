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
use crate::workspace::{Scope, inventory};

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
/// Sources walked (contract `catalog-extensions-p3.md` §Reference-counting):
///
/// 1. The global `config.toml` (`paths.config_file`).
/// 2. Every workspace path in `paths.workspace_registry` — opt-in; the
///    file is absent on a fresh install and reads as an empty list.
///
/// URL equality is exact-string match. Callers must pass the same
/// scrubbed-and-resolved URL the catalog was registered with.
///
/// ## Concurrency / TOCTOU
///
/// The reference-count read is **not** taken under any lock. Two
/// processes racing through `catalog remove` may both observe an empty
/// list and both call `fs::remove_dir_all`; one succeeds, the other
/// gets `NotFound` and silently continues. The worse race — process A
/// observes empty, process B adds a new reference, process A removes
/// the clone — leaves a dangling reference; the next
/// `tome catalog update` for that scope re-clones. No data loss; one
/// extra round-trip. Same TOCTOU profile as the Phase 2
/// `cascade_disable_for_catalog` pre-check.
pub fn reference_count(url: &str, paths: &Paths) -> Vec<Scope> {
    let mut refs = Vec::new();
    if let Ok(global) = load(&paths.config_file)
        && global.catalogs.values().any(|e| e.url == url)
    {
        refs.push(Scope::Global);
    }
    for ws in inventory::read_registry(&paths.workspace_registry) {
        let cfg_path = ws.join(".tome/config.toml");
        let Ok(cfg) = load(&cfg_path) else { continue };
        if cfg.catalogs.values().any(|e| e.url == url) {
            refs.push(Scope::Workspace(ws));
        }
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

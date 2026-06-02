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

pub fn load(config_file: &Path) -> Result<Config, TomeError> {
    match crate::util::bounded_read_to_string(config_file, crate::util::TOME_CONFIG_MAX) {
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
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(e),
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

/// Write `bytes` to `target` atomically: write to a same-directory temp file,
/// fsync, set the file mode, then rename. The rename is the only step
/// visible to readers.
///
/// **Mode preservation (Unix)**: when `target` already exists, the staged
/// tempfile is chmod'd to match the prior file's mode before persisting,
/// preserving any developer-set mode bits (e.g. group-readable workspace
/// files) across the rewrite. When `target` is absent, the staged tempfile
/// is chmod'd to 0o600 — `config.toml` and friends may carry user-supplied
/// tokens via `tome catalog add` source URLs; the umask-default 0644 would
/// let any local user read them.
///
/// **Symlink refusal**: if `target` is an existing symlink, the write is
/// refused (returns `TomeError::Io` / exit 7). Mirrors the discipline
/// established in `src/harness/rules_file.rs` and `src/harness/
/// mcp_config.rs` (Phase 3 P8 PR-F → Phase 4 US1.d-2a S-M3). Without this
/// guard a TOCTOU race could see the target replaced by a symlink between
/// the parent's `create_dir_all` and the rename, causing the rename to
/// dereference and clobber the link target.
pub fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), TomeError> {
    let parent = target
        .parent()
        .ok_or_else(|| TomeError::Io(std::io::Error::other("target path has no parent")))?;
    std::fs::create_dir_all(parent).map_err(TomeError::Io)?;

    // Symlink refusal: TOCTOU-protective. `rename(2)` is no-follow, but
    // a hostile actor could still aim a pre-existing symlink at e.g.
    // `~/.ssh/authorized_keys` and have our `set_permissions` follow it
    // before we rename — explicit refusal is cleaner than relying on
    // the rename's no-follow semantics. Delegates to the SSOT guard
    // (`util::symlink_safe`) so the registry sink also refuses a symlinked
    // INTERMEDIATE component (FR-007), not just the final node. Refusal →
    // `TomeError::Io` (exit 7), this sink's existing dedicated code.
    crate::util::refuse_symlinked_component(target).map_err(TomeError::Io)?;

    // Mode preservation: capture the prior file's mode (if any) so we can
    // reapply it after the atomic rename. Absent target → fall through to
    // the 0o600 default below.
    #[cfg(unix)]
    let target_mode: Option<u32> = {
        use std::os::unix::fs::PermissionsExt;
        std::fs::symlink_metadata(target)
            .ok()
            .map(|m| m.permissions().mode())
    };

    let mut tmp = NamedTempFile::new_in(parent).map_err(TomeError::Io)?;
    tmp.write_all(bytes).map_err(TomeError::Io)?;
    tmp.as_file().sync_all().map_err(TomeError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = target_mode.unwrap_or(0o600);
        let perms = std::fs::Permissions::from_mode(mode);
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

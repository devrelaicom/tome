//! Bounded, capacity-pre-allocating file reads.
//!
//! Phase 4 Polish PR-E (S-M1) introduces this helper to replace the
//! 26+ unbounded `std::fs::read_to_string` calls scattered across the
//! tree. Each call site was happy to slurp a file of unbounded size
//! into memory — fine for a well-behaved local `config.toml`, less fine
//! when a hostile catalog ships a multi-GiB `SKILL.md` or a corrupt
//! `tome.json` MCP config grows unboundedly.
//!
//! ## Per-class caps
//!
//! The caps are deliberately conservative; bump on evidence rather than
//! prophylactically. The constants live here so a future tightening (or
//! widening) is a one-line edit:
//!
//! - [`TOME_CONFIG_MAX`] (1 MiB): Tome-owned config / settings TOML.
//! - [`PLUGIN_MANIFEST_MAX`] (256 KiB): third-party plugin manifests
//!   and `SKILL.md` frontmatter blocks.
//! - [`HARNESS_MCP_MAX`] (1 MiB): harness MCP config files (JSON / TOML)
//!   owned by external harnesses; we read these to splice the `tome`
//!   entry but otherwise round-trip the existing content.
//! - [`HARNESS_RULES_MAX`] (4 MiB): harness rules files (e.g. `CLAUDE.md`,
//!   `AGENTS.md`); user-authored prose can run long.
//! - [`ENTRY_BODY_MAX`] (1 MiB): the Markdown body of an untrusted entry
//!   (`SKILL.md` / command / agent) being read by a Phase 8 `convert`
//!   importer (R13). A body over 1 MiB is pathological for an LLM-facing
//!   skill; the cap bounds a hostile source tree.
//!
//! ## Allocation behaviour
//!
//! The helper pre-allocates the destination `String` to the smaller of
//! the file's reported length and the per-call cap, so the happy path
//! reads in a single syscall without any reallocation.

use std::path::Path;

use crate::error::TomeError;

/// Cap for Tome-owned config / settings TOML files (1 MiB).
pub const TOME_CONFIG_MAX: u64 = 1024 * 1024;

/// Cap for plugin manifests + SKILL.md frontmatter (256 KiB).
pub const PLUGIN_MANIFEST_MAX: u64 = 256 * 1024;

/// Cap for harness-owned MCP config files (1 MiB).
pub const HARNESS_MCP_MAX: u64 = 1024 * 1024;

/// Cap for harness-owned rules files such as `CLAUDE.md` (4 MiB).
pub const HARNESS_RULES_MAX: u64 = 4 * 1024 * 1024;

/// Cap for the Markdown body of an untrusted entry being converted (1 MiB).
pub const ENTRY_BODY_MAX: u64 = 1024 * 1024;

/// Read a UTF-8 file into a `String`, refusing files larger than
/// `max_bytes`.
///
/// Pre-allocates `String::with_capacity(min(file_len, max_bytes))` so
/// the happy path is a single syscall with no reallocation. Returns
/// [`TomeError::Io`] with [`std::io::ErrorKind::InvalidInput`] when the
/// file's metadata-reported length exceeds the cap; this avoids
/// streaming a hostile multi-GiB file into memory before refusing.
///
/// Non-UTF-8 contents surface the underlying
/// [`std::io::ErrorKind::InvalidData`] from
/// [`std::io::Read::read_to_string`], unchanged.
///
/// # Errors
///
/// - [`TomeError::Io`] (exit 7) when the file is missing, unreadable,
///   over-cap, or not valid UTF-8.
pub fn bounded_read_to_string(path: &Path, max_bytes: u64) -> Result<String, TomeError> {
    let meta = std::fs::metadata(path).map_err(TomeError::Io)?;
    let len = meta.len();
    if len > max_bytes {
        return Err(TomeError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "file {} exceeds the {} byte cap (size: {} bytes)",
                path.display(),
                max_bytes,
                len
            ),
        )));
    }
    // Cap the pre-allocation so a sparse / lying metadata doesn't push
    // us past the budget either. usize cast: max_bytes is small (≤ 4 MiB
    // in production callers) and fits comfortably on every supported
    // platform.
    let cap = std::cmp::min(len, max_bytes) as usize;
    let mut buf = String::with_capacity(cap);
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(TomeError::Io)?;
    file.read_to_string(&mut buf).map_err(TomeError::Io)?;
    Ok(buf)
}

/// Read a file's raw bytes into a `Vec<u8>`, refusing files larger than
/// `max_bytes`. Mirrors [`bounded_read_to_string`] but for callers that
/// need raw bytes (e.g. byte-equality compares of rules files).
///
/// # Errors
///
/// - [`TomeError::Io`] (exit 7) when the file is missing, unreadable,
///   or over-cap.
pub fn bounded_read(path: &Path, max_bytes: u64) -> Result<Vec<u8>, TomeError> {
    let meta = std::fs::metadata(path).map_err(TomeError::Io)?;
    let len = meta.len();
    if len > max_bytes {
        return Err(TomeError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "file {} exceeds the {} byte cap (size: {} bytes)",
                path.display(),
                max_bytes,
                len
            ),
        )));
    }
    let cap = std::cmp::min(len, max_bytes) as usize;
    let mut buf = Vec::with_capacity(cap);
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(TomeError::Io)?;
    file.read_to_end(&mut buf).map_err(TomeError::Io)?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn small_file_accepts() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("small.txt");
        std::fs::write(&path, b"hello world").unwrap();
        let body = bounded_read_to_string(&path, TOME_CONFIG_MAX).unwrap();
        assert_eq!(body, "hello world");
    }

    #[test]
    fn exact_size_accepts() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("exact.txt");
        let body: Vec<u8> = (0..128).map(|i| b'A' + (i % 26) as u8).collect();
        std::fs::write(&path, &body).unwrap();
        let read = bounded_read_to_string(&path, 128).unwrap();
        assert_eq!(read.len(), 128);
    }

    #[test]
    fn over_cap_rejects_before_read() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("big.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(&vec![b'A'; 2048]).unwrap();
        let err = bounded_read_to_string(&path, 1024).unwrap_err();
        match err {
            TomeError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::InvalidInput),
            other => panic!("expected Io InvalidInput, got {other:?}"),
        }
    }

    #[test]
    fn missing_file_returns_io_error() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("does-not-exist.txt");
        let err = bounded_read_to_string(&path, TOME_CONFIG_MAX).unwrap_err();
        match err {
            TomeError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("expected Io NotFound, got {other:?}"),
        }
    }
}

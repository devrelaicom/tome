//! JSON-lines file appender + size-based rotation + tracing subscriber.
//!
//! Per [`contracts/log-format.md`](../../specs/003-phase-3-mcp-workspaces/contracts/log-format.md):
//! the file lives at `${XDG_STATE_HOME}/tome/mcp.log`, rotates to
//! `mcp.log.1` at startup if oversized (10 MiB cap), and the on-disk
//! footprint is bounded at ~20 MiB total per machine.
//!
//! The MCP server is the one Tome surface allowed to use `tracing`'s
//! JSON formatter. Every other command renders human-readable logs to
//! stderr via [`crate::logging`]; this module installs a separate
//! subscriber that writes structured JSON to a file plus a stderr layer
//! filtered to `error!` only (FR-222 — stderr is reserved for fatal
//! startup errors).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::error::TomeError;
use crate::paths::Paths;

/// 10 MiB rotation cap per the log-format contract.
pub const ROTATE_AT_BYTES: u64 = 10 * 1024 * 1024;

/// Rotate `current → prev` if `current` exceeds [`ROTATE_AT_BYTES`].
/// Atomic rename, overwriting any pre-existing `prev`. Idempotent —
/// safe to call on every startup regardless of whether the log exists.
pub fn rotate_if_oversized(current: &Path, prev: &Path) -> Result<(), TomeError> {
    match std::fs::metadata(current) {
        Ok(meta) if meta.len() > ROTATE_AT_BYTES => {
            std::fs::rename(current, prev).map_err(TomeError::Io)
        }
        _ => Ok(()),
    }
}

/// Open the MCP log file in append mode, creating the parent directory
/// and the file itself if absent. Caller is responsible for rotating
/// first via [`rotate_if_oversized`] — [`open_appender`] is the one-shot
/// entry that does both.
pub fn open_appender(paths: &Paths) -> Result<File, TomeError> {
    rotate_if_oversized(&paths.mcp_log, &paths.mcp_log_prev)?;

    if let Some(parent) = paths.mcp_log.parent() {
        std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
    }

    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.mcp_log)
        .map_err(TomeError::Io)
}

/// `tracing-subscriber`-compatible writer that serialises every emit
/// through a `Mutex<File>`. The MCP server is single-threaded by design
/// (research §R-2), so contention here is theoretical; the mutex keeps
/// the appender `Send + Sync` and lets tests share a handle.
pub struct FileMakeWriter {
    inner: Mutex<File>,
}

impl FileMakeWriter {
    pub fn new(file: File) -> Self {
        Self {
            inner: Mutex::new(file),
        }
    }
}

/// Held by [`FileMakeWriter::make_writer`]; forwards `Write` to the
/// inner [`File`] via the blanket `impl Write for &File`. The mutex
/// guard stays alive for the duration of one log emit, preventing
/// interleaved writes from corrupting JSON-lines framing.
pub struct LockedFile<'a>(MutexGuard<'a, File>);

impl<'a> Write for LockedFile<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        (&*self.0).write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        (&*self.0).flush()
    }
}

impl<'a> MakeWriter<'a> for FileMakeWriter {
    type Writer = LockedFile<'a>;
    fn make_writer(&'a self) -> Self::Writer {
        LockedFile(
            self.inner
                .lock()
                .expect("mcp log mutex poisoned — concurrent writer panicked"),
        )
    }
}

/// Build the MCP tracing subscriber. Wires the JSON file layer (filtered
/// by `TOME_LOG` / `RUST_LOG`, default `info`) and a stderr layer
/// restricted to `error!`-and-above so fatal startup diagnostics survive
/// even if the file handle isn't open yet.
///
/// Returns `Err(TomeError::McpStartupFailed)` if a subscriber is already
/// installed for this thread — `try_init` is fallible to keep the call
/// safe for tests that may have an existing global subscriber from the
/// CLI logging module.
pub fn init_subscriber(file: File) -> Result<(), TomeError> {
    let env_filter = EnvFilter::try_from_env("TOME_LOG")
        .or_else(|_| EnvFilter::try_from_env("RUST_LOG"))
        .unwrap_or_else(|_| EnvFilter::new("info"));

    let file_layer = fmt::layer()
        .json()
        .with_writer(FileMakeWriter::new(file))
        .with_target(true)
        .with_current_span(false)
        .with_span_list(false);

    let stderr_layer = fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false)
        .without_time()
        .with_filter(LevelFilter::ERROR);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .with(stderr_layer)
        .try_init()
        .map_err(|e| TomeError::McpStartupFailed {
            reason: format!("install mcp tracing subscriber: {e}"),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn rotate_skips_when_under_cap() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("mcp.log");
        let prev = dir.path().join("mcp.log.1");
        let mut f = File::create(&current).unwrap();
        f.write_all(b"hello\n").unwrap();
        drop(f);

        rotate_if_oversized(&current, &prev).unwrap();
        assert!(current.exists(), "small file must stay in place");
        assert!(!prev.exists(), "rotation must not run below the cap");
    }

    #[test]
    fn rotate_renames_when_oversized() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("mcp.log");
        let prev = dir.path().join("mcp.log.1");
        let f = File::create(&current).unwrap();
        f.set_len(ROTATE_AT_BYTES + 1).unwrap();
        drop(f);

        rotate_if_oversized(&current, &prev).unwrap();
        assert!(!current.exists(), "oversized current must be renamed away");
        assert!(prev.exists(), "rotation must produce a .1");
    }

    #[test]
    fn rotate_overwrites_existing_prev() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("mcp.log");
        let prev = dir.path().join("mcp.log.1");

        // Pre-existing .1 from a previous session.
        File::create(&prev).unwrap().write_all(b"old").unwrap();
        // Oversized current.
        let f = File::create(&current).unwrap();
        f.set_len(ROTATE_AT_BYTES + 1).unwrap();
        drop(f);

        rotate_if_oversized(&current, &prev).unwrap();
        // .1 now holds the bytes that were in current.
        let meta = std::fs::metadata(&prev).unwrap();
        assert!(
            meta.len() > ROTATE_AT_BYTES,
            "rotation must overwrite the pre-existing .1",
        );
    }

    #[test]
    fn rotate_is_noop_when_current_absent() {
        let dir = TempDir::new().unwrap();
        let current = dir.path().join("nonexistent.log");
        let prev = dir.path().join("nonexistent.log.1");
        // No file to rotate. Must not error.
        rotate_if_oversized(&current, &prev).unwrap();
        assert!(!prev.exists());
    }
}

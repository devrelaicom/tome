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

use std::fmt as stdfmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use serde_json::{Map, Number, Value};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use tracing::Event;
use tracing::Subscriber;
use tracing::field::{Field, Visit};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::Layer;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::writer::MakeWriter;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
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

/// Custom JSON-lines event formatter that emits the contract-pinned
/// field names (`ts`, `level`, `target`, `msg`). `tracing-subscriber`'s
/// default `.json()` formatter uses `timestamp` and `message` — a
/// silent divergence from `contracts/log-format.md` §File format. Every
/// `tail -F | jq` filter in the contract depends on these exact names,
/// so the formatter renders them directly via `serde_json` rather than
/// relying on `tracing-subscriber`'s reserved field names.
///
/// Output shape per line:
///
/// ```json
/// {"ts":"2026-05-14T12:34:55.823Z","level":"info","target":"tome::mcp::server","msg":"startup ok",…}
/// ```
///
/// All event-level structured fields flatten in alongside the required
/// four. Field-name collisions with the four required names (an event
/// recording a field literally named `ts` / `level` / `target` / `msg`)
/// are NOT possible in our codebase by inspection; reserve the names
/// for the framework.
pub struct ContractEventFormat;

impl<S, N> FormatEvent<S, N> for ContractEventFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> stdfmt::Result {
        let meta = event.metadata();
        let mut out = Map::new();

        let ts = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_owned());
        out.insert("ts".to_owned(), Value::String(ts));
        out.insert(
            "level".to_owned(),
            Value::String(meta.level().to_string().to_lowercase()),
        );
        out.insert("target".to_owned(), Value::String(meta.target().to_owned()));

        let mut visitor = JsonFieldVisitor::default();
        event.record(&mut visitor);

        // tracing routes the macro message through a reserved field
        // named `message`; lift it to `msg` per contract.
        if let Some(msg) = visitor.fields.remove("message") {
            out.insert("msg".to_owned(), msg);
        }
        for (k, v) in visitor.fields {
            out.insert(k, v);
        }

        let line = serde_json::to_string(&out).map_err(|_| stdfmt::Error)?;
        writeln!(writer, "{line}")
    }
}

/// `tracing::Field` visitor that collects every event field into a
/// `serde_json::Map`. Numeric / boolean / string fields land typed;
/// everything else (including `?` and `%` formatted values) becomes a
/// JSON string via the value's `Debug` impl.
#[derive(Default)]
struct JsonFieldVisitor {
    fields: Map<String, Value>,
}

impl Visit for JsonFieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn stdfmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), Value::String(format!("{value:?}")));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), Value::String(value.to_owned()));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().to_owned(), Value::Number(value.into()));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_owned(), Value::Number(value.into()));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_owned(), Value::Bool(value));
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        let n = Number::from_f64(value).unwrap_or_else(|| Number::from(0));
        self.fields
            .insert(field.name().to_owned(), Value::Number(n));
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
        .event_format(ContractEventFormat)
        .with_writer(FileMakeWriter::new(file));

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

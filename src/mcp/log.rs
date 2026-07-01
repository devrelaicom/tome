//! JSON-lines file appender + size-based rotation + tracing subscriber.
//!
//! Per [`contracts/log-format.md`](../../specs/003-phase-3-mcp-workspaces/contracts/log-format.md):
//! the file lives at `<home>/.tome/logs/mcp.log`, rotates to
//! `mcp.log.1` at startup if oversized (10 MiB cap), and the on-disk
//! footprint is bounded at ~20 MiB total per machine.
//!
//! The MCP server is the one Tome surface allowed to use `tracing`'s
//! JSON formatter. Every other command renders human-readable logs to
//! stderr via [`crate::logging`]; this module installs a separate
//! subscriber that writes structured JSON to a file plus a stderr layer
//! filtered to `error!` only (FR-222 — stderr is reserved for fatal
//! startup errors).
//!
//! # `TOME_MCP_LOG` — disable or redirect the file sink
//!
//! The file sink is on by default. The `TOME_MCP_LOG` environment
//! variable overrides where — or whether — it is written. This is
//! distinct from `TOME_LOG` / `RUST_LOG`, which tune *verbosity*; this
//! var controls the *destination* of the file sink only. stdout stays
//! protocol-only and stderr stays `error!`-only regardless.
//!
//! - unset → default path (`<home>/.tome/logs/mcp.log`, 10 MiB rotation).
//! - `off` (case-insensitive) or empty → NO file sink is opened. Nothing
//!   is created on disk; only the stderr `error!` layer stays.
//! - `<path>` → open the rotating file sink at `<path>` instead of the
//!   default, creating parent directories and rotating `<path>.1` with
//!   the same 10 MiB cap.
//!
//! Resolution is **fail-soft**: if an explicit override path cannot be
//! opened (bad path, permissions, or a directory), the server prints one
//! warning to stderr and continues with NO file sink rather than
//! crashing — the MCP server must always start. The default path (unset)
//! is NOT fail-soft: a failure there still propagates as a `TomeError`,
//! preserving the pre-existing byte-identical behaviour.

use std::fmt as stdfmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
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
use crate::logging::resolve_directive;
use crate::paths::Paths;

/// 10 MiB rotation cap per the log-format contract.
pub const ROTATE_AT_BYTES: u64 = 10 * 1024 * 1024;

/// Environment variable that overrides the MCP file-log destination.
///
/// Deliberately distinct from `TOME_LOG` / `RUST_LOG` (which control
/// verbosity): this var controls *whether* and *where* the file sink is
/// written. See the module docs for the accepted values.
pub const LOG_ENV: &str = "TOME_MCP_LOG";

/// Resolved destination for the MCP file-log sink.
///
/// Produced by [`resolve_sink`] from the default path plus the raw
/// `TOME_MCP_LOG` value; consumed by [`open_sink`] to decide whether to
/// open a file at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogSink {
    /// No file sink — nothing is opened or created on disk.
    Off,
    /// Open the rotating file sink at this path.
    File(PathBuf),
}

/// Pure resolver for the file-sink destination. Kept free of environment
/// and filesystem access so it is directly unit-testable; callers read
/// [`LOG_ENV`] and pass the raw value in.
///
/// Semantics (see the module docs):
/// - `None` (unset) → `File(default_log)` — the historical behaviour.
/// - `Some("off")` (ASCII case-insensitive, surrounding whitespace
///   trimmed) or an empty/whitespace-only value → [`LogSink::Off`].
/// - `Some(path)` → `File(path)` (trimmed of surrounding whitespace).
pub fn resolve_sink(default_log: &Path, env: Option<&str>) -> LogSink {
    match env {
        None => LogSink::File(default_log.to_path_buf()),
        Some(raw) => {
            let trimmed = raw.trim();
            if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("off") {
                LogSink::Off
            } else {
                LogSink::File(PathBuf::from(trimmed))
            }
        }
    }
}

/// Open the file sink for the resolved [`LogSink`], honouring
/// [`LOG_ENV`]. Returns `Ok(None)` when the sink is disabled (or an
/// explicit override path could not be opened — a warning is printed to
/// stderr and the server continues sink-less). Returns `Err` ONLY when the *default*
/// (unset) path fails to open, preserving the pre-override behaviour where
/// the harness sees a `TomeError` on stderr rather than a silently missing
/// default log.
///
/// This is the entry point `tome mcp` calls to obtain the file sink at
/// startup, wrapping [`open_appender_at`] with the `TOME_MCP_LOG` policy.
pub fn open_sink(paths: &Paths) -> Result<Option<File>, TomeError> {
    let env = std::env::var(LOG_ENV).ok();
    match resolve_sink(&paths.mcp_log, env.as_deref()) {
        LogSink::Off => Ok(None),
        // Deliberate asymmetry: an explicit `TOME_MCP_LOG` set to the EXACT
        // default path routes here to the fail-LOUD branch (it *is* the
        // default location, so a failure there is as fatal as it always
        // was), while any OTHER override path falls through to the
        // fail-soft branch below.
        LogSink::File(path) if path == paths.mcp_log => {
            // Default path: fail loud, exactly as before the override existed.
            open_appender_at(&path).map(Some)
        }
        LogSink::File(path) => {
            // Explicit override: fail soft so the server always starts.
            match open_appender_at(&path) {
                Ok(file) => Ok(Some(file)),
                Err(e) => {
                    // The tracing subscriber is not installed yet (that
                    // happens in `init_subscriber`, after this returns), so
                    // write the degradation notice straight to stderr — the
                    // channel reserved for fatal/startup diagnostics. Ignore
                    // the write result; a broken stderr must not abort startup.
                    let _ = writeln!(
                        io::stderr(),
                        "warning: {LOG_ENV}={} could not be opened ({e}); \
                         continuing with no MCP file log",
                        path.display()
                    );
                    Ok(None)
                }
            }
        }
    }
}

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

/// Open a rotating log-file appender at an arbitrary path.
///
/// Rotates `<path>` → `<path>.1` first via [`rotate_if_oversized`],
/// creates the parent directory if absent, then opens `<path>` in append
/// mode with `0600` permissions on Unix. This is the shared core behind
/// both the default sink and the [`LOG_ENV`] override.
pub fn open_appender_at(path: &Path) -> Result<File, TomeError> {
    let prev = rotation_sibling(path);
    rotate_if_oversized(path, &prev)?;

    if let Some(parent) = path.parent() {
        // `parent()` of a bare filename is `Some("")`, which
        // `create_dir_all` treats as the current directory (a no-op);
        // guard it to avoid a spurious error on some platforms.
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(TomeError::Io)?;
        }
    }

    // FR-S-01: the MCP log carries workspace paths + scrubbed error
    // chains; on a shared machine the default umask-0644 lets every
    // local user read the file. Match the discipline Phase 2 / PR #36
    // applied to `config.toml` and the workspace registry: chmod 0600
    // explicitly. On Unix the `mode(0o600)` `OpenOptions` extension
    // takes effect at creation; this is a no-op on Windows (ACL model
    // not covered by this PR).
    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let file = opts.open(path).map_err(TomeError::Io)?;

    // If the file already existed with a more-permissive mode (e.g.
    // pre-fix install), tighten it now. `set_permissions` is a no-op
    // on Windows.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(path, perms);
    }

    Ok(file)
}

/// The rotation sibling for a log path: `<path>` → `<path>.1`.
/// Mirrors the default `mcp.log` → `mcp.log.1` pairing for override paths.
fn rotation_sibling(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".1");
    PathBuf::from(name)
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
/// by `TOME_LOG` / `RUST_LOG`, then `[logging] level` in `config.toml`,
/// default `info`) and a stderr layer restricted to `error!`-and-above so
/// fatal startup diagnostics survive even if the file handle isn't open yet.
///
/// Precedence for the MCP log level:
/// 1. `TOME_LOG` env var
/// 2. `RUST_LOG` env var
/// 3. `[logging] level` in `config.toml` (`config_level`)
/// 4. Built-in default: `"info"`
///
/// Note: the `-v`/`-vv` flag is a CLI-only concept and is not threaded into
/// the MCP server, so `Verbosity` is not consulted here.
///
/// The file sink is optional: pass `None` (from [`open_sink`] resolving
/// `TOME_MCP_LOG=off` or a fail-soft override) to install ONLY the stderr
/// `error!` layer, opening no file. When `Some(file)`, the JSON file layer
/// is added alongside it. stdout is untouched in both cases.
///
/// Returns `Err(TomeError::McpStartupFailed)` if a subscriber is already
/// installed for this thread — `try_init` is fallible to keep the call
/// safe for tests that may have an existing global subscriber from the
/// CLI logging module.
pub fn init_subscriber(
    file: Option<File>,
    config_level: Option<crate::config::LogLevel>,
) -> Result<(), TomeError> {
    // Build the effective directive: same precedence as the CLI path but with
    // no verbosity flag (MCP has none) and `"info"` as the built-in default.
    let directive = resolve_directive(
        None,
        config_level,
        std::env::var("TOME_LOG").ok(),
        std::env::var("RUST_LOG").ok(),
        "info",
    );
    let env_filter = EnvFilter::new(directive);

    // The file layer is optional; `Option<Layer>` implements `Layer` and is
    // a no-op when `None`, so the registry composition stays byte-identical
    // to the pre-override wiring whenever a file is present.
    let file_layer = file.map(|f| {
        fmt::layer()
            .event_format(ContractEventFormat)
            .with_writer(FileMakeWriter::new(f))
    });

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
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Serialises tests mutating the process-global `TOME_MCP_LOG` env var.
    static LOG_ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// RAII guard restoring `TOME_MCP_LOG` to its prior value on drop, so a
    /// panicking test never leaks the override into a sibling test.
    struct LogEnvGuard {
        prior: Option<String>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    impl LogEnvGuard {
        fn set(value: Option<&str>) -> Self {
            let lock = LOG_ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let prior = std::env::var(LOG_ENV).ok();
            // SAFETY: guarded by LOG_ENV_MUTEX; single-threaded within the guard.
            unsafe {
                match value {
                    Some(v) => std::env::set_var(LOG_ENV, v),
                    None => std::env::remove_var(LOG_ENV),
                }
            }
            Self { prior, _lock: lock }
        }
    }

    impl Drop for LogEnvGuard {
        fn drop(&mut self) {
            // SAFETY: guarded by the held LOG_ENV_MUTEX.
            unsafe {
                match &self.prior {
                    Some(v) => std::env::set_var(LOG_ENV, v),
                    None => std::env::remove_var(LOG_ENV),
                }
            }
        }
    }

    fn paths_in(dir: &TempDir) -> Paths {
        Paths::from_root(dir.path().to_path_buf())
    }

    // --- resolve_sink (pure) -------------------------------------------------

    #[test]
    fn resolve_sink_unset_uses_default() {
        let default = Path::new("/x/logs/mcp.log");
        assert_eq!(
            resolve_sink(default, None),
            LogSink::File(default.to_path_buf())
        );
    }

    #[test]
    fn resolve_sink_off_is_case_insensitive() {
        let default = Path::new("/x/logs/mcp.log");
        for v in ["off", "OFF", "Off", " off ", "\toff\n"] {
            assert_eq!(resolve_sink(default, Some(v)), LogSink::Off, "value {v:?}");
        }
    }

    #[test]
    fn resolve_sink_empty_and_whitespace_is_off() {
        let default = Path::new("/x/logs/mcp.log");
        assert_eq!(resolve_sink(default, Some("")), LogSink::Off);
        assert_eq!(resolve_sink(default, Some("   ")), LogSink::Off);
    }

    #[test]
    fn resolve_sink_path_is_redirected_and_trimmed() {
        let default = Path::new("/x/logs/mcp.log");
        assert_eq!(
            resolve_sink(default, Some("/tmp/custom.log")),
            LogSink::File(PathBuf::from("/tmp/custom.log"))
        );
        assert_eq!(
            resolve_sink(default, Some("  /tmp/custom.log  ")),
            LogSink::File(PathBuf::from("/tmp/custom.log"))
        );
    }

    // --- open_sink (env + filesystem) ----------------------------------------

    #[test]
    fn open_sink_unset_opens_default_path() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let _g = LogEnvGuard::set(None);

        let sink = open_sink(&paths).unwrap();
        assert!(sink.is_some(), "unset must open the default sink");
        assert!(paths.mcp_log.exists(), "default log file must be created");
    }

    #[test]
    fn open_sink_off_opens_no_file() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let _g = LogEnvGuard::set(Some("off"));

        let sink = open_sink(&paths).unwrap();
        assert!(sink.is_none(), "TOME_MCP_LOG=off must yield no sink");
        assert!(
            !paths.mcp_log.exists(),
            "off must not create the default log file"
        );
        assert!(
            !paths.logs_dir.exists(),
            "off must not create the logs dir either"
        );
    }

    #[test]
    fn open_sink_empty_opens_no_file() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let _g = LogEnvGuard::set(Some(""));

        let sink = open_sink(&paths).unwrap();
        assert!(sink.is_none(), "empty TOME_MCP_LOG must yield no sink");
        assert!(!paths.mcp_log.exists());
    }

    #[test]
    fn open_sink_path_redirects_and_leaves_default_untouched() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // Override lands in a fresh, not-yet-existing nested dir to prove
        // parent creation.
        let custom = dir.path().join("custom/nested/tome-mcp.log");
        let _g = LogEnvGuard::set(Some(custom.to_str().unwrap()));

        let sink = open_sink(&paths).unwrap();
        assert!(sink.is_some(), "override path must open a sink");
        assert!(custom.exists(), "override log file must be created");
        assert!(
            !paths.mcp_log.exists(),
            "default log must be untouched when redirected"
        );
    }

    #[cfg(unix)]
    #[test]
    fn open_sink_bad_path_degrades_to_no_sink() {
        // A path whose parent is a *file* cannot be created — this exercises
        // the fail-soft branch without needing root-only permission tricks.
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let blocker = dir.path().join("not-a-dir");
        std::fs::write(&blocker, b"i am a file").unwrap();
        let bad = blocker.join("mcp.log"); // parent is a file → create fails
        let _g = LogEnvGuard::set(Some(bad.to_str().unwrap()));

        // Fail SOFT: no panic, no error, no sink — the server would start.
        let sink = open_sink(&paths).unwrap();
        assert!(
            sink.is_none(),
            "unopenable override must degrade to no sink"
        );
        assert!(
            !paths.mcp_log.exists(),
            "degradation must NOT silently fall back to the default path"
        );
    }

    #[cfg(unix)]
    #[test]
    fn open_sink_directory_path_degrades_to_no_sink() {
        // A directory given as the log path can't be opened for append.
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let as_dir = dir.path().join("iamdir");
        std::fs::create_dir_all(&as_dir).unwrap();
        let _g = LogEnvGuard::set(Some(as_dir.to_str().unwrap()));

        let sink = open_sink(&paths).unwrap();
        assert!(
            sink.is_none(),
            "a directory-valued override must degrade to no sink"
        );
    }

    #[test]
    fn open_appender_at_rotates_and_creates_parent() {
        let dir = TempDir::new().unwrap();
        let target = dir.path().join("deep/dir/app.log");
        let _f = open_appender_at(&target).unwrap();
        assert!(target.exists());
        // Sibling rotation path is `<path>.1`.
        assert_eq!(
            rotation_sibling(&target),
            dir.path().join("deep/dir/app.log.1")
        );
    }

    #[test]
    fn rotation_sibling_matches_default_prev() {
        // The default sink's rotation sibling must equal the historical
        // `mcp_log_prev` so unset behaviour stays byte-identical.
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        assert_eq!(rotation_sibling(&paths.mcp_log), paths.mcp_log_prev);
    }

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

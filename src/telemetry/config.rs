//! On-disk telemetry config (`telemetry/config.toml`) + the enabled-state
//! precedence resolver (Phase 10, US Foundational).
//!
//! Telemetry is **opt-OUT**: absent or default config means enabled. The config
//! file is Tome-owned, so it parses strictly (`deny_unknown_fields`) — a
//! malformed file is a [`TomeError::TelemetryConfigInvalid`] (exit 91), never a
//! lenient third-party case.
//!
//! Two layers of "is telemetry on?" live here:
//! - [`load`] / [`set_enabled`] — the file itself.
//! - [`resolve_enabled`] — the full precedence (env force-on > CI auto-off >
//!   env force-off > file). This is the function the CLI status/subcommands
//!   call; the silent enqueue path ([`crate::telemetry::is_enabled`]) wraps it
//!   in a fail-safe-off shim.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::TomeError;
use crate::paths::Paths;

/// Which precedence rule decided the resolved enabled-state.
///
/// This is the structured provenance the `tome telemetry status` surface reports
/// (and the byte-stable shape the JSON pin test asserts). `snake_case` keeps the
/// wire tokens stable independent of the Rust variant spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// `TOME_TELEMETRY=1` forced telemetry ON (overrides CI + file).
    EnvOn,
    /// `TOME_TELEMETRY=0` forced telemetry OFF.
    EnvOff,
    /// A CI environment was detected ⇒ auto-OFF.
    Ci,
    /// The `telemetry/config.toml` file decided it (present and parsed).
    Config,
    /// No env override, no CI, no file ⇒ the opt-out default (ON).
    Default,
}

/// The serde default for [`TelemetryConfig::enabled`]: telemetry is opt-out, so
/// an omitted `enabled` key means ON.
fn default_true() -> bool {
    true
}

/// The Tome-owned `telemetry/config.toml` document.
///
/// `deny_unknown_fields` keeps this a strict parse — a typo'd key is a config
/// error, not silently ignored. The only field today is the opt-out switch;
/// the endpoint override is an *env* var (`TOME_TELEMETRY_ENDPOINT`), not a
/// config key, so it stays out of this struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TelemetryConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for TelemetryConfig {
    fn default() -> Self {
        // Opt-out: the absence of any config means telemetry is enabled.
        TelemetryConfig { enabled: true }
    }
}

/// Read `telemetry/config.toml`.
///
/// - Missing file ⇒ `Ok(TelemetryConfig::default())` (opt-out default-on).
/// - Present but malformed (parse error / unknown field) ⇒
///   [`TomeError::TelemetryConfigInvalid`] (exit 91), carrying the scrubbed
///   path. The path can't contain credentials, but we scrub it uniformly so
///   every telemetry surface is scrubbed by construction.
pub fn load(paths: &Paths) -> Result<TelemetryConfig, TomeError> {
    let path = paths.telemetry_config();
    let body = match crate::util::bounded_read_to_string(&path, crate::util::TOME_CONFIG_MAX) {
        Ok(s) => s,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TelemetryConfig::default());
        }
        Err(e) => return Err(e),
    };
    toml::from_str::<TelemetryConfig>(&body).map_err(|e| TomeError::TelemetryConfigInvalid {
        path: scrubbed_path(&path),
        detail: e.to_string(),
    })
}

/// Surgically set `enabled` in `telemetry/config.toml`, preserving any existing
/// comments/order, and write it back atomically with a `0600` mode.
///
/// Mirrors the `settings/edit.rs` discipline: open as a `toml_edit::DocumentMut`
/// (missing file ⇒ empty doc), mutate the single key, then route the bytes
/// through [`crate::catalog::store::write_atomic`] — which creates the
/// `telemetry/` dir, refuses a symlinked component, and chmod 0600 by default
/// (the file may eventually carry user intent and pairs with the `0600` id).
pub fn set_enabled(paths: &Paths, enabled: bool) -> Result<(), TomeError> {
    let path = paths.telemetry_config();

    // Read-modify-write through toml_edit so existing comments/order survive.
    let mut doc = match crate::util::bounded_read_to_string(&path, crate::util::TOME_CONFIG_MAX) {
        Ok(body) => body.parse::<toml_edit::DocumentMut>().map_err(|e| {
            TomeError::TelemetryConfigInvalid {
                path: scrubbed_path(&path),
                detail: e.to_string(),
            }
        })?,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            toml_edit::DocumentMut::new()
        }
        Err(e) => return Err(e),
    };

    doc["enabled"] = toml_edit::value(enabled);
    crate::catalog::store::write_atomic(&path, doc.to_string().as_bytes())
}

/// Detect a CI environment from the conventional vendor env vars (research
/// §R-14). Telemetry auto-disables under CI so build farms don't skew the
/// product-fitness signal.
///
/// `CI`/`*_CI`/`GITHUB_ACTIONS`/`TF_BUILD` are equality-checked against their
/// documented truthy token; `JENKINS_URL`/`TEAMCITY_VERSION` are presence
/// markers (any non-empty value).
pub fn is_ci() -> bool {
    fn eq(name: &str, want: &str) -> bool {
        std::env::var(name).map(|v| v == want).unwrap_or(false)
    }
    fn present(name: &str) -> bool {
        std::env::var_os(name).is_some_and(|v| !v.is_empty())
    }

    eq("CI", "true")
        || eq("GITHUB_ACTIONS", "true")
        || eq("GITLAB_CI", "true")
        || eq("CIRCLECI", "true")
        || eq("BUILDKITE", "true")
        || present("JENKINS_URL")
        || eq("TF_BUILD", "True")
        || present("TEAMCITY_VERSION")
}

/// The full enabled-state precedence, returning the deciding [`Source`].
///
/// This is the SSOT every caller routes through: [`resolve_enabled`] (the silent
/// gate's backing) delegates to it dropping the source, and `tome telemetry
/// status` keeps the source for its report. Precedence:
///
/// 1. `TOME_TELEMETRY == "1"` ⇒ `(true, EnvOn)` — overrides CI + file.
/// 2. CI detected ⇒ `(false, Ci)` — build farms never emit.
/// 3. `TOME_TELEMETRY == "0"` ⇒ `(false, EnvOff)` — explicit force-off.
/// 4. file present ⇒ `(load(..).enabled, Config)`; absent ⇒ `(true, Default)`.
///
/// A malformed config only surfaces (as exit 91) when step 4 reads a present
/// file: an explicit env override or CI short-circuits before any file read.
/// The present-vs-absent split is decided BEFORE `load` (which collapses a
/// missing file to the opt-out default) so the source faithfully distinguishes
/// `Config` from `Default`.
pub fn resolve_enabled_with_source(paths: &Paths) -> Result<(bool, Source), TomeError> {
    let force = std::env::var("TOME_TELEMETRY").ok();

    // 1. Explicit force-on wins over everything (incl. CI + a disabled file).
    if force.as_deref() == Some("1") {
        return Ok((true, Source::EnvOn));
    }

    // 2. CI auto-off. Placed above force-off only matters when the two
    //    disagree, which they can't (both yield `false`); the ordering is kept
    //    explicit so the documented precedence reads top-to-bottom.
    if is_ci() {
        return Ok((false, Source::Ci));
    }

    // 3. Explicit force-off.
    if force.as_deref() == Some("0") {
        return Ok((false, Source::EnvOff));
    }

    // 4. Fall back to the file. Probe presence FIRST so we can report `Config`
    //    vs `Default` truthfully — `load` itself folds a missing file into the
    //    opt-out default and would erase that distinction. The presence probe
    //    is cheap and racy-but-benign: a file appearing/disappearing between the
    //    probe and the read only mislabels the source, never the enabled bool
    //    (which `load` re-derives authoritatively).
    if paths.telemetry_config().exists() {
        // Present: `load` may surface a malformed-config error (exit 91).
        Ok((load(paths)?.enabled, Source::Config))
    } else {
        // Absent: opt-out default-on, no file read.
        Ok((true, Source::Default))
    }
}

/// The full enabled-state precedence (the function the CLI surfaces call).
///
/// A thin wrapper over [`resolve_enabled_with_source`] that drops the deciding
/// [`Source`] — kept so existing callers (the silent gate, the `notice`/config
/// tests) read unchanged.
pub fn resolve_enabled(paths: &Paths) -> Result<bool, TomeError> {
    resolve_enabled_with_source(paths).map(|(enabled, _)| enabled)
}

/// Scrub a path for inclusion in a telemetry error/log surface. A filesystem
/// path can't carry URL credentials, but routing it through the shared scrubber
/// keeps "every telemetry-facing string is scrubbed" true by construction.
fn scrubbed_path(path: &Path) -> std::path::PathBuf {
    let bytes = path.to_string_lossy();
    let scrubbed = crate::catalog::git::scrub_credentials(bytes.as_bytes());
    std::path::PathBuf::from(String::from_utf8_lossy(&scrubbed).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Serialises the env-mutating tests in this module. `cargo test` runs
    /// tests in a module on multiple threads; the `TOME_TELEMETRY` / CI vars
    /// are process-global, so concurrent mutation would race. Mirrors the
    /// `ENV_MUTEX` idiom used by the integration suites.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    /// Snapshot + clear every env var the resolver consults, restore on drop.
    /// Holds `ENV_MUTEX` for its lifetime so the restore can't interleave with
    /// another test's set.
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    const TELEMETRY_ENV_VARS: &[&str] = &[
        "TOME_TELEMETRY",
        "CI",
        "GITHUB_ACTIONS",
        "GITLAB_CI",
        "CIRCLECI",
        "BUILDKITE",
        "JENKINS_URL",
        "TF_BUILD",
        "TEAMCITY_VERSION",
    ];

    impl EnvGuard {
        fn new() -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let saved = TELEMETRY_ENV_VARS
                .iter()
                .map(|&k| (k, std::env::var_os(k)))
                .collect::<Vec<_>>();
            // SAFETY: we hold ENV_MUTEX for the lifetime of the guard, so no
            // other test in this module mutates these vars concurrently.
            for &k in TELEMETRY_ENV_VARS {
                unsafe { std::env::remove_var(k) };
            }
            EnvGuard { _lock: lock, saved }
        }

        fn set(&self, key: &str, val: &str) {
            // SAFETY: guarded by ENV_MUTEX (held via `_lock`).
            unsafe { std::env::set_var(key, val) };
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: still holding ENV_MUTEX (dropped after this).
            for (k, v) in &self.saved {
                match v {
                    Some(val) => unsafe { std::env::set_var(k, val) },
                    None => unsafe { std::env::remove_var(k) },
                }
            }
        }
    }

    fn paths_in(dir: &TempDir) -> Paths {
        Paths::from_root(dir.path().to_path_buf())
    }

    fn write_config(dir: &TempDir, body: &str) {
        let paths = paths_in(dir);
        std::fs::create_dir_all(paths.telemetry_dir()).unwrap();
        std::fs::write(paths.telemetry_config(), body).unwrap();
    }

    #[test]
    fn default_is_enabled_opt_out() {
        assert!(TelemetryConfig::default().enabled);
    }

    #[test]
    fn load_missing_file_is_default_on() {
        let dir = TempDir::new().unwrap();
        let cfg = load(&paths_in(&dir)).unwrap();
        assert_eq!(cfg, TelemetryConfig::default());
        assert!(cfg.enabled);
    }

    #[test]
    fn load_enabled_false_file() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "enabled = false\n");
        assert!(!load(&paths_in(&dir)).unwrap().enabled);
    }

    #[test]
    fn load_malformed_file_is_exit_91() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "enabled = \"not a bool\"\n");
        let err = load(&paths_in(&dir)).unwrap_err();
        assert!(matches!(err, TomeError::TelemetryConfigInvalid { .. }));
        assert_eq!(err.exit_code(), 91);
    }

    #[test]
    fn load_unknown_field_is_exit_91() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "enabled = true\nendpoint = \"x\"\n");
        let err = load(&paths_in(&dir)).unwrap_err();
        assert!(matches!(err, TomeError::TelemetryConfigInvalid { .. }));
        assert_eq!(err.exit_code(), 91);
    }

    #[test]
    fn set_enabled_round_trips() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);

        set_enabled(&paths, false).unwrap();
        assert!(!load(&paths).unwrap().enabled);

        set_enabled(&paths, true).unwrap();
        assert!(load(&paths).unwrap().enabled);
    }

    #[test]
    fn set_enabled_preserves_comments() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "# keep me\nenabled = true\n");
        let paths = paths_in(&dir);
        set_enabled(&paths, false).unwrap();
        let body = std::fs::read_to_string(paths.telemetry_config()).unwrap();
        assert!(body.contains("# keep me"), "comment must survive: {body}");
        assert!(body.contains("enabled = false"));
    }

    #[cfg(unix)]
    #[test]
    fn set_enabled_writes_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        set_enabled(&paths, false).unwrap();
        let mode = std::fs::metadata(paths.telemetry_config())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // --- precedence matrix (env-mutating, serialised via EnvGuard) ---

    #[test]
    fn force_on_beats_ci_and_disabled_file() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "enabled = false\n");
        let g = EnvGuard::new();
        g.set("TOME_TELEMETRY", "1");
        g.set("CI", "true");
        assert!(resolve_enabled(&paths_in(&dir)).unwrap());
    }

    #[test]
    fn ci_beats_absent_force_off_and_config() {
        let dir = TempDir::new().unwrap();
        // Config says ON; CI must still force OFF.
        write_config(&dir, "enabled = true\n");
        let g = EnvGuard::new();
        g.set("GITHUB_ACTIONS", "true");
        assert!(!resolve_enabled(&paths_in(&dir)).unwrap());
    }

    #[test]
    fn force_off_beats_config() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "enabled = true\n");
        let g = EnvGuard::new();
        g.set("TOME_TELEMETRY", "0");
        assert!(!resolve_enabled(&paths_in(&dir)).unwrap());
    }

    #[test]
    fn no_file_resolves_on() {
        let dir = TempDir::new().unwrap();
        let _g = EnvGuard::new();
        assert!(resolve_enabled(&paths_in(&dir)).unwrap());
    }

    #[test]
    fn disabled_file_resolves_off() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "enabled = false\n");
        let _g = EnvGuard::new();
        assert!(!resolve_enabled(&paths_in(&dir)).unwrap());
    }

    #[test]
    fn malformed_config_surfaces_91_when_reached() {
        let dir = TempDir::new().unwrap();
        write_config(&dir, "enabled = 123\n");
        let _g = EnvGuard::new();
        // No env override, no CI ⇒ we reach the file ⇒ 91 surfaces.
        let err = resolve_enabled(&paths_in(&dir)).unwrap_err();
        assert_eq!(err.exit_code(), 91);
    }

    #[test]
    fn is_ci_detects_each_vendor() {
        let cases: &[(&str, &str)] = &[
            ("CI", "true"),
            ("GITHUB_ACTIONS", "true"),
            ("GITLAB_CI", "true"),
            ("CIRCLECI", "true"),
            ("BUILDKITE", "true"),
            ("JENKINS_URL", "http://ci.local/"),
            ("TF_BUILD", "True"),
            ("TEAMCITY_VERSION", "2024.1"),
        ];
        for (key, val) in cases {
            let g = EnvGuard::new();
            assert!(!is_ci(), "clean env must not be CI");
            g.set(key, val);
            assert!(is_ci(), "{key}={val} should be detected as CI");
            drop(g);
        }
    }
}

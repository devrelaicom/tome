//! Telemetry opt-out resolver — reads the `[telemetry]` section of the unified
//! `~/.tome/config.toml` (Task 3 of the unified-global-config fold).
//!
//! The opt-out config knob moved from the old `telemetry/config.toml` into the
//! unified `config.toml [telemetry] enabled` key (Task 3). The other 7
//! `telemetry/*` runtime files (id, queue, locks, stamps) are unchanged.
//!
//! Telemetry is **opt-OUT**: absent or default config means enabled. A malformed
//! `config.toml` surfaces as `ManifestInvalid::TomlParse` (exit 5) — consistent
//! with the unified config policy established in Task 1/2.
//!
//! Two layers of "is telemetry on?" live here:
//! - [`set_enabled`] — surgically edits `config.toml [telemetry] enabled`.
//! - [`resolve_enabled`] — the full precedence (env force-on > CI auto-off >
//!   env force-off > config.toml). This is the function the CLI status/subcommands
//!   call; the silent enqueue path ([`crate::telemetry::is_enabled`]) wraps it
//!   in a fail-safe-off shim.

use serde::Serialize;

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
    /// The `config.toml [telemetry]` section decided it (present and parsed).
    Config,
    /// No env override, no CI, no file ⇒ the opt-out default (ON).
    Default,
}

/// Surgically set `[telemetry] enabled` in `~/.tome/config.toml`, preserving
/// any existing comments/order, and write it back atomically with `0600` mode.
///
/// Mirrors the `settings/edit.rs` discipline: open as a `toml_edit::DocumentMut`
/// (missing file ⇒ empty doc), mutate the single key, then route the bytes
/// through [`crate::catalog::store::write_atomic`] (atomic, symlink-refusing,
/// 0600).
///
/// A malformed `config.toml` maps to `ManifestInvalid::TomlParse` (exit 5),
/// consistent with the unified config policy. A hand-edited non-table
/// `[telemetry]` key returns a clean error rather than panicking (the
/// `as_table_mut().ok_or_else(...)` pattern, not `.expect()`).
pub fn set_enabled(paths: &Paths, enabled: bool) -> Result<(), TomeError> {
    let path = &paths.global_config_file;

    // Read-modify-write through toml_edit so existing comments/order survive.
    let mut doc = match crate::util::bounded_read_to_string(path, crate::util::TOME_CONFIG_MAX) {
        Ok(body) => body.parse::<toml_edit::DocumentMut>().map_err(|e| {
            TomeError::ManifestInvalid(crate::error::ManifestInvalid::TomlParse {
                file: path.clone(),
                message: e.to_string(),
            })
        })?,
        Err(TomeError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            toml_edit::DocumentMut::new()
        }
        Err(e) => return Err(e),
    };

    // Ensure `[telemetry]` is a table — use a let-else so a hand-edited
    // non-table `telemetry` key returns a clean error rather than panicking
    // under `panic = "abort"`.
    let tbl = doc
        .entry("telemetry")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()))
        .as_table_mut()
        .ok_or_else(|| {
            TomeError::ManifestInvalid(crate::error::ManifestInvalid::TomlParse {
                file: path.clone(),
                message: "[telemetry] is not a TOML table".to_string(),
            })
        })?;

    tbl["enabled"] = toml_edit::value(enabled);
    crate::catalog::store::write_atomic(path, doc.to_string().as_bytes())
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
/// 4. `config.toml [telemetry] enabled` present ⇒ `(value, Config)`;
///    absent/default ⇒ `(true, Default)`.
///
/// A malformed `config.toml` only surfaces (as exit 5) when step 4 is reached:
/// an explicit env override or CI short-circuits before any file read.
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

    // 4. Fall back to config.toml [telemetry]. Present `[telemetry] enabled`
    //    ⇒ Source::Config; absent ⇒ opt-out default-on (Source::Default).
    let cfg = crate::config::load(paths)?;
    match cfg.telemetry.enabled {
        Some(enabled) => Ok((enabled, Source::Config)),
        None => Ok((true, Source::Default)),
    }
}

/// The `[telemetry].enabled` config value (default ON). Defensive: a malformed
/// config reads as ON here; the kernel's other consent inputs (env/CI/global)
/// still apply on top. Strict config errors surface on the foreground path via
/// the normal [`resolve_enabled_with_source`] (used by the `tome telemetry`
/// surface), unchanged.
///
/// This is the single bool [`crate::telemetry::init`] hands the kernel builder —
/// the kernel folds env/CI/global itself, so we never double-gate.
pub fn config_enabled_value(paths: &crate::paths::Paths) -> bool {
    crate::config::load_or_default(paths)
        .telemetry
        .enabled
        .unwrap_or(true)
}

/// The pinned default Gauge collector endpoint.
pub const DEFAULT_ENDPOINT: &str = "https://gauge-telemetry.fly.dev";

/// Resolve the telemetry endpoint: `TOME_GAUGE_ENDPOINT` env >
/// `[telemetry].endpoint` config > the pinned default. Defensive (best-effort
/// config load): a malformed config falls through to the default.
pub fn resolve_endpoint(paths: &crate::paths::Paths) -> String {
    if let Ok(v) = std::env::var("TOME_GAUGE_ENDPOINT") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            return v;
        }
    }
    let cfg = crate::config::load_or_default(paths);
    if let Some(ep) = cfg.telemetry.endpoint.as_deref() {
        let ep = ep.trim();
        if !ep.is_empty() {
            return ep.to_string();
        }
    }
    DEFAULT_ENDPOINT.to_string()
}

/// The full enabled-state precedence (the function the CLI surfaces call).
///
/// A thin wrapper over [`resolve_enabled_with_source`] that drops the deciding
/// [`Source`] — kept so existing callers (the silent gate, the `notice`/config
/// tests) read unchanged.
pub fn resolve_enabled(paths: &Paths) -> Result<bool, TomeError> {
    resolve_enabled_with_source(paths).map(|(enabled, _)| enabled)
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

    /// Write `[telemetry]\nenabled = <bool>` to `config.toml` (the unified config).
    fn write_telemetry(dir: &TempDir, enabled: bool) {
        let paths = paths_in(dir);
        std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
        std::fs::write(
            &paths.global_config_file,
            format!("[telemetry]\nenabled = {enabled}\n"),
        )
        .unwrap();
    }

    // --- config.toml-backed resolver tests ---

    #[test]
    fn disabled_in_config_resolves_off() {
        let dir = TempDir::new().unwrap();
        write_telemetry(&dir, false);
        let _g = EnvGuard::new();
        assert!(!resolve_enabled(&paths_in(&dir)).unwrap());
    }

    #[test]
    fn enabled_in_config_resolves_on() {
        let dir = TempDir::new().unwrap();
        write_telemetry(&dir, true);
        let _g = EnvGuard::new();
        assert!(resolve_enabled(&paths_in(&dir)).unwrap());
    }

    #[test]
    fn no_config_resolves_on() {
        let dir = TempDir::new().unwrap();
        let _g = EnvGuard::new();
        assert!(resolve_enabled(&paths_in(&dir)).unwrap());
    }

    #[test]
    fn no_telemetry_section_resolves_on() {
        // A config.toml with other sections but no [telemetry] section.
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
        std::fs::write(&paths.global_config_file, "[query]\ntop_k = 5\n").unwrap();
        let _g = EnvGuard::new();
        // No [telemetry] section → enabled = None → Source::Default → true.
        assert!(resolve_enabled(&paths).unwrap());
    }

    #[test]
    fn set_enabled_round_trips() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let _g = EnvGuard::new();

        set_enabled(&paths, false).unwrap();
        assert!(!resolve_enabled(&paths).unwrap());

        set_enabled(&paths, true).unwrap();
        assert!(resolve_enabled(&paths).unwrap());
    }

    #[test]
    fn set_enabled_preserves_comments() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
        std::fs::write(
            &paths.global_config_file,
            "# keep me\n[telemetry]\nenabled = true\n",
        )
        .unwrap();
        set_enabled(&paths, false).unwrap();
        let body = std::fs::read_to_string(&paths.global_config_file).unwrap();
        assert!(body.contains("# keep me"), "comment must survive: {body}");
        assert!(
            body.contains("[telemetry]"),
            "section header must survive: {body}"
        );
        assert!(body.contains("enabled = false"));
    }

    #[test]
    fn set_enabled_creates_config_file_when_absent() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        // No config.toml exists yet.
        assert!(!paths.global_config_file.exists());
        set_enabled(&paths, false).unwrap();
        // Now config.toml exists and contains the telemetry section.
        let body = std::fs::read_to_string(&paths.global_config_file).unwrap();
        assert!(body.contains("[telemetry]"));
        assert!(body.contains("enabled = false"));
    }

    #[cfg(unix)]
    #[test]
    fn set_enabled_writes_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let _g = EnvGuard::new();
        set_enabled(&paths, false).unwrap();
        let mode = std::fs::metadata(&paths.global_config_file)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    // --- precedence matrix (env-mutating, serialised via EnvGuard) ---

    #[test]
    fn force_on_beats_ci_and_disabled_file() {
        let dir = TempDir::new().unwrap();
        write_telemetry(&dir, false);
        let g = EnvGuard::new();
        g.set("TOME_TELEMETRY", "1");
        g.set("CI", "true");
        assert!(resolve_enabled(&paths_in(&dir)).unwrap());
    }

    #[test]
    fn ci_beats_absent_force_off_and_config() {
        let dir = TempDir::new().unwrap();
        // Config says ON; CI must still force OFF.
        write_telemetry(&dir, true);
        let g = EnvGuard::new();
        g.set("GITHUB_ACTIONS", "true");
        assert!(!resolve_enabled(&paths_in(&dir)).unwrap());
    }

    #[test]
    fn force_off_beats_config() {
        let dir = TempDir::new().unwrap();
        write_telemetry(&dir, true);
        let g = EnvGuard::new();
        g.set("TOME_TELEMETRY", "0");
        assert!(!resolve_enabled(&paths_in(&dir)).unwrap());
    }

    #[test]
    fn disabled_file_resolves_off() {
        let dir = TempDir::new().unwrap();
        write_telemetry(&dir, false);
        let _g = EnvGuard::new();
        assert!(!resolve_enabled(&paths_in(&dir)).unwrap());
    }

    #[test]
    fn malformed_config_surfaces_exit_5_when_reached() {
        let dir = TempDir::new().unwrap();
        // Write a malformed config.toml that the strict parser will reject.
        let paths = paths_in(&dir);
        std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
        std::fs::write(&paths.global_config_file, "[telemetry]\nenabled = 123\n").unwrap();
        let _g = EnvGuard::new();
        // No env override, no CI ⇒ we reach the file ⇒ exit 5 (ManifestInvalid).
        let err = resolve_enabled(&paths).unwrap_err();
        assert_eq!(err.exit_code(), 5);
    }

    #[test]
    fn resolve_endpoint_prefers_env_then_config_then_default() {
        let _serial = crate::telemetry::test_serial();
        let dir = tempfile::TempDir::new().unwrap();
        let paths = crate::paths::Paths::from_root(dir.path().to_path_buf());

        // Default when nothing set — clear any ambient TOME_GAUGE_ENDPOINT.
        let g = EnvGuard::new();
        // EnvGuard::new() only clears telemetry CI vars; clear our key separately.
        // SAFETY: ENV_MUTEX held via the EnvGuard above.
        unsafe { std::env::remove_var("TOME_GAUGE_ENDPOINT") };
        assert_eq!(resolve_endpoint(&paths), "https://gauge-telemetry.fly.dev");

        // Env wins.
        // SAFETY: ENV_MUTEX held via the EnvGuard above.
        unsafe { std::env::set_var("TOME_GAUGE_ENDPOINT", "https://example.test/") };
        assert_eq!(resolve_endpoint(&paths), "https://example.test/");

        // Config tier: env absent ⇒ the `[telemetry].endpoint` value wins over
        // the default. Write the config and clear the env (we hold ENV_MUTEX).
        std::fs::create_dir_all(paths.global_config_file.parent().unwrap()).unwrap();
        std::fs::write(
            &paths.global_config_file,
            "[telemetry]\nendpoint = \"https://config.test/\"\n",
        )
        .unwrap();
        // SAFETY: ENV_MUTEX held via the EnvGuard above.
        unsafe { std::env::remove_var("TOME_GAUGE_ENDPOINT") };
        assert_eq!(resolve_endpoint(&paths), "https://config.test/");

        // Restore to clean state (EnvGuard::drop restores CI vars; clear ours).
        // SAFETY: ENV_MUTEX held via the EnvGuard above.
        unsafe { std::env::remove_var("TOME_GAUGE_ENDPOINT") };
        drop(g);
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

//! The first-run telemetry opt-out notice (FR-013/014/015).
//!
//! CLI-only, printed to **stderr exactly once** — on the very first run that
//! mints the install id. The "exactly once" guarantee is NOT a separate marker
//! file: it is the O_EXCL mint in [`super::identity::ensure_install_id`]. Only
//! the run that wins the atomic create reports `just_minted = true`, so tying
//! the notice to that bool is structurally once-per-install.
//!
//! The wording is deliberately forward-looking ("will be seen on the next run",
//! not "already sent"): at the moment we print, nothing has been emitted yet
//! (FR-015 — no over-claim). The MCP surface passes `surface_is_cli = false` and
//! stays silent (a server has no stderr a human reads).

use crate::paths::Paths;
use crate::telemetry::identity;

/// Print the one-line opt-out notice to stderr.
///
/// Plain text only (no color) — it goes to stderr regardless of `NO_COLOR`/TTY,
/// so it never pollutes `--json` stdout and never needs styling. Kept to a
/// single sentence pair so it is unobtrusive.
pub fn print_first_run_notice() {
    // stderr, not stdout: a `--json` consumer parses stdout, and this must not
    // appear there. Best-effort — a failed write to stderr is not actionable.
    eprintln!(
        "Tome collects anonymous usage telemetry to improve the project. \
         It is opt-out: run `tome telemetry off` to disable (or set TOME_TELEMETRY=0) \
         before the next run. See TELEMETRY.md."
    );
}

/// Print the first-run notice IF this run just minted the install id.
///
/// Behaviour:
/// - telemetry disabled (opt-out / CI / force-off) ⇒ do NOTHING: no id is
///   minted, no notice is printed (FR-010 — a disabled install leaves no trace).
/// - not the CLI surface (`surface_is_cli = false`, i.e. MCP) ⇒ silent.
/// - enabled + CLI ⇒ ensure the install id; print the notice ONLY when this call
///   minted it (the once-per-install guarantee).
///
/// This is the single entry point `main.rs` calls once per CLI run. It swallows
/// any id-ensure error: a failure to mint must never break the foreground
/// command — we simply skip the notice (and the next run retries the mint).
pub fn first_run_notice_if_needed(paths: &Paths, surface_is_cli: bool) {
    if !surface_is_cli {
        return;
    }
    if !crate::telemetry::is_enabled() {
        return;
    }
    match identity::ensure_install_id(paths) {
        Ok((_uuid, just_minted)) => {
            if just_minted {
                print_first_run_notice();
            }
        }
        Err(e) => {
            // Never break the foreground command on a telemetry id failure.
            tracing::debug!(error = %e, "telemetry first-run notice skipped: id mint failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// `TOME_TELEMETRY` / CI vars are process-global; serialise the env-mutating
    /// tests here. Mirrors `config.rs`'s `ENV_MUTEX` idiom.
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

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

    /// Snapshot + clear every telemetry env var, restore on drop. Also clears
    /// `HOME`-independent state by operating on an explicit `Paths` root.
    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
    }

    impl EnvGuard {
        fn new() -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let saved = TELEMETRY_ENV_VARS
                .iter()
                .map(|&k| (k, std::env::var_os(k)))
                .collect::<Vec<_>>();
            // SAFETY: ENV_MUTEX is held for the guard's lifetime.
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
            for (k, v) in &self.saved {
                match v {
                    // SAFETY: still holding ENV_MUTEX.
                    Some(val) => unsafe { std::env::set_var(k, val) },
                    None => unsafe { std::env::remove_var(k) },
                }
            }
        }
    }

    fn paths_in(dir: &TempDir) -> Paths {
        Paths::from_root(dir.path().to_path_buf())
    }

    #[test]
    fn enabled_cli_mints_once_then_does_not_re_mint() {
        // `is_enabled()` resolves the DEFAULT (`$HOME`) Paths, so we force-on via
        // env to make the gate true regardless of the developer's real config.
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let g = EnvGuard::new();
        g.set("TOME_TELEMETRY", "1");

        // First call: enabled + CLI ⇒ mints the id (and would print the notice).
        first_run_notice_if_needed(&paths, true);
        assert!(paths.telemetry_id().exists(), "first CLI run mints the id");
        let (_, minted_again) = identity::ensure_install_id(&paths).unwrap();
        assert!(!minted_again, "the id already exists after the first call");

        // Second call is a no-op mint-wise (already exists ⇒ just_minted=false).
        first_run_notice_if_needed(&paths, true);
        assert!(paths.telemetry_id().exists());
    }

    #[test]
    fn mcp_surface_never_mints() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let g = EnvGuard::new();
        g.set("TOME_TELEMETRY", "1"); // even force-on...

        first_run_notice_if_needed(&paths, false); // ...MCP stays silent.
        assert!(
            !paths.telemetry_id().exists(),
            "MCP surface must not mint an id"
        );
    }

    #[test]
    fn disabled_never_mints() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let g = EnvGuard::new();
        g.set("TOME_TELEMETRY", "0"); // force-off.

        first_run_notice_if_needed(&paths, true);
        assert!(
            !paths.telemetry_id().exists(),
            "disabled telemetry mints no id and prints no notice"
        );
    }

    #[test]
    fn ci_never_mints() {
        let dir = TempDir::new().unwrap();
        let paths = paths_in(&dir);
        let g = EnvGuard::new();
        g.set("CI", "true"); // CI auto-off.

        first_run_notice_if_needed(&paths, true);
        assert!(
            !paths.telemetry_id().exists(),
            "CI auto-off mints no id and prints no notice"
        );
    }
}

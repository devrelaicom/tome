//! Shared launcher resolution + ownership-recognition for the Tome binary that
//! every harness sink invokes (`tome` issue #290 / #337).
//!
//! ## Why a shared module
//!
//! Several harness sinks emit a command that the *host* (a CI runner, a
//! sandboxed non-IDE agent, an editor extension) later executes:
//!
//! - the standard MCP-config writer (`mcp_config` + `sync`) — the `command`
//!   field of the `tome` server entry,
//! - the Open Plugins `tome-op` bundle (`open_plugins`) — its `.mcp.json`
//!   server command + its SessionStart shell hook,
//! - the Claude/Codex session-start hooks + the new-harness `CommandHook`
//!   session-start + the `run-hook` dispatcher (`routing` / `reconcile::hooks`).
//!
//! On a host whose `PATH` does not contain `tome`, a bare `"tome"` silently
//! fails to start and the agent gets zero skills. #290 fixed the `tome-op`
//! bundle by resolving an ABSOLUTE launcher ([`tome_command`]); this module
//! promotes that resolver (and its companion [`shell_quote`]) to the SSOT so
//! every sink shares ONE implementation rather than re-deriving it.
//!
//! ## The ownership tension (#337)
//!
//! For the `tome-op` bundle, ownership is keyed on the bundle's `plugin.json`
//! `name`, so the launcher can vary freely. For the **standard MCP sink**, the
//! bare command string `"tome"` was itself the load-bearing ownership marker
//! (`is_tome_owned` compared `command == "tome"`). Emitting an absolute
//! launcher there would break idempotence, clash classification (exit 19), and
//! removal. [`looks_like_tome_launcher`] is the launcher-tolerant recogniser
//! that lets the emitted command become an absolute path while still being
//! recognised as Tome's own: it accepts a command whose *file name* is `tome`
//! (so `/usr/local/bin/tome`, `/opt/tome/bin/tome`, and the bare `tome` all
//! match) — paired at every call site with the Tome arg-shape check
//! (`args[0] == "mcp"`) so a genuine user/foreign entry is never claimed.
//!
//! Sync-only — `tests/sync_boundary.rs` guards this tree.

use std::path::Path;

/// Env var overriding the launcher every sink invokes (`TOME_BIN`). When set
/// and non-empty it wins the [`tome_command`] resolution; otherwise the running
/// binary's absolute path (`current_exe`) is used, falling back to the bare
/// `"tome"` name. The override is also the deterministic test seam for the byte
/// pins (since `current_exe` is machine-specific).
pub const TOME_BIN_ENV: &str = "TOME_BIN";

/// The bare launcher name + the recognised launcher BASENAME (#337). A command
/// whose final path component equals this is recognised as Tome's own launcher
/// regardless of the leading directory.
pub const TOME_BIN_NAME: &str = "tome";

/// Resolve the absolute launcher every harness sink should invoke (#290).
///
/// The emitted command is executed by the *host* (a CI runner or a sandboxed
/// non-IDE agent), whose `PATH` need not contain `tome`. A bare `"tome"`
/// therefore silently fails to start the MCP server / hook and the agent gets
/// zero skills. Resolution order:
///
/// 1. `$TOME_BIN`, if set and non-empty — an explicit operator override (and the
///    deterministic test seam, since `current_exe` is machine-specific). It MUST
///    be an ABSOLUTE path: the value is used verbatim, NOT shell-expanded, so a
///    leading `~` is treated literally (the host will not find `~/…/tome`).
/// 2. [`std::env::current_exe`] — the absolute path of the running binary, so the
///    emitted command points at the exact `tome` that ran the sync.
/// 3. The bare name `"tome"` — the old behaviour, used only when both above
///    fail (an exotic platform / a deleted binary). Never panics, never errors
///    the sync: this resolver is infallible by design.
///
/// The tiers are tried in order and each falls through INDEPENDENTLY:
/// - A non-empty but non-UTF-8 `$TOME_BIN` is IGNORED and resolution continues
///   at tier 2 (`current_exe`) — it does NOT short-circuit to the bare fallback
///   (we cannot embed a non-UTF-8 value in JSON / a shell command cleanly).
/// - A `current_exe` that fails to resolve, or whose path is not valid UTF-8,
///   falls through to tier 3 (the bare name).
pub fn tome_command() -> String {
    // (1) Explicit override wins.
    if let Some(value) = std::env::var_os(TOME_BIN_ENV)
        && !value.is_empty()
        && let Some(s) = value.to_str()
    {
        return s.to_string();
    }

    // (2) The running binary's absolute path. UTF-8-fail and `current_exe`-fail
    //     both fall through to the bare name.
    if let Ok(exe) = std::env::current_exe()
        && let Some(s) = exe.to_str()
    {
        return s.to_string();
    }

    // (3) Last-resort fallback: the old bare-PATH behaviour.
    TOME_BIN_NAME.to_string()
}

/// Launcher-tolerant ownership recogniser (#337).
///
/// Returns `true` when `command` is a reference to the Tome binary — the bare
/// name `"tome"`, OR any path whose final component is `tome`
/// (`/usr/local/bin/tome`, `/opt/tome/bin/tome`, `C:\\tools\\tome.exe` on
/// Windows, …). The launcher is free to vary per machine (the #290 resolver
/// returns `current_exe`), so the *string* can no longer be the marker;
/// instead the BASENAME is.
///
/// **Self-recognition arm.** A command is ALSO recognised when it equals the
/// launcher THIS binary would emit right now ([`tome_command`]). In production
/// every install path (`cargo install`, Homebrew, `cargo run`) yields a binary
/// literally named `tome`, so the basename arm already covers it; the
/// self-recognition arm additionally covers a renamed / wrapped binary (and the
/// integration-test binary, whose `current_exe` basename is hash-named, not
/// `tome`) so a sync ALWAYS recognises its OWN just-written entry on the next
/// pass — without that arm a non-`tome`-named binary would treat its own entry
/// as a foreign clash. It compares against the resolver, NOT against the
/// historical bare name, so it tracks `$TOME_BIN` / `current_exe` exactly.
///
/// This arm is per-PROCESS: it recognises an entry whose command equals the
/// resolved launcher of the running binary. Cross-process recognition is NOT a
/// cache property — a later process re-resolves its own `current_exe` / `$TOME_BIN`
/// (see [`current_launcher`]); two processes at the SAME path recognise each
/// other's entries because they resolve to the same string, two at DIFFERENT
/// paths fall back to the basename arm (production: both are `tome`).
///
/// This is deliberately NOT a sufficient ownership check on its own: a user
/// could have an unrelated entry whose command happens to be named `tome`.
/// Callers MUST pair it with the Tome arg-shape check (`args[0] == "mcp"` for
/// the MCP sink) so a genuine foreign entry is never claimed. See
/// [`crate::harness::mcp_config::is_tome_owned`], which does exactly that.
///
/// A `tome.exe` final component (Windows) also matches: the file *stem* is
/// compared so the platform executable suffix does not defeat recognition.
pub fn looks_like_tome_launcher(command: &str) -> bool {
    if command == TOME_BIN_NAME {
        return true;
    }
    // Self-recognition: the exact launcher this binary would emit now. Covers
    // a renamed/wrapped binary (and the hash-named integration-test binary) so
    // a sync recognises its own just-written entry regardless of basename.
    if command == current_launcher() {
        return true;
    }
    // Use the path file name (final component). `Path::file_name` strips any
    // directory prefix; comparing the *stem* additionally tolerates a `.exe`
    // suffix on Windows. An empty / dir-only command (e.g. `"/"`) has no file
    // name → not a launcher.
    let p = Path::new(command);
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name == TOME_BIN_NAME {
        return true;
    }
    // Tolerate an executable suffix (Windows `tome.exe`): compare the stem.
    p.file_stem().and_then(|s| s.to_str()) == Some(TOME_BIN_NAME)
}

/// The launcher this binary resolves to, cached for the process lifetime.
///
/// [`tome_command`] reads `$TOME_BIN` / `current_exe`; both are stable for a
/// running process, so resolving ONCE is correct and avoids a syscall on every
/// ownership check. `OnceLock` makes the cache lazily-initialised and
/// thread-safe with no `unsafe`. (Process-lifetime caching is acceptable: a
/// long-running process does not change its own executable path mid-run; the
/// MCP server freezes other launch-time state similarly.)
///
/// CAVEAT: the `OnceLock` FREEZES the resolution at the FIRST call for the rest
/// of the process — it is NOT coordinated with the `ENV_MUTEX` the `$TOME_BIN`
/// test seam uses. In production this is fine (`$TOME_BIN` / `current_exe` are
/// immutable for the process). But a TEST that mutates `$TOME_BIN` mid-process
/// MUST NOT rely on the self-recognition arm reflecting that change: once the
/// cache is set, a later `$TOME_BIN` value is ignored here. Tests that need a
/// specific launcher should drive `tome_command()` directly (it re-reads the
/// env every call), or assert via the basename arm with a `tome`-named path.
fn current_launcher() -> &'static str {
    use std::sync::OnceLock;
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE.get_or_init(tome_command)
}

/// Quote a launcher path for safe interpolation into a single shell-command
/// string (e.g. the `tome-op` SessionStart hook's `"<cmd> harness …"`). An
/// absolute `current_exe` path can contain spaces (e.g. macOS `Application
/// Support`), which would otherwise split into multiple shell words. POSIX
/// single-quoting wraps the path and escapes any embedded single quote via the
/// `'\''` idiom. The bare name `"tome"` (no shell-special chars) is returned
/// unquoted so the fallback string stays identical to the historical bytes.
///
/// NOTE: the standard MCP-config `command` field is the execve-style sink (the
/// host runs it directly, NOT through a shell), so it must NOT be quoted — it
/// receives the raw [`tome_command`] value. Only sinks that interpolate the
/// launcher into a single shell string use this.
pub fn shell_quote(cmd: &str) -> String {
    if !cmd.is_empty() && cmd.bytes().all(is_shell_safe_byte) {
        return cmd.to_string();
    }
    format!("'{}'", cmd.replace('\'', "'\\''"))
}

/// Bytes that need no shell quoting (a conservative POSIX-portable set).
fn is_shell_safe_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialises every test that mutates `TOME_BIN` (process-global; `cargo
    /// test` runs a module's tests on multiple threads). Mirrors the `ENV_MUTEX`
    /// idiom used across the codebase (see `open_plugins`, `provider::config`).
    static ENV_MUTEX: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Option<std::ffi::OsString>,
    }

    impl EnvGuard {
        fn new() -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let saved = std::env::var_os(TOME_BIN_ENV);
            // SAFETY: ENV_MUTEX held for the guard's lifetime.
            unsafe { std::env::remove_var(TOME_BIN_ENV) };
            EnvGuard { _lock: lock, saved }
        }
        fn set(&self, val: &str) {
            // SAFETY: guarded by ENV_MUTEX (held via `_lock`).
            unsafe { std::env::set_var(TOME_BIN_ENV, val) };
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: still holding ENV_MUTEX.
            match &self.saved {
                Some(v) => unsafe { std::env::set_var(TOME_BIN_ENV, v) },
                None => unsafe { std::env::remove_var(TOME_BIN_ENV) },
            }
        }
    }

    // ---- tome_command resolution (promoted from open_plugins #290) -------

    #[test]
    fn tome_command_honors_tome_bin_override() {
        let guard = EnvGuard::new();
        guard.set("/custom/path/to/tome");
        assert_eq!(tome_command(), "/custom/path/to/tome");
    }

    #[test]
    fn tome_command_falls_back_to_current_exe_when_override_unset() {
        let _guard = EnvGuard::new();
        let cmd = tome_command();
        let exe = std::env::current_exe().expect("current_exe");
        assert_eq!(cmd, exe.to_str().expect("test binary path is UTF-8"));
        assert_ne!(cmd, "tome", "the launcher must not be the bare name");
        assert!(Path::new(&cmd).is_absolute());
    }

    #[test]
    fn tome_command_ignores_empty_override() {
        let guard = EnvGuard::new();
        guard.set("");
        let cmd = tome_command();
        assert!(!cmd.is_empty());
    }

    // ---- looks_like_tome_launcher (#337) --------------------------------

    #[test]
    fn recognises_bare_name() {
        assert!(looks_like_tome_launcher("tome"));
    }

    #[test]
    fn recognises_absolute_paths() {
        assert!(looks_like_tome_launcher("/usr/local/bin/tome"));
        assert!(looks_like_tome_launcher("/opt/tome/bin/tome"));
        assert!(looks_like_tome_launcher("/Users/dev/.cargo/bin/tome"));
        // current_exe-style (a temp test-binary path) ending in `tome` matches.
        assert!(looks_like_tome_launcher("/tmp/build/target/release/tome"));
    }

    #[test]
    fn recognises_windows_exe_suffix() {
        assert!(looks_like_tome_launcher("tome.exe"));
        assert!(looks_like_tome_launcher("/c/tools/tome.exe"));
    }

    #[test]
    fn does_not_over_broaden_to_foreign_commands() {
        // The recurring "don't claim a foreign entry" guard: a command that is
        // NOT named tome must never be recognised, even if `tome` appears as a
        // directory component or substring.
        assert!(!looks_like_tome_launcher("not-tome"));
        assert!(!looks_like_tome_launcher("tome-wrapper"));
        assert!(!looks_like_tome_launcher("/opt/tome/bin/other"));
        assert!(!looks_like_tome_launcher("/usr/bin/tomestone"));
        assert!(!looks_like_tome_launcher("mytome"));
        assert!(!looks_like_tome_launcher(""));
        assert!(!looks_like_tome_launcher("/"));
        // NOTE: `/usr/local/tome/` (a trailing-slash directory path) DOES have
        // `Path::file_name() == "tome"`, so it matches — but a `command` field
        // is always an executable, never a bare directory, so this is not a real
        // over-broadening risk and is deliberately not asserted against.
    }

    // ---- shell_quote (promoted from open_plugins #290) ------------------

    #[test]
    fn shell_quote_leaves_simple_paths_unquoted() {
        assert_eq!(shell_quote("tome"), "tome");
        assert_eq!(shell_quote("/usr/local/bin/tome"), "/usr/local/bin/tome");
    }

    #[test]
    fn shell_quote_wraps_paths_with_spaces() {
        assert_eq!(
            shell_quote("/Applications/My Tome.app/tome"),
            "'/Applications/My Tome.app/tome'",
        );
    }

    #[test]
    fn shell_quote_escapes_embedded_single_quote() {
        assert_eq!(shell_quote("/o'dd/tome"), "'/o'\\''dd/tome'");
    }
}

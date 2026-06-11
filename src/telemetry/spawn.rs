//! The detached flusher spawn (Phase 10, US3 — R-4/FR-047a).
//!
//! [`spawn_detached_flusher`] forks `tome telemetry flush --quiet` as a fully
//! detached child and does NOT wait on it: the parent's foreground command exits
//! immediately while the child reparents to init and drains the queue best-effort
//! in the background. This is the ONE side of telemetry delivery that crosses a
//! process boundary on the CLI — and it crosses it WITHOUT blocking (NFR-001).
//!
//! Three properties make the child safe to abandon:
//! - **new session** (`setsid` in `pre_exec`): a terminal SIGINT delivered to the
//!   parent's process group AFTER the parent exits can't also hit the flusher —
//!   the flusher leads its own session/group (R-4/FR-047a).
//! - **null stdio**: the child inherits nothing and writes nothing to the parent's
//!   terminal (the `--quiet` child also suppresses its own output).
//! - **no `wait()`**: dropping the `Child` handle leaves the child reparented to
//!   init; the parent never reaps it.
//!
//! Best-effort throughout: a failure to resolve the target binary or to spawn is
//! a `debug!` + return — it NEVER breaks the parent's exit (FR-046a).

/// Spawn `tome telemetry flush --quiet` as a detached, session-leading child and
/// return immediately without waiting.
///
/// FR-046a fail-closed: the target binary is [`std::env::current_exe`]. If that
/// can't be resolved we return WITHOUT spawning — we never guess a path or
/// `to_string_lossy` an un-resolvable one into an executed command (a wrong/
/// attacker-controlled path is far worse than a skipped flush).
///
/// Returns `Ok(())` on a successful spawn (or a deliberate no-op skip); an
/// `Err` only carries a spawn failure the caller logs and ignores. On non-Unix
/// this is a no-op (the support matrix is macOS + Linux).
#[cfg(unix)]
pub fn spawn_detached_flusher() -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};

    // FR-046a: resolve the running binary. A failure here means we cannot name a
    // trustworthy executable — skip rather than guess.
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "flusher spawn skipped: current_exe unresolvable");
            return Ok(());
        }
    };

    let mut cmd = Command::new(exe);
    cmd.args(["telemetry", "flush", "--quiet"])
        // Inherit/emit nothing: the child is fully detached from the terminal.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    // SAFETY: `setsid()` is async-signal-safe and touches no shared state of the
    // (about-to-exec) child, so it is sound to call in `pre_exec` between fork and
    // exec. It puts the child in a NEW session + process group so a terminal
    // SIGINT aimed at the parent's group after the parent exits can't reach the
    // flusher (R-4/FR-047a).
    unsafe {
        cmd.pre_exec(|| {
            rustix::process::setsid()
                .map(|_| ())
                .map_err(|e| std::io::Error::from_raw_os_error(e.raw_os_error()))
        });
    }

    // Spawn and DROP the handle: the child reparents to init; we never `wait()`.
    // A spawn failure is best-effort — log + return, never break the parent's exit.
    match cmd.spawn() {
        Ok(_child) => Ok(()),
        Err(e) => {
            tracing::debug!(target: "telemetry", error = %e, "flusher spawn failed (best-effort)");
            Ok(())
        }
    }
}

/// No-op on non-Unix: the support matrix is macOS + Linux, and `setsid`/detached
/// session semantics are Unix-only.
#[cfg(not(unix))]
pub fn spawn_detached_flusher() -> std::io::Result<()> {
    Ok(())
}

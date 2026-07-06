//! Integration tests for `tome init` (issue #418).
//!
//! 1. non-TTY refusal → exit 54 (`NotATerminal`) with the pointer message
//!    naming every equivalent manual command;
//! 2. `--json` rejection → usage error (exit 2, `TomeError::Usage`);
//! 3. a scripted pty session (the `plugin_interactive.rs` harness pattern —
//!    `inquire` has no test backend) driving the skip path through every
//!    step on a fresh install: Esc skips the bind step, an empty submit
//!    skips the catalog step, the plugin step self-skips with no catalogs,
//!    and the run closes with the status panel + remaining steps and
//!    exits 0 (never the status health code).
//!
//! The planning half (`init::plan` over `InitState`) is covered by unit
//! tests in `src/commands/init.rs`.

use crate::common::ToolEnv;

#[test]
fn init_without_a_terminal_exits_54_and_names_the_manual_commands() {
    // Plain `Command::output()` — no pty, so stdin/stdout are not terminals
    // and the refusal fires before any prompt or disk state is touched.
    let env = ToolEnv::new();

    let out = env
        .cmd()
        .arg("init")
        .env("NO_COLOR", "1")
        .output()
        .expect("spawn tome init");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(54),
        "expected exit 54 (not_a_terminal), got {:?}; stderr: {stderr}",
        out.status.code(),
    );
    assert!(
        stderr.contains("requires a terminal"),
        "stderr should describe the TTY refusal; got: {stderr}",
    );
    // The message must point at the full equivalent manual flow
    // (catalog add → plugin enable → harness use → query).
    for needle in [
        "tome catalog add",
        "tome plugin enable",
        "tome harness use",
        "tome query",
    ] {
        assert!(
            stderr.contains(needle),
            "stderr should point at `{needle}`; got: {stderr}",
        );
    }
    assert!(
        out.stdout.is_empty(),
        "expected empty stdout on non-TTY refusal; got {:?}",
        String::from_utf8_lossy(&out.stdout),
    );
}

#[test]
fn init_wizard_skips_every_step_and_exits_zero() {
    use rexpect::process::WaitStatus;
    use rexpect::session::{PtySession, spawn_command};

    /// `rexpect`'s `send` does not flush; single-byte writes (Esc, Enter)
    /// can otherwise sit in the buffer indefinitely.
    fn send_flush(sess: &mut PtySession, bytes: &str) {
        sess.send(bytes).expect("write to pty");
        sess.flush().expect("flush pty");
    }

    let env = ToolEnv::new();
    // A fresh project dir with no `.tome/` marker in its ancestry, so the
    // scope resolves to the global fallback and the bind step is offered.
    let project = tempfile::TempDir::new().unwrap();

    let mut cmd = std::process::Command::new(env!("CARGO_BIN_EXE_tome"));
    cmd.arg("init")
        .current_dir(project.path())
        .env("HOME", env.home_path())
        // This pty spawn bypasses `ToolEnv::cmd`, so force-disable telemetry
        // explicitly — no detached flusher should fork on exit (#225).
        .env("TOME_TELEMETRY", "0")
        .env("XDG_CONFIG_HOME", env.home_path().join(".config"))
        .env("XDG_DATA_HOME", env.home_path().join(".local/share"))
        .env("NO_COLOR", "1")
        .env_remove("TOME_LOG")
        .env_remove("RUST_LOG")
        .env_remove("TOME_WORKSPACE")
        .env_remove("TOME_NONINTERACTIVE");

    let mut sess = spawn_command(cmd, Some(30_000)).expect("spawn tome init under pty");

    // Header: the resolved scope. Fresh temp HOME ⇒ global fallback, and no
    // harness detects ⇒ the plan is [bind, catalog, plugins].
    sess.exp_string("Workspace: `global`").expect("header");

    // Step 1 — bind. Esc = skip (the wizard's cancel-means-skip contract).
    sess.exp_string("bind this directory to a workspace")
        .expect("bind step banner");
    sess.exp_string("Bind it to a workspace?")
        .expect("bind prompt");
    send_flush(&mut sess, "\x1b");
    sess.exp_string("Skipped")
        .expect("bind step skipped on Esc");

    // Step 2 — catalog. Empty submit = skip.
    sess.exp_string("add a plugin catalog")
        .expect("catalog step banner");
    sess.exp_string("Catalog source").expect("catalog prompt");
    send_flush(&mut sess, "\r");
    sess.exp_string("Skipped")
        .expect("catalog step skipped on empty");

    // Step 3 — plugins. No catalogs enrolled ⇒ the step skips itself.
    sess.exp_string("enable plugins")
        .expect("plugin step banner");
    sess.exp_string("No catalogs enrolled")
        .expect("plugin step self-skips without catalogs");

    // Close: the status panel renders (models missing on a fresh install is
    // fine — the wizard never adopts the status health exit code), then the
    // outstanding manual commands.
    sess.exp_string("Remaining steps:")
        .expect("remaining steps");
    sess.exp_string("tome catalog add")
        .expect("catalog next step");

    sess.exp_eof().expect("clean EOF after wizard");
    let status = sess.process().wait().expect("collect child status");
    assert!(
        matches!(status, WaitStatus::Exited(_, 0)),
        "expected exit 0 after an all-skipped wizard run; got {status:?}",
    );
}

#[test]
fn init_refuses_json_mode_with_usage_error() {
    // `tome init --json` is a usage error (exit 2) — the wizard is
    // interactive-only and must not half-emit structured output. Checked
    // before the TTY gate, so it holds in this non-TTY harness too.
    let env = ToolEnv::new();

    let out = env
        .cmd()
        .args(["init", "--json"])
        .output()
        .expect("spawn tome init --json");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit 2 (usage), got {:?}; stderr: {stderr}",
        out.status.code(),
    );
    assert!(
        stderr.contains("--json"),
        "stderr should explain the --json refusal; got: {stderr}",
    );
}

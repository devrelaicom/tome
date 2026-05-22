//! Integration tests for the bare `tome plugin` interactive browse flow.
//!
//! Two scenarios:
//!
//! 1. **T101 — scripted pty session.** Pre-enable a plugin via the library
//!    API (the CLI's enable path loads `FastembedEmbedder`; not testable in
//!    CI), then drive the interactive flow via a real pty to disable the
//!    plugin and navigate back out. Asserts the plugin ends up disabled in
//!    the index DB and that the process exited 0.
//!
//!    The contract task wording says "select catalog → select plugin → enable
//!    → back → back → quit; assert the plugin ends up enabled". We invert the
//!    direction (start-enabled → disable) because the enable verb inside the
//!    interactive flow delegates to `commands::plugin::enable::run` which
//!    constructs `FastembedEmbedder` — that requires ~345 MB of real ONNX
//!    model files and is the same reason the existing `plugin_enable.rs`
//!    suite drives the lifecycle via the library API rather than the CLI
//!    binary. The disable verb hits `lifecycle::disable` which does not
//!    load the embedder, so it is exerciseable from the CLI binary.
//!
//!    Tested transitions: catalog selector → plugin browser → plugin view →
//!    action prompt → confirm prompt → action prompt (after redraw) → plugin
//!    browser (Back) → catalog selector (Back) → exit (Quit). That covers
//!    every level's Back, Quit, and the post-action redraw path. Test name
//!    deliberately names the inversion so future readers don't get tripped
//!    up by the contract wording.
//!
//! 2. **T102 — non-TTY refusal.** Plain `Command::new(...).output()`. No pty
//!    is attached, so `output::stdin_is_tty() && stdout_is_tty()` is false
//!    and the flow exits 54 with the documented pointer message.
//!
//! Spec: `contracts/plugin-commands.md` §"`tome plugin` (no subcommand —
//! interactive)"; FR-050 / FR-051.

mod common;

use std::process::Command;
use std::time::Duration;

use common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_models, paths_for,
    stub_embedder_seed, stub_reranker_seed, write_config_for_cli,
};
use rexpect::session::{PtySession, spawn_command};
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::paths::Paths;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

/// Common setup: isolated env, sample-plugin-catalog copied in, config.toml
/// written, fabricated model manifests, and `plugin-alpha` pre-enabled via
/// the library API. Returns the env and fixture dir so callers can keep
/// them alive for the test's lifetime.
fn setup_pre_enabled(catalog_name: &str) -> (ToolEnv, TempDir, Paths) {
    let env = ToolEnv::new();
    let fixture_tmp = TempDir::new().unwrap();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&fixture_tmp, "catalog");
    let config = config_with_catalog(catalog_name, &catalog_root);
    write_config_for_cli(&paths, &config);

    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &tome::workspace::Scope::Global,
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        allow_model_download: false,
    };
    let id: PluginId = format!("{catalog_name}/plugin-alpha").parse().unwrap();
    lifecycle::enable(&id, &deps).expect("pre-enable plugin-alpha");

    (env, fixture_tmp, paths)
}

/// Send `bytes` to the pty and flush. `rexpect::PtySession::send` does NOT
/// flush — without an explicit flush, single-byte writes (Enter, arrow
/// keys) can sit in the buffer indefinitely and the child reads nothing.
fn send_flush(sess: &mut PtySession, bytes: &str) {
    sess.send(bytes).expect("write to pty");
    sess.flush().expect("flush pty");
}

/// Press Enter on the slave terminal — `0x0D` (carriage return) is what
/// crossterm decodes as `KeyCode::Enter` under a Unix pty in raw mode.
fn press_enter(sess: &mut PtySession) {
    send_flush(sess, "\r");
}

/// Press Down arrow — ANSI `ESC [ B`.
fn press_down(sess: &mut PtySession) {
    send_flush(sess, "\x1b[B");
}

/// Query the index DB for plugin-alpha's enabled state. Returns `(total
/// rows, enabled rows)`.
fn read_alpha_enabled_state(paths: &Paths) -> (i64, i64) {
    use tome::index::{self, OpenOptions};
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
        },
    )
    .expect("open index DB");
    conn.query_row(
        "SELECT COUNT(*), COALESCE(SUM(enabled), 0)
         FROM skills
         WHERE catalog = 'sample-plugin-catalog' AND plugin = 'plugin-alpha'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("query skills aggregate")
}

#[test]
fn interactive_disable_via_scripted_session_exits_zero_and_flips_state() {
    let catalog_name = "sample-plugin-catalog";
    let (env, _fixture_tmp, paths) = setup_pre_enabled(catalog_name);

    // Sanity: pre-state should be all-enabled.
    let (total_before, enabled_before) = read_alpha_enabled_state(&paths);
    assert!(
        total_before > 0 && enabled_before > 0 && enabled_before == total_before,
        "expected plugin-alpha fully enabled before drive; got {total_before}/{enabled_before}",
    );

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tome"));
    cmd.arg("plugin")
        .env("HOME", env.home_path())
        .env("XDG_CONFIG_HOME", env.home_path().join(".config"))
        .env("XDG_DATA_HOME", env.home_path().join(".local/share"))
        // Reduce ANSI noise so `exp_string` matches the prompt copy
        // reliably. inquire still emits some cursor positioning under
        // `NO_COLOR`, but the prompt text itself appears verbatim.
        .env("NO_COLOR", "1")
        .env_remove("TOME_LOG")
        .env_remove("RUST_LOG");

    let mut sess = spawn_command(cmd, Some(30_000)).expect("spawn tome plugin under pty");

    // Level 1 — catalog selector. One catalog + Quit; cursor on the catalog.
    sess.exp_string("Pick a catalog")
        .expect("catalog selector prompt");
    press_enter(&mut sess);

    // Level 2 — plugin browser. plugin-alpha + plugin-beta + Back; cursor
    // on the alphabetically-first plugin.
    sess.exp_string("Pick a plugin")
        .expect("plugin browser prompt");
    press_enter(&mut sess);

    // Level 3 — plugin view + action prompt. Status is Enabled, so the
    // action menu offers [Disable, Back] with cursor on Disable.
    sess.exp_string("Plugin:").expect("plugin view header");
    sess.exp_string("Action").expect("action prompt");
    press_enter(&mut sess);

    // Confirm prompt — default "no". Send 'y' + Enter to confirm.
    sess.exp_string("Disable sample-plugin-catalog/plugin-alpha?")
        .expect("disable confirm prompt");
    send_flush(&mut sess, "y\r");

    // Disable line, view redraw, fresh action prompt — menu is now
    // [Enable, Back] (status flipped to Disabled). Cursor on Enable.
    // We want Back to climb out without invoking the embedder.
    sess.exp_string("disabled sample-plugin-catalog/plugin-alpha")
        .expect("disable confirmation line");
    sess.exp_string("Action")
        .expect("redrawn action prompt after disable");
    press_down(&mut sess);
    press_enter(&mut sess);

    // Plugin browser again. Two plugins + Back; cursor at top. Two Downs to
    // highlight Back, then Enter.
    sess.exp_string("Pick a plugin")
        .expect("plugin browser re-rendered after view-loop Back");
    press_down(&mut sess);
    press_down(&mut sess);
    press_enter(&mut sess);

    // Catalog selector again. One catalog + Quit; cursor on catalog. Down
    // to highlight Quit, then Enter.
    sess.exp_string("Pick a catalog")
        .expect("catalog selector re-rendered after plugin-loop Back");
    press_down(&mut sess);
    press_enter(&mut sess);

    // Wait for the process to exit. rexpect 0.7's `process()` accessor
    // returns a `&mut PtyProcess` whose `wait()` returns a `WaitStatus`.
    sess.exp_eof().expect("clean EOF after Quit");
    let status = sess.process().wait().expect("collect child status");
    // WaitStatus::Exited(_, 0) is the only acceptable outcome.
    use rexpect::process::WaitStatus;
    assert!(
        matches!(status, WaitStatus::Exited(_, 0)),
        "expected exit 0 on clean Quit; got {status:?}",
    );

    // Post-state: plugin-alpha is fully disabled in the index.
    let (total_after, enabled_after) = read_alpha_enabled_state(&paths);
    assert_eq!(total_after, total_before, "row count must be unchanged");
    assert_eq!(
        enabled_after, 0,
        "every plugin-alpha skill row must have enabled=0 after interactive disable",
    );
}

#[test]
fn bare_plugin_without_a_terminal_exits_54_with_pointer_message() {
    // Pure `Command::new` — no pty. stdin/stdout aren't terminals, so the
    // FR-051 refusal fires before any prompt is constructed.
    let env = ToolEnv::new();
    // Working config not required for this path: the TTY check happens
    // before any disk reads. But construct one anyway so we don't depend
    // on which error fires first if that ever changes.
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();

    let out = env
        .cmd()
        .arg("plugin")
        // Belt-and-braces — don't let a real TTY in the developer's
        // environment leak in.
        .env("NO_COLOR", "1")
        .output()
        .expect("spawn tome plugin");

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(54),
        "expected exit 54, got {:?}; stderr: {stderr}",
        out.status.code(),
    );
    assert!(
        stderr.contains("requires a terminal"),
        "stderr should describe the TTY refusal; got: {stderr}",
    );
    // The contract's specific pointer message — flagged in retro/P4.md as
    // a brittle duplicated string. If the wording changes, update this
    // assertion together with the writeln! in
    // `src/commands/plugin/interactive.rs`.
    assert!(
        stderr.contains("tome plugin list") && stderr.contains("tome plugin show"),
        "stderr should point at the non-interactive subcommands; got: {stderr}",
    );

    // Sanity: stdout is empty — interactive flow only writes to stdout
    // inside the loops.
    assert!(
        out.stdout.is_empty(),
        "expected empty stdout on non-TTY refusal; got {:?}",
        String::from_utf8_lossy(&out.stdout),
    );

    // Sanity: the failure is bounded — the harness shouldn't sit waiting
    // for stdin. Spawn one more time with a hard timeout to be safe.
    let started = std::time::Instant::now();
    let _ = env.cmd().arg("plugin").output();
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "non-TTY tome plugin should exit promptly; took {:?}",
        started.elapsed(),
    );
}

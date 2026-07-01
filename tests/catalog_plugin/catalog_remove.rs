//! `tome catalog remove` integration tests.

use crate::common::{Fixture, ToolEnv, has_global_enrolment, paths_for};
use serde_json::Value;
use std::process::Stdio;

#[test]
fn force_happy_path_human_mode() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    let out = env
        .cmd()
        .args(["catalog", "remove", "sample-experts", "--force"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Removed catalog `sample-experts`"));
    // Enrolment no longer present in workspace_catalogs.
    let paths = paths_for(&env);
    assert!(
        !has_global_enrolment(&paths, "sample-experts"),
        "enrolment should be removed",
    );
}

#[test]
fn force_happy_path_json_mode() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    let out = env
        .cmd()
        .args(["catalog", "remove", "sample-experts", "--force", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["removed"]["name"], "sample-experts");
    assert!(v["removed"]["cache_path"].is_string());
}

#[test]
fn non_tty_without_force_exits_2() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    // stdin redirected to /dev/null is non-TTY; output()'s default piping
    // also yields non-TTY stdin.
    let out = env
        .cmd()
        .args(["catalog", "remove", "sample-experts"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--force"), "{}", stderr);
}

#[test]
fn unregistered_name_exits_3() {
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["catalog", "remove", "nope", "--force"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
}

/// Issue #305 — the global `--non-interactive` flag suppresses the
/// confirmation prompt, so a non-TTY `catalog remove` succeeds without the
/// per-command `--force` (baseline: `non_tty_without_force_exits_2`).
#[test]
fn non_interactive_flag_suppresses_prompt() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    let out = env
        .cmd()
        .args(["catalog", "remove", "sample-experts", "--non-interactive"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "--non-interactive should skip the prompt; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let paths = paths_for(&env);
    assert!(
        !has_global_enrolment(&paths, "sample-experts"),
        "enrolment should be removed",
    );
}

/// Issue #305 — `TOME_NONINTERACTIVE=1` does the same as `--non-interactive`.
/// Env vars are process-global, but each spawned `tome` gets its own env map
/// via `ToolEnv::cmd().env(...)`, so no in-process serialization is needed.
#[test]
fn tome_noninteractive_env_var_suppresses_prompt() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    let out = env
        .cmd()
        .env("TOME_NONINTERACTIVE", "1")
        .args(["catalog", "remove", "sample-experts"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "TOME_NONINTERACTIVE=1 should skip the prompt; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let paths = paths_for(&env);
    assert!(
        !has_global_enrolment(&paths, "sample-experts"),
        "enrolment should be removed",
    );
}

/// Issue #305 — a falsey `TOME_NONINTERACTIVE=0` must NOT suppress the prompt;
/// the non-TTY refusal (exit 2) still applies. Guards the truthy-token parse.
#[test]
fn tome_noninteractive_env_var_falsey_still_refuses() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    let out = env
        .cmd()
        .env("TOME_NONINTERACTIVE", "0")
        .args(["catalog", "remove", "sample-experts"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(2),
        "falsey env value must not suppress the prompt; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Issue #305 — `--yes` is a hidden alias on `catalog remove` (which only
/// documented `--force`), so both non-interactive spellings work everywhere.
#[test]
fn yes_alias_is_accepted() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    let out = env
        .cmd()
        .args(["catalog", "remove", "sample-experts", "--yes"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "--yes alias should behave like --force; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let paths = paths_for(&env);
    assert!(
        !has_global_enrolment(&paths, "sample-experts"),
        "enrolment should be removed",
    );
}

#[test]
fn cache_already_missing_still_succeeds() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    // Pre-emptively delete the cache directory. F11b: derive it from
    // the enrolment URL via paths.cache_dir_for.
    let paths = paths_for(&env);
    let url = crate::common::global_enrolment_url(&paths, "sample-experts").expect("enrolment");
    let cache_path = paths.cache_dir_for(&url);
    std::fs::remove_dir_all(&cache_path).unwrap();

    let out = env
        .cmd()
        .args(["catalog", "remove", "sample-experts", "--force"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

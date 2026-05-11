//! `tome catalog remove` integration tests.

mod common;

use common::{Fixture, ToolEnv};
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
    // Registry no longer contains the entry.
    let cfg = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(!cfg.contains("[catalogs.sample-experts]"), "{}", cfg);
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

#[test]
fn cache_already_missing_still_succeeds() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    // Pre-emptively delete the cache directory.
    let cfg = std::fs::read_to_string(env.config_file()).unwrap();
    let path_line = cfg
        .lines()
        .find(|l| l.trim_start().starts_with("path = "))
        .unwrap();
    let cache_path = path_line.split('"').nth(1).unwrap();
    std::fs::remove_dir_all(cache_path).unwrap();

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

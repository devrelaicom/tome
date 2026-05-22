//! `tome catalog show` integration tests.

mod common;

use common::{Fixture, ToolEnv, global_enrolment_url, paths_for};
use serde_json::Value;

#[test]
fn happy_path_json_returns_full_manifest() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    let out = env
        .cmd()
        .args(["catalog", "show", "sample-experts", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["name"], "sample-experts");
    assert_eq!(v["version"], "0.1.0");
    assert_eq!(v["owner"]["email"], "tests@tome.invalid");
    assert_eq!(v["registered"]["url"], fix.url);
    assert_eq!(v["registered"]["ref"], "main");
    assert!(v["registered"]["last_synced"].is_string());
    assert_eq!(v["plugins"].as_array().unwrap().len(), 2);
    assert_eq!(v["plugins"][0]["name"], "midnight-compact-expert");
}

#[test]
fn unregistered_name_exits_3() {
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["catalog", "show", "nope"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn human_mode_shows_metadata_block() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    let out = env
        .cmd()
        .args(["catalog", "show", "sample-experts"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("sample-experts (v0.1.0)"), "{}", stdout);
    assert!(stdout.contains("Owner: Tome Test Harness"), "{}", stdout);
    assert!(stdout.contains("Plugins:"), "{}", stdout);
    assert!(stdout.contains("midnight-compact-expert"), "{}", stdout);
}

#[test]
fn cache_manifest_deleted_returns_io_error() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    // Locate the cache dir via the enrolment URL.
    let paths = paths_for(&env);
    let url = global_enrolment_url(&paths, "sample-experts").expect("enrolment");
    let cache_dir = paths.cache_dir_for(&url);
    let manifest = cache_dir.join("tome-catalog.toml");
    std::fs::remove_file(&manifest).expect("rm manifest");

    let out = env
        .cmd()
        .args(["catalog", "show", "sample-experts"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(7),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

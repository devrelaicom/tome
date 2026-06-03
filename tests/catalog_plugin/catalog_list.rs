//! `tome catalog list` integration tests.

use crate::common::{Fixture, ToolEnv};
use serde_json::Value;

#[test]
fn zero_catalogs_human_mode_prints_hint() {
    let env = ToolEnv::new();
    let out = env.cmd().args(["catalog", "list"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("No catalogs registered"),
        "stdout: {}",
        stdout
    );
}

#[test]
fn zero_catalogs_json_mode_prints_nothing() {
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["catalog", "list", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty(), "stdout: {:?}", out.stdout);
}

#[test]
fn one_catalog_shows_in_table() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    let out = env.cmd().args(["catalog", "list"]).output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("NAME"));
    assert!(stdout.contains("sample-experts"));
    assert!(stdout.contains("main"), "stdout: {}", stdout);
}

#[test]
fn two_catalogs_listed_alphabetically() {
    let fix1 = Fixture::build_sample();
    let fix2 = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix1.url, "--name", "zeta"])
        .output()
        .unwrap();
    env.cmd()
        .args(["catalog", "add", &fix2.url, "--name", "alpha"])
        .output()
        .unwrap();

    let out = env
        .cmd()
        .args(["catalog", "list", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let lines: Vec<&[u8]> = out
        .stdout
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(
        lines.len(),
        2,
        "stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let v0: Value = serde_json::from_slice(lines[0]).unwrap();
    let v1: Value = serde_json::from_slice(lines[1]).unwrap();
    assert_eq!(v0["name"], "alpha");
    assert_eq!(v1["name"], "zeta");
}

#[test]
fn json_record_includes_documented_fields() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    let out = env
        .cmd()
        .args(["catalog", "list", "--json"])
        .output()
        .unwrap();
    let line = out.stdout.split(|&b| b == b'\n').next().unwrap();
    let v: Value = serde_json::from_slice(line).unwrap();
    for key in &["name", "url", "ref", "plugin_count", "last_synced"] {
        assert!(v.get(*key).is_some(), "missing key {} in {}", key, v);
    }
    assert_eq!(v["plugin_count"], 2);
}

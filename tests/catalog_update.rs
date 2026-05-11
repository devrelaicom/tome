//! `tome catalog update` integration tests.

mod common;

use common::{Fixture, ToolEnv};
use serde_json::Value;
use std::process::Command;

#[test]
fn single_catalog_update_is_up_to_date() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    let out = env
        .cmd()
        .args(["catalog", "update", "sample-experts"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("up-to-date") || stdout.contains("advanced 0 commit"),
        "{}",
        stdout
    );
}

#[test]
fn unregistered_name_exits_3() {
    let env = ToolEnv::new();
    let out = env
        .cmd()
        .args(["catalog", "update", "nope"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
}

#[test]
fn sha_pinned_catalog_is_a_no_op() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    // Find the upstream HEAD SHA so we can pin to it.
    let sha = Command::new("git")
        .args([
            "-C",
            &fix.repo_path.display().to_string(),
            "rev-parse",
            "HEAD",
        ])
        .output()
        .unwrap()
        .stdout;
    let sha = String::from_utf8(sha).unwrap().trim().to_string();
    // Don't pass --ref because git clone --depth 1 --branch <sha> doesn't
    // work on arbitrary SHAs. Instead we manually patch the registry to a
    // pinned ref after add.
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    let cfg = std::fs::read_to_string(env.config_file()).unwrap();
    let new_cfg = cfg.replace("ref = \"main\"", &format!("ref = \"{}\"", sha));
    std::fs::write(env.config_file(), new_cfg).unwrap();

    let out = env
        .cmd()
        .args(["catalog", "update", "sample-experts", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(v.get("pinned").is_some(), "expected `pinned`, got: {}", v);
    assert_eq!(v["pinned"]["name"], "sample-experts");
}

#[test]
fn refresh_all_runs_alphabetically() {
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
        .args(["catalog", "update", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let lines: Vec<&[u8]> = out
        .stdout
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .collect();
    assert_eq!(lines.len(), 2);
    let v0: Value = serde_json::from_slice(lines[0]).unwrap();
    let v1: Value = serde_json::from_slice(lines[1]).unwrap();
    assert_eq!(v0["refreshed"]["name"], "alpha");
    assert_eq!(v1["refreshed"]["name"], "zeta");
}

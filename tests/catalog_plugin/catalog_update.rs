//! `tome catalog update` integration tests.

use crate::common::{Fixture, ToolEnv, paths_for, set_global_enrolment_ref};
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
    let paths = paths_for(&env);
    set_global_enrolment_ref(&paths, "sample-experts", &sha);

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

/// F11b reshape of the old `refresh_all_runs_alphabetically`. The
/// refresh order is `workspace_catalogs::distinct_urls`, which selects
/// `MAX(rowid)` per URL grouping and orders by `url` (ascending). We
/// don't assert a specific order (URL-bytes ordering is an
/// implementation detail of the SQL query); we assert determinism —
/// two runs over the same enrolment state must emit the same per-URL
/// sequence.
#[test]
fn refresh_all_is_deterministic() {
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

    let collect_names = || -> Vec<String> {
        let out = env
            .cmd()
            .args(["catalog", "update", "--json"])
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "stderr: {}",
            String::from_utf8_lossy(&out.stderr),
        );
        out.stdout
            .split(|&b| b == b'\n')
            .filter(|l| !l.is_empty())
            .map(|line| {
                let v: Value = serde_json::from_slice(line).expect("valid JSON line");
                // Each NDJSON envelope is either `refreshed` (URL fetched) or
                // `pinned` (SHA-pinned no-op). We pull the display name from
                // whichever shape is present.
                if let Some(name) = v.get("refreshed").and_then(|r| r.get("name")) {
                    name.as_str().unwrap_or_default().to_owned()
                } else if let Some(name) = v.get("pinned").and_then(|r| r.get("name")) {
                    name.as_str().unwrap_or_default().to_owned()
                } else {
                    String::new()
                }
            })
            .filter(|n| !n.is_empty())
            .collect()
    };

    let first = collect_names();
    let second = collect_names();
    assert_eq!(
        first.len(),
        2,
        "expected two refresh envelopes (one per URL), got {:?}",
        first,
    );
    assert_eq!(
        first, second,
        "refresh order must be deterministic across runs",
    );
}

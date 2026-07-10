//! `tome catalog update` integration tests.

use crate::common::{
    Fixture, ToolEnv, paths_for, sample_plugin_catalog_fixture, set_global_enrolment_ref,
};
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

/// Regression test for issue #512: `catalog update` must exit 0 after fetching
/// a new upstream commit. Before the fix, the advisory lock was held across the
/// entire `refresh_one` + reindex span; `reindex_catalog_plugins` internally
/// calls `acquire_lock`, which returned `WouldBlock` → `IndexBusy` (exit 50)
/// even when no plugins were enabled (the reindex loop was skipped entirely, so
/// exit 50 would only surface with enabled plugins — see the library-path
/// counterpart in `catalog_update_reindex.rs`).
///
/// This CLI test exercises the `refresh_one` path and the lock scope around it.
/// It uses `sample-plugin-catalog` (which has real `tome-plugin.toml` entries)
/// but does NOT enable any plugins, so the reindex body is skipped and no ONNX
/// embedder is constructed. The assertion is: exit 0 after a successful git
/// fetch + reset, without any lock contention.
#[test]
fn update_after_upstream_commit_exits_0() {
    // Build an upstream git repo from the sample-plugin-catalog fixture —
    // a catalog that has real plugin manifests (unlike sample-catalog whose
    // plugin dirs contain only .keep files).
    let fix = Fixture::build_from(sample_plugin_catalog_fixture());
    let env = ToolEnv::new();

    // Enrol the catalog via the CLI binary (real git clone + DB row).
    let out = env
        .cmd()
        .args([
            "catalog",
            "add",
            &fix.url,
            "--name",
            "sample-plugin-catalog",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "catalog add failed: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // Commit a change to the upstream repo so `catalog update` actually
    // performs a fetch + reset (instead of seeing an already-up-to-date SHA).
    let upstream = &fix.repo_path;
    std::fs::write(upstream.join("VERSION"), b"0.2.0\n").unwrap();
    for args in [
        &["add", "-A"][..],
        &["commit", "-q", "-m", "bump version"][..],
    ] {
        let s = Command::new("git")
            .args(args)
            .current_dir(upstream)
            .env("GIT_AUTHOR_NAME", "Tome Test")
            .env("GIT_AUTHOR_EMAIL", "tests@tome.invalid")
            .env("GIT_COMMITTER_NAME", "Tome Test")
            .env("GIT_COMMITTER_EMAIL", "tests@tome.invalid")
            .status()
            .unwrap();
        assert!(s.success(), "git {:?} failed: {}", args, s);
    }

    // Run `catalog update` via the CLI binary. No plugins are enabled so the
    // reindex body is skipped — this path exercises the advisory-lock scope
    // fix in `update.rs` (the lock is held only during `refresh_one`, then
    // released before any lifecycle call that would re-acquire it).
    let out = env
        .cmd()
        .args(["catalog", "update", "sample-plugin-catalog"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "catalog update exited {} (expected 0) — stderr: {}",
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr),
    );
    // Confirm a `refreshed` record is present (not `up-to-date` or `pinned`).
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("advanced") || stdout.contains("Refreshed"),
        "expected a 'advanced N commit(s)' or 'Refreshed' message, got: {stdout}",
    );
}

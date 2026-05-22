//! Phase 6 / US4 — `tome doctor --json` wire-shape regression net.
//!
//! Pins the JSON envelope's field names + values per
//! `contracts/doctor.md` §"Output (`--json`)" and data-model §5. If the
//! shape drifts, this test breaks loudly — the JSON is consumed by
//! harnesses and by users piping into `jq`.

mod common;

use common::{Fixture, ToolEnv, fabricate_all_installed_models, paths_for};
use serde_json::Value;
use tempfile::TempDir;

#[test]
fn doctor_json_shape_is_pinned_on_healthy_install() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_installed_models(&paths);

    let out = env.cmd().args(["--json", "doctor"]).output().unwrap();
    assert!(out.status.success(), "exit={:?}", out.status.code());

    let v: Value = serde_json::from_slice(&out.stdout).expect("doctor --json parses");

    // Top-level fields, in any order — but every one of these must be
    // present.
    for field in [
        "tome_version",
        "workspace",
        "embedder",
        "reranker",
        "index",
        "drift",
        "catalogs",
        "workspace_registry",
        "harnesses",
        "overall",
        "suggested_fixes",
    ] {
        assert!(v.get(field).is_some(), "missing top-level field `{field}`");
    }

    // Workspace embeds the WorkspaceInfo schema verbatim.
    let ws = &v["workspace"];
    assert_eq!(ws["scope"], "global");
    assert!(ws.get("path").is_some());
    assert!(ws.get("source").is_some());

    // ModelHealth has name / version / state.
    let emb = &v["embedder"];
    for field in ["name", "version", "state"] {
        assert!(emb.get(field).is_some(), "embedder.{field} missing");
    }
    assert_eq!(emb["state"], "ok");

    // IndexHealth has present / schema_version / plugins_enabled /
    // skills_indexed / size_bytes / integrity_ok.
    let idx = &v["index"];
    for field in [
        "present",
        "schema_version",
        "plugins_enabled",
        "skills_indexed",
        "size_bytes",
        "integrity_ok",
    ] {
        assert!(idx.get(field).is_some(), "index.{field} missing");
    }

    // catalogs + harnesses are arrays.
    assert!(v["catalogs"].is_array());
    assert!(v["harnesses"].is_array());

    // Harnesses array contains every known name in fixed order; first
    // entry is claude_code per the contract example.
    let harnesses = v["harnesses"].as_array().unwrap();
    assert_eq!(harnesses.len(), 6);
    assert_eq!(harnesses[0]["name"], "claude_code");
    for h in harnesses {
        for field in ["name", "path", "present"] {
            assert!(h.get(field).is_some(), "harness.{field} missing");
        }
    }

    // overall is one of "ok" | "degraded" | "unhealthy".
    assert!(
        matches!(
            v["overall"].as_str(),
            Some("ok") | Some("degraded") | Some("unhealthy")
        ),
        "overall: {}",
        v["overall"],
    );

    // suggested_fixes is an array; on a healthy install it's empty.
    assert!(v["suggested_fixes"].is_array());
    assert_eq!(v["suggested_fixes"].as_array().unwrap().len(), 0);
}

#[test]
fn doctor_json_includes_suggested_fix_record_on_broken_catalog() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_installed_models(&paths);

    let fix = Fixture::build_sample();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    // Break the catalog cache: remove `.git/`.
    let cache_dir = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(fix.url.as_bytes());
        env.catalogs_dir().join(hex::encode(h.finalize()))
    };
    std::fs::remove_dir_all(cache_dir.join(".git")).unwrap();

    let out = env.cmd().args(["--json", "doctor"]).output().unwrap();
    // Exit 1 — degraded — but JSON still emitted on stdout.
    assert_eq!(out.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&out.stdout).expect("doctor --json parses");

    assert_eq!(v["overall"], "degraded");
    let catalogs = v["catalogs"].as_array().unwrap();
    assert_eq!(catalogs.len(), 1);
    assert_eq!(catalogs[0]["state"], "not_a_repo");
    let fixes = v["suggested_fixes"].as_array().unwrap();
    assert!(!fixes.is_empty(), "expected suggested fix");
    let cat_fix = fixes
        .iter()
        .find(|f| {
            f["subsystem"]
                .as_str()
                .map(|s| s.starts_with("catalog:"))
                .unwrap_or(false)
        })
        .expect("catalog suggested fix");
    assert_eq!(cat_fix["auto_fixable"], true);
    assert!(
        cat_fix["command"]
            .as_str()
            .unwrap()
            .contains("catalog update")
    );
}

#[test]
fn doctor_json_overall_unhealthy_when_models_missing() {
    let env = ToolEnv::new();
    // No fabricate — both models are absent.
    let out = env.cmd().args(["--json", "doctor"]).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let v: Value = serde_json::from_slice(&out.stdout).expect("doctor --json parses");
    assert_eq!(v["overall"], "unhealthy");
    assert_eq!(v["embedder"]["state"], "missing");
}

// Silence unused-import warning when no harness check uses TempDir.
#[allow(dead_code)]
fn _silence(_: TempDir) {}

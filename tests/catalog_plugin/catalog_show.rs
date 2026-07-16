//! `tome catalog show` integration tests.

use crate::common::{Fixture, ToolEnv, global_enrolment_url, paths_for};
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

    // `sample-catalog` declares none of the optional metadata fields — they
    // must stay OUT of the JSON entirely, not render as `null`.
    assert!(v.get("keywords").is_none(), "{v}");
    assert!(v.get("homepage").is_none(), "{v}");
    assert!(v.get("repository").is_none(), "{v}");
    assert!(v.get("bugs").is_none(), "{v}");
    assert!(v.get("documentation").is_none(), "{v}");
    assert!(v.get("display_name").is_none(), "{v}");
    assert!(v["owner"].get("url").is_none(), "{v}");
    assert!(v["owner"].get("handles").is_none(), "{v}");
}

#[test]
fn json_includes_present_metadata_fields() {
    let fix = Fixture::build_metadata_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    let out = env
        .cmd()
        .args(["catalog", "show", "metadata-experts", "--json"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["keywords"], serde_json::json!(["auth", "oauth"]));
    assert_eq!(v["homepage"], "https://example.com");
    assert_eq!(v["repository"], "https://github.com/x/y");
    assert_eq!(v["bugs"], "https://github.com/x/y/issues");
    assert_eq!(v["documentation"], "https://docs.example.com");
    assert_eq!(v["display_name"], "Metadata Catalog");
    assert_eq!(v["owner"]["url"], "https://jo.dev");
    assert_eq!(v["owner"]["handles"]["github"], "jodev");
}

#[test]
fn human_mode_shows_present_metadata_fields() {
    let fix = Fixture::build_metadata_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    let out = env
        .cmd()
        .args(["catalog", "show", "metadata-experts"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("Display name: Metadata Catalog"),
        "{stdout}"
    );
    assert!(stdout.contains("Homepage: https://example.com"), "{stdout}");
    assert!(
        stdout.contains("Repository: https://github.com/x/y"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Bugs: https://github.com/x/y/issues"),
        "{stdout}"
    );
    assert!(
        stdout.contains("Documentation: https://docs.example.com"),
        "{stdout}"
    );
    assert!(stdout.contains("Keywords: auth, oauth"), "{stdout}");
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

    // `sample-catalog` declares none of the optional metadata fields — none
    // of their lines should render.
    assert!(!stdout.contains("Display name:"), "{}", stdout);
    assert!(!stdout.contains("Homepage:"), "{}", stdout);
    assert!(!stdout.contains("Repository:"), "{}", stdout);
    assert!(!stdout.contains("Bugs:"), "{}", stdout);
    assert!(!stdout.contains("Documentation:"), "{}", stdout);
    assert!(!stdout.contains("Keywords:"), "{}", stdout);
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

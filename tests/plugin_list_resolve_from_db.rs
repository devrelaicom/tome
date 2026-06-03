//! FF2 regression: scoped catalog DISCOVERY commands must enumerate catalogs
//! from the `workspace_catalogs` DB enrolment, NOT `config.toml [catalogs]`.
//!
//! Companion to `plugin_resolve_from_db.rs` (FF1, which migrated the shared
//! `resolve_plugin_dir`). This file covers `tome plugin list` — both the
//! bare form (enumerate every enrolled catalog) and the `--catalog <name>`
//! filter — driven against a catalog that is enrolled ONLY in the DB, with
//! NO `config.toml` written. This is the real `tome catalog add` shape, so
//! before FF2 the bare list emitted ZERO rows (it read the empty
//! `config.catalogs` map) on a fresh install.
//!
//! Setup uses [`stage_sample_catalog_in_db`] / `tome catalog add` — never
//! `write_config_for_cli` — so the assertions are honest about the no-config
//! state a real user lands in.

mod common;

use common::{ToolEnv, paths_for, stage_sample_catalog_in_db};
use serde_json::Value;

/// Parse NDJSON stdout into a `Vec<Value>`.
fn parse_ndjson(stdout: &[u8]) -> Vec<Value> {
    stdout
        .split(|&b| b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_slice::<Value>(l).expect("parse json line"))
        .collect()
}

/// Bare `tome plugin list` must surface every plugin declared by a catalog
/// that is enrolled only in the DB (no config.toml). Before FF2 the catalog
/// iteration read `config.catalogs.keys()`, which is empty on a fresh
/// install, so zero rows were emitted.
#[test]
fn list_bare_enumerates_db_enrolled_catalog_without_config() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");

    // Guard the production invariant: no config.toml is involved.
    assert!(
        !paths.global_config_file.exists(),
        "this test must run with NO config.toml",
    );

    let out = env
        .cmd()
        .args(["plugin", "list", "--json"])
        .output()
        .expect("spawn plugin list");
    assert!(
        out.status.success(),
        "plugin list must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let records = parse_ndjson(&out.stdout);
    let plugins: std::collections::HashSet<String> = records
        .iter()
        .filter_map(|r| r["id"]["plugin"].as_str().map(str::to_owned))
        .collect();
    assert!(
        plugins.contains("plugin-alpha") && plugins.contains("plugin-beta"),
        "bare list must enumerate the DB-enrolled catalog's plugins; got {plugins:?}",
    );
    for r in &records {
        assert_eq!(r["id"]["catalog"], "sample-plugin-catalog");
    }
}

/// `tome plugin list --catalog <name>` must resolve the named catalog from
/// the DB enrolment and emit its plugins (exit 0).
#[test]
fn list_catalog_filter_resolves_from_db_without_config() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");

    let out = env
        .cmd()
        .args([
            "plugin",
            "list",
            "--catalog",
            "sample-plugin-catalog",
            "--json",
        ])
        .output()
        .expect("spawn plugin list --catalog");
    assert_eq!(
        out.status.code(),
        Some(0),
        "plugin list --catalog must resolve from the DB (exit 0); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let records = parse_ndjson(&out.stdout);
    assert!(
        !records.is_empty(),
        "filtered list must emit the catalog's plugins",
    );
    for r in &records {
        assert_eq!(r["id"]["catalog"], "sample-plugin-catalog");
    }
}

/// `--catalog <name>` for a catalog absent from the DB must still exit 3 —
/// the migration narrows the lookup to the DB; it does not weaken the
/// `CatalogNotFound` contract for genuinely-unknown catalogs.
#[test]
fn list_unknown_catalog_filter_still_exits_3_without_config() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    stage_sample_catalog_in_db(&paths, "global", "sample-plugin-catalog");

    let out = env
        .cmd()
        .args(["plugin", "list", "--catalog", "ghost-catalog", "--json"])
        .output()
        .expect("spawn plugin list --catalog ghost");
    assert_eq!(
        out.status.code(),
        Some(3),
        "unknown catalog filter must remain CatalogNotFound (exit 3); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

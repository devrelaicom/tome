//! Integration tests for `tome plugin disable <id>` against the CLI binary.
//!
//! The disable path does NOT construct `FastembedEmbedder` — `lifecycle::disable`
//! only touches the index. That makes the CLI binary safe to drive in CI
//! without real model artefacts. We pre-enable plugins via the library API
//! (with `StubEmbedder`) and then exercise the disable subcommand through
//! `Command::new(tome)`.
//!
//! Covers FR-005 (skill records retained on disable), FR-007 (non-TTY
//! requires `--force`), FR-051 (non-TTY confirmation refusal → `NotATerminal`).
//!
//! Spec: `contracts/plugin-commands.md` §"`tome plugin disable`".

mod common;

use common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_models, paths_for,
    stub_embedder_seed, stub_reranker_seed, stub_summariser_seed, write_config_for_cli,
};
use serde_json::Value;
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::index::{self, OpenOptions};
use tome::paths::Paths;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

/// End-to-end setup: register the sample-plugin-catalog in the env's config,
/// pre-enable `plugin-alpha` via the library API, return the resolved
/// `Paths`. Mirrors `plugin_show.rs::setup` but with the catalog name
/// parameterised so callers can have multiple isolated catalogs in one test
/// run.
fn setup_with_alpha_enabled(env: &ToolEnv, fixture_tmp: &TempDir) -> Paths {
    let paths = paths_for(env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(fixture_tmp, "catalog");
    let cli_config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &cli_config);

    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        config: &cli_config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("pre-enable plugin-alpha");

    paths
}

fn enabled_row_counts(paths: &Paths) -> (i64, i64) {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .unwrap();
    conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN ws.skill_id IS NOT NULL THEN 1 ELSE 0 END), 0)
         FROM skills AS s
         LEFT JOIN workspace_skills AS ws
                ON ws.skill_id = s.id
               AND ws.workspace_id = (SELECT id FROM workspaces WHERE name = 'global')
         WHERE s.catalog = ?1 AND s.plugin = ?2",
        rusqlite::params!["sample-plugin-catalog", "plugin-alpha"],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )
    .unwrap()
}

#[test]
fn disable_with_force_flips_rows_to_disabled_and_retains_count_via_json() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = setup_with_alpha_enabled(&env, &fixture_tmp);

    // Pre-condition: all 4 skill rows enabled (matches plugin_enable.rs's
    // total_skills assertion for the sample fixture).
    let (total_before, enabled_before) = enabled_row_counts(&paths);
    assert_eq!(total_before, 4);
    assert_eq!(enabled_before, 4);

    let out = env
        .cmd()
        .args([
            "plugin",
            "disable",
            "sample-plugin-catalog/plugin-alpha",
            "--force",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "exit code: {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    // JSON record contract: { plugin, status: "disabled", skills_retained }.
    let record: Value = serde_json::from_slice(&out.stdout)
        .expect("plugin disable --json must emit a single JSON record");
    assert_eq!(record["plugin"], "sample-plugin-catalog/plugin-alpha");
    assert_eq!(record["status"], "disabled");
    assert_eq!(record["skills_retained"], 4);

    // Post-condition: rows still present but every `enabled` flipped to 0.
    let (total_after, enabled_after) = enabled_row_counts(&paths);
    assert_eq!(total_after, 4, "skill records must be retained on disable");
    assert_eq!(enabled_after, 0, "all enabled flags must be zeroed");
}

#[test]
fn disable_without_force_in_non_tty_context_exits_54_with_pointer_message() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp);

    // Subprocess stdin/stdout/stderr are pipes — not TTYs. Without `--force`
    // the handler must short-circuit before the confirm prompt, emit the
    // documented pointer line to stderr, and exit with code 54.
    let out = env
        .cmd()
        .args(["plugin", "disable", "sample-plugin-catalog/plugin-alpha"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(54),
        "expected exit 54 (NotATerminal), got {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--force"),
        "stderr must point the user at --force, got: {stderr}",
    );

    // State must NOT have changed: rows still enabled.
    let (_total, enabled_after) = enabled_row_counts(&paths_for(&env));
    assert_eq!(
        enabled_after, 4,
        "refused disable must not mutate the index",
    );
}

#[test]
fn disable_already_disabled_plugin_exits_21() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_enabled(&env, &fixture_tmp);

    // First disable: succeeds.
    let first = env
        .cmd()
        .args([
            "plugin",
            "disable",
            "sample-plugin-catalog/plugin-alpha",
            "--force",
        ])
        .output()
        .unwrap();
    assert!(first.status.success(), "first disable must succeed");

    // Second disable: should exit 21 (PluginAlreadyInState) per FR-008 +
    // contracts/plugin-commands.md §"tome plugin disable" step 2.
    let second = env
        .cmd()
        .args([
            "plugin",
            "disable",
            "sample-plugin-catalog/plugin-alpha",
            "--force",
        ])
        .output()
        .unwrap();
    assert_eq!(
        second.status.code(),
        Some(21),
        "expected exit 21 (PluginAlreadyInState), got {:?}; stderr: {}",
        second.status.code(),
        String::from_utf8_lossy(&second.stderr),
    );
}

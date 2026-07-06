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

use crate::common::{
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

/// Like [`setup_with_alpha_enabled`] but pre-enables BOTH `plugin-alpha` and
/// `plugin-beta` from the sample catalog — the fixture for #314 batch / glob
/// disable tests, which need more than one enabled plugin to exercise.
fn setup_with_alpha_and_beta_enabled(env: &ToolEnv, fixture_tmp: &TempDir) -> Paths {
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
    for plugin in ["plugin-alpha", "plugin-beta"] {
        let id: PluginId = format!("sample-plugin-catalog/{plugin}").parse().unwrap();
        lifecycle::enable(&id, &deps).unwrap_or_else(|e| panic!("pre-enable {plugin}: {e:?}"));
    }

    paths
}

/// Enabled-flag sum for one `(catalog, plugin)` in the `global` workspace.
fn enabled_count_for(paths: &Paths, plugin: &str) -> i64 {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .unwrap();
    conn.query_row(
        "SELECT COALESCE(SUM(CASE WHEN ws.skill_id IS NOT NULL THEN 1 ELSE 0 END), 0)
         FROM skills AS s
         LEFT JOIN workspace_skills AS ws
                ON ws.skill_id = s.id
               AND ws.workspace_id = (SELECT id FROM workspaces WHERE name = 'global')
         WHERE s.catalog = ?1 AND s.plugin = ?2",
        rusqlite::params!["sample-plugin-catalog", plugin],
        |row| row.get::<_, i64>(0),
    )
    .unwrap()
}

fn enabled_row_counts(paths: &Paths) -> (i64, i64) {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
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
        stderr.contains("--yes"),
        "stderr must point the user at --yes (#438), got: {stderr}",
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

// ---- issue #314: variadic ids, wildcard globs, --catalog -----------------

/// A `*` glob disables every matching plugin. `sample-plugin-catalog/plugin-*`
/// spans both `plugin-alpha` and `plugin-beta`; one glob token disables both.
#[test]
fn disable_glob_expands_to_multiple_plugins() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = setup_with_alpha_and_beta_enabled(&env, &fixture_tmp);

    assert!(enabled_count_for(&paths, "plugin-alpha") > 0);
    assert!(enabled_count_for(&paths, "plugin-beta") > 0);

    let out = env
        .cmd()
        .args([
            "plugin",
            "disable",
            "sample-plugin-catalog/plugin-*",
            "--force",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "glob disable must succeed; exit {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    // NDJSON: one record per successfully-disabled plugin (two lines here).
    let stdout = String::from_utf8_lossy(&out.stdout);
    let plugins: Vec<String> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let v: Value = serde_json::from_str(l).expect("each line is a JSON record");
            assert_eq!(v["status"], "disabled");
            v["plugin"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        plugins,
        vec![
            "sample-plugin-catalog/plugin-alpha".to_owned(),
            "sample-plugin-catalog/plugin-beta".to_owned(),
        ],
        "both plugins must be disabled, sorted by candidate order",
    );

    // Both plugins are now disabled on disk.
    assert_eq!(enabled_count_for(&paths, "plugin-alpha"), 0);
    assert_eq!(enabled_count_for(&paths, "plugin-beta"), 0);
}

/// A batch with one BAD id among good ids: the good ids are processed, the
/// process exits with the first error's code, and the JSON stream carries ONLY
/// the good records. A bad literal `<catalog>/<plugin>` that does not exist maps
/// to `PluginNotFound` (exit 20) downstream.
#[test]
fn disable_forward_progress_processes_good_and_surfaces_first_error() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = setup_with_alpha_and_beta_enabled(&env, &fixture_tmp);

    let out = env
        .cmd()
        .args([
            "plugin",
            "disable",
            // bad (nonexistent) literal id FIRST, then a good one — proves the
            // loop does not short-circuit on the bad token.
            "sample-plugin-catalog/does-not-exist",
            "sample-plugin-catalog/plugin-alpha",
            "--force",
            "--json",
        ])
        .output()
        .unwrap();

    // The good plugin was disabled...
    assert_eq!(
        enabled_count_for(&paths, "plugin-alpha"),
        0,
        "the good id must still be disabled despite the bad one",
    );
    // ...beta was not in the selection, so it stays enabled.
    assert!(enabled_count_for(&paths, "plugin-beta") > 0);

    // Exit code is the first error's — PluginNotFound (20) for the bad literal.
    assert_eq!(
        out.status.code(),
        Some(20),
        "expected exit 20 (PluginNotFound) from the bad id; stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );

    // JSON stream: exactly ONE record, for the good plugin. The failed id is
    // NOT emitted in JSON (stderr + non-zero exit convey it).
    let stdout = String::from_utf8_lossy(&out.stdout);
    let records: Vec<Value> = stdout
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).expect("JSON record"))
        .collect();
    assert_eq!(
        records.len(),
        1,
        "only the good record is emitted; got {stdout:?}"
    );
    assert_eq!(records[0]["plugin"], "sample-plugin-catalog/plugin-alpha");
    assert_eq!(records[0]["status"], "disabled");
}

/// A wildcard that matches nothing is an ERROR (exit 2, Usage), never a silent
/// success — a user who typed `xyz-*` and hit nothing wants to know.
#[test]
fn disable_zero_glob_match_is_usage_error_not_silent_success() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_and_beta_enabled(&env, &fixture_tmp);

    let out = env
        .cmd()
        .args([
            "plugin",
            "disable",
            "sample-plugin-catalog/xyz-*",
            "--force",
        ])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(2),
        "a zero-match glob must be a Usage error (exit 2), got {:?}; stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("sample-plugin-catalog/xyz-*"),
        "error must echo the pattern, got: {stderr}",
    );
}

/// A bare name scoped by `--catalog` resolves to that catalog. Disabling
/// `plugin-alpha --catalog sample-plugin-catalog` targets exactly it.
#[test]
fn disable_bare_name_scoped_by_catalog_flag() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = setup_with_alpha_and_beta_enabled(&env, &fixture_tmp);

    let out = env
        .cmd()
        .args([
            "plugin",
            "disable",
            "plugin-alpha",
            "--catalog",
            "sample-plugin-catalog",
            "--force",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "bare-name + --catalog disable must succeed; exit {:?}, stderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let record: Value = serde_json::from_slice(&out.stdout).expect("one JSON record");
    assert_eq!(record["plugin"], "sample-plugin-catalog/plugin-alpha");
    assert_eq!(enabled_count_for(&paths, "plugin-alpha"), 0);
    assert!(enabled_count_for(&paths, "plugin-beta") > 0);
}

// ---- issue #314 FIX 1: malformed literal ids → exit 2 (Usage) ------------
//
// The pre-#314 single-id path parsed via `PluginId::from_str` and mapped any
// shape failure to `Usage` (exit 2). The selector must preserve that — and the
// sibling `tome reindex bad/id/extra` PINS exit 2, which #316 inherits by
// reusing `selector::resolve` verbatim. These drive BOTH enable and disable
// through the CLI binary; a malformed id short-circuits in `resolve` (empty
// match set) BEFORE any embedder/model work, so `plugin enable` is ONNX-free
// on this path and safe to run in CI.

/// `plugin disable bad/id/extra` (two slashes) → exit 2, mirroring the reindex
/// pin. No state is touched.
#[test]
fn disable_malformed_literal_two_slashes_exits_2() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_and_beta_enabled(&env, &fixture_tmp);

    let out = env
        .cmd()
        .args(["plugin", "disable", "bad/id/extra", "--force"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(2),
        "malformed literal (two slashes) must be Usage (exit 2); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// `plugin disable mycat/..` (traversal segment) → exit 2. Rejecting the shape
/// here means no unvalidated `..` reaches `resolve_plugin_dir`'s `join`.
#[test]
fn disable_traversal_segment_exits_2() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_and_beta_enabled(&env, &fixture_tmp);

    let out = env
        .cmd()
        .args(["plugin", "disable", "mycat/..", "--force"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(2),
        "traversal segment must be Usage (exit 2); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// `plugin enable bad/id/extra` → exit 2, matching disable + reindex. This is
/// the enable-side parity: the malformed id short-circuits in `resolve` before
/// any model work.
#[test]
fn enable_malformed_literal_two_slashes_exits_2() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_and_beta_enabled(&env, &fixture_tmp);

    let out = env
        .cmd()
        .args(["plugin", "enable", "bad/id/extra", "--yes"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(2),
        "malformed literal on enable must be Usage (exit 2); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

/// An empty-string id → exit 2 (a shape error, not a confusing `NotFound`).
#[test]
fn disable_empty_id_exits_2() {
    let fixture_tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let _paths = setup_with_alpha_and_beta_enabled(&env, &fixture_tmp);

    let out = env
        .cmd()
        .args(["plugin", "disable", "", "--force"])
        .output()
        .unwrap();

    assert_eq!(
        out.status.code(),
        Some(2),
        "empty id must be Usage (exit 2); stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
}

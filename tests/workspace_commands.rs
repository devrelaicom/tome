//! Phase 5 / US3 — cross-product scope isolation.
//!
//! Proves that every Phase 1/2 command honours the resolved `Scope`:
//! mutations against a workspace land in `<workspace>/.tome/` and only
//! there; mutations against `--global` from inside the workspace land in
//! `${XDG_CONFIG_HOME}/tome/` and only there.
//!
//! Most of the legwork was done by Foundational F1 (which threaded
//! `ResolvedScope` into every `commands::*::run` and added the
//! `Paths::config_file_for(&Scope)` / `index_db_for(&Scope)` /
//! `index_lock_for(&Scope)` accessors). This file is the integration
//! evidence that nothing was missed.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{
    Fixture, ToolEnv, config_with_catalog, copy_sample_plugin_catalog, lifecycle_paths, paths_for,
    write_config_for_cli,
};
use tempfile::TempDir;
use tome::commands::plugin::registry_seeds;
use tome::embedding::stub::StubEmbedder;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::Scope;

/// Create a fresh workspace under `parent` and return its canonical
/// absolute path. The directory is populated via `tome workspace init`
/// so it matches what a real user gets.
fn make_workspace(env: &ToolEnv, parent: &Path, name: &str) -> PathBuf {
    let root = parent.join(name);
    std::fs::create_dir_all(&root).unwrap();
    let out = env
        .cmd()
        .args(["workspace", "init", root.to_str().unwrap()])
        .output()
        .expect("workspace init");
    assert!(
        out.status.success(),
        "init failed: stderr={}",
        String::from_utf8_lossy(&out.stderr),
    );
    std::fs::canonicalize(&root).unwrap()
}

/// Build a `Command` for the `tome` binary with the env's HOME / XDG vars
/// and an additional `current_dir(cwd)` so any CWD-walk inside the
/// resolver behaves predictably.
fn cmd_in(env: &ToolEnv, cwd: &Path) -> Command {
    let mut c = env.cmd();
    c.current_dir(cwd);
    c
}

// ---- Catalog scope isolation ---------------------------------------------

#[test]
fn catalog_add_workspace_does_not_touch_global() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace(&env, tmp.path(), "project");

    // Add the catalog under the workspace scope (explicit --workspace flag).
    let out = env
        .cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "add",
            &fix.url,
        ])
        .output()
        .expect("spawn tome");
    assert!(
        out.status.success(),
        "exit={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    // Workspace config has the catalog.
    let ws_config = ws.join(".tome/config.toml");
    let ws_body = std::fs::read_to_string(&ws_config).expect("workspace config exists");
    assert!(
        ws_body.contains("[catalogs.sample-experts]"),
        "workspace config missing catalog: {ws_body}",
    );

    // Global config either doesn't exist, or exists but is empty.
    let global_config = env.config_file();
    if global_config.exists() {
        let global_body = std::fs::read_to_string(&global_config).unwrap();
        assert!(
            !global_body.contains("sample-experts"),
            "global config leaked the workspace catalog: {global_body}",
        );
    }
}

#[test]
fn catalog_add_global_inside_workspace_does_not_touch_workspace() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace(&env, tmp.path(), "project");

    // Run from inside the workspace but with --global; the resolver
    // must pick global despite the CWD walk would otherwise find .tome/.
    let out = cmd_in(&env, &ws)
        .args(["--global", "catalog", "add", &fix.url])
        .output()
        .expect("spawn tome");
    assert!(
        out.status.success(),
        "exit={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    // Global config has the catalog; workspace config is unchanged.
    let global_body = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(global_body.contains("[catalogs.sample-experts]"));

    let ws_body = std::fs::read_to_string(ws.join(".tome/config.toml")).unwrap();
    assert!(
        !ws_body.contains("sample-experts"),
        "workspace config leaked the global catalog: {ws_body}",
    );
}

#[test]
fn catalog_list_returns_only_scope_catalogs() {
    // Two distinct catalogs — same-URL sharing across scopes is the
    // job of US3.b's reference-counting, not this test.
    let fix_g = Fixture::build_sample();
    let fix_w = Fixture::build_sample();
    let env = ToolEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace(&env, tmp.path(), "project");

    let out_g = env
        .cmd()
        .args(["--global", "catalog", "add", &fix_g.url, "--name", "g"])
        .output()
        .unwrap();
    assert!(
        out_g.status.success(),
        "{}",
        String::from_utf8_lossy(&out_g.stderr)
    );
    let out_w = env
        .cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "add",
            &fix_w.url,
            "--name",
            "w",
        ])
        .output()
        .unwrap();
    assert!(
        out_w.status.success(),
        "{}",
        String::from_utf8_lossy(&out_w.stderr)
    );

    // List under workspace scope returns only the workspace alias.
    let ws_list = env
        .cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "--json",
            "catalog",
            "list",
        ])
        .output()
        .unwrap();
    assert!(ws_list.status.success());
    let ws_stdout = String::from_utf8_lossy(&ws_list.stdout);
    assert!(ws_stdout.contains("\"w\""), "workspace list: {ws_stdout}");
    assert!(
        !ws_stdout.contains("\"g\""),
        "workspace list leaked global alias: {ws_stdout}",
    );

    // List under global scope returns only the global alias.
    let g_list = env
        .cmd()
        .args(["--global", "--json", "catalog", "list"])
        .output()
        .unwrap();
    assert!(g_list.status.success());
    let g_stdout = String::from_utf8_lossy(&g_list.stdout);
    assert!(g_stdout.contains("\"g\""), "global list: {g_stdout}");
    assert!(
        !g_stdout.contains("\"w\""),
        "global list leaked workspace alias: {g_stdout}",
    );
}

#[test]
fn catalog_remove_affects_only_resolved_scope() {
    // Two distinct catalogs (see note on `catalog_list_returns_only_scope_catalogs`).
    let fix_g = Fixture::build_sample();
    let fix_w = Fixture::build_sample();
    let env = ToolEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace(&env, tmp.path(), "project");

    env.cmd()
        .args(["--global", "catalog", "add", &fix_g.url, "--name", "g"])
        .output()
        .unwrap();
    env.cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "add",
            &fix_w.url,
            "--name",
            "w",
        ])
        .output()
        .unwrap();

    // Remove the workspace one.
    let rm = env
        .cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "remove",
            "w",
            "--force",
        ])
        .output()
        .unwrap();
    assert!(
        rm.status.success(),
        "rm stderr={}",
        String::from_utf8_lossy(&rm.stderr)
    );

    // Workspace config no longer has it; global still has its own catalog.
    let ws_body = std::fs::read_to_string(ws.join(".tome/config.toml")).unwrap();
    assert!(!ws_body.contains("[catalogs.w]"), "{ws_body}");
    let g_body = std::fs::read_to_string(env.config_file()).unwrap();
    assert!(g_body.contains("[catalogs.g]"), "{g_body}");
}

// ---- Workspace bootstrap-on-first-write ----------------------------------

#[test]
fn catalog_add_in_freshly_init_workspace_creates_config_toml() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace(&env, tmp.path(), "project");

    // After init the workspace has .tome/config.toml (init writes an empty
    // one). Delete it to simulate "user removed config.toml then ran
    // catalog add" — we must recreate, not crash.
    std::fs::remove_file(ws.join(".tome/config.toml")).unwrap();

    let out = env
        .cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "add",
            &fix.url,
        ])
        .output()
        .expect("spawn");
    assert!(
        out.status.success(),
        "exit={:?} stderr={}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    let body = std::fs::read_to_string(ws.join(".tome/config.toml")).unwrap();
    assert!(body.contains("[catalogs.sample-experts]"));
}

// ---- Workspace info reflects per-scope state -----------------------------

#[test]
fn workspace_info_reflects_per_scope_catalog_count() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace(&env, tmp.path(), "project");

    // Add only to global. Workspace scope should still report 0 catalogs.
    env.cmd()
        .args(["--global", "catalog", "add", &fix.url])
        .output()
        .unwrap();

    let ws_info = env
        .cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "--json",
            "workspace",
            "info",
        ])
        .output()
        .unwrap();
    assert!(ws_info.status.success());
    let v: serde_json::Value = serde_json::from_slice(&ws_info.stdout).unwrap();
    assert_eq!(v["scope"], "workspace");
    assert_eq!(v["catalogs"], 0);

    let g_info = env
        .cmd()
        .args(["--global", "--json", "workspace", "info"])
        .output()
        .unwrap();
    let g: serde_json::Value = serde_json::from_slice(&g_info.stdout).unwrap();
    assert_eq!(g["scope"], "global");
    assert_eq!(g["catalogs"], 1);
}

// ---- CWD walk picks the workspace ----------------------------------------

#[test]
fn cwd_walk_inside_workspace_picks_workspace_scope() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace(&env, tmp.path(), "project");

    // No --workspace flag — resolution should walk from CWD and find
    // .tome/.
    let out = cmd_in(&env, &ws)
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let ws_body = std::fs::read_to_string(ws.join(".tome/config.toml")).unwrap();
    assert!(ws_body.contains("[catalogs.sample-experts]"));
    let global_present = env.config_file().exists()
        && std::fs::read_to_string(env.config_file())
            .unwrap()
            .contains("sample-experts");
    assert!(
        !global_present,
        "CWD walk leaked workspace catalog into global",
    );
}

// ---- Plugin enable scope isolation (library API; embedder = stub) --------

#[test]
fn plugin_enable_indexes_into_resolved_scope_only() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.data_dir).unwrap();
    common::fabricate_all_installed_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");

    // Set up TWO scopes:
    //  - the global scope (paths.config_file_for(&Scope::Global))
    //  - a workspace at <tmp>/workspace
    let workspace_root = tmp.path().join("workspace");
    std::fs::create_dir_all(workspace_root.join(".tome")).unwrap();

    let global_config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    let ws_config = config_with_catalog("sample-plugin-catalog", &catalog_root);

    // Write both configs.
    write_config_for_cli(&paths, &global_config);
    std::fs::write(
        paths.config_file_for(&Scope::Workspace(workspace_root.clone())),
        toml::to_string_pretty(&ws_config).unwrap(),
    )
    .unwrap();

    let ws_scope = Scope::Workspace(workspace_root.clone());
    let (embedder_seed, reranker_seed) = registry_seeds();
    let embedder = StubEmbedder::new();

    // Enable plugin under WORKSPACE scope only.
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &ws_scope,
        config: &ws_config,
        embedder: &embedder,
        embedder_seed: embedder_seed.clone(),
        reranker_seed: reranker_seed.clone(),
        allow_model_download: false,
    };
    lifecycle::enable(&id, &deps).expect("enable in workspace");

    // Workspace DB has rows.
    let ws_db = paths.index_db_for(&ws_scope);
    assert!(ws_db.is_file(), "workspace DB should exist at {ws_db:?}");

    // Global DB must NOT have been touched.
    let global_db = paths.index_db_for(&Scope::Global);
    assert!(
        !global_db.is_file(),
        "global DB should not exist; workspace enable should not have touched it",
    );

    // Now enable the same plugin under GLOBAL scope.
    let g_deps = LifecycleDeps {
        paths: &paths,
        scope: &Scope::Global,
        config: &global_config,
        embedder: &embedder,
        embedder_seed,
        reranker_seed,
        allow_model_download: false,
    };
    lifecycle::enable(&id, &g_deps).expect("enable globally");
    assert!(global_db.is_file(), "global DB should be bootstrapped");

    // Both DBs exist independently. Disabling in one must not affect the
    // other.
    let (e_seed, r_seed) = registry_seeds();
    lifecycle::disable(&id, &paths, &Scope::Global, &global_config, e_seed, r_seed)
        .expect("disable global");
    // Workspace plugin row must still be enabled.
    let ws_conn = tome::index::open_read_only(&ws_db).unwrap();
    let enabled: i64 = ws_conn
        .query_row(
            "SELECT COUNT(*) FROM skills WHERE catalog = 'sample-plugin-catalog' \
             AND plugin = 'plugin-alpha' AND enabled = 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(
        enabled > 0,
        "workspace plugin should still be enabled after disabling globally",
    );
}

// ---- Status + reindex resolve the right scope ---------------------------

#[test]
fn status_reports_per_scope_index() {
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    let tmp = TempDir::new().unwrap();
    let ws = make_workspace(&env, tmp.path(), "project");

    env.cmd()
        .args(["--global", "catalog", "add", &fix.url])
        .output()
        .unwrap();

    // Workspace status reports zero plugins enabled (no global bleed).
    let s = env
        .cmd()
        .args(["--workspace", ws.to_str().unwrap(), "--json", "status"])
        .output()
        .unwrap();
    // Status exits 1 on degraded — models not present in the isolated XDG
    // home — but the JSON record is still emitted on stdout before the
    // exit. Parse it regardless of exit code.
    let stdout = String::from_utf8_lossy(&s.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("status json");
    // Either the workspace index doesn't exist yet OR plugins_enabled is 0.
    let enabled = v["index"]["plugins_enabled"].as_u64().unwrap_or(0);
    assert_eq!(enabled, 0, "status: {v}");
}

// ---- Unused imports silencer --------------------------------------------

#[allow(dead_code)]
fn _unused_imports_silencer() {
    let _ = paths_for;
}

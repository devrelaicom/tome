//! Phase 9 / US7 — `tome catalog remove` cascade semantics.
//!
//! - Refuse case: enabled plugins in the catalog → exit 53.
//! - Cascade case: `--force` drops rows then removes the catalog.
//! - No-enabled case: behaves identically to the Phase 1 catalog-remove flow.
//!
//! Enable goes through the library API + StubEmbedder so we don't need to
//! load ONNX models in CI. The remove path is driven by the CLI binary —
//! it doesn't construct a FastembedEmbedder, the cascade is pure deletion.

mod common;

use common::{
    Fixture, ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_models, paths_for,
    stub_embedder_seed, stub_reranker_seed, write_config_for_cli,
};
use serde_json::Value;
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::index::{self, OpenOptions};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

fn enable_alpha(
    paths: &tome::paths::Paths,
    config: &tome::config::Config,
    embedder: &StubEmbedder,
) {
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let deps = LifecycleDeps {
        paths,
        scope: &tome::workspace::Scope::Global,
        config,
        embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        allow_model_download: false,
    };
    lifecycle::enable(&id, &deps).expect("enable alpha");
}

fn count_skill_rows(paths: &tome::paths::Paths, catalog: &str) -> i64 {
    if !paths.index_db.is_file() {
        return 0;
    }
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
        },
    )
    .unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM skills WHERE catalog = ?1",
        rusqlite::params![catalog],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

#[test]
fn refuse_remove_when_enabled_plugins_exist() {
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // Bootstrap on-disk state: copy sample-plugin-catalog into the env's
    // catalogs dir, write a Config to disk so the CLI can find it, enable
    // plugin-alpha via library API.
    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);
    let embedder = StubEmbedder::new();
    enable_alpha(&paths, &config, &embedder);

    let out = env
        .cmd()
        .args(["catalog", "remove", "sample-plugin-catalog"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(53),
        "expected exit 53, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("sample-plugin-catalog/plugin-alpha"),
        "stderr should mention the enabled plugin id, got: {stderr}",
    );
    // Catalog config NOT mutated.
    let cfg_text = std::fs::read_to_string(&paths.global_config_file).unwrap();
    assert!(cfg_text.contains("sample-plugin-catalog"));
    // Skill rows NOT dropped.
    assert!(count_skill_rows(&paths, "sample-plugin-catalog") > 0);
}

#[test]
fn force_cascades_disable_and_removes_catalog() {
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &config);
    let embedder = StubEmbedder::new();
    enable_alpha(&paths, &config, &embedder);
    let baseline = count_skill_rows(&paths, "sample-plugin-catalog");
    assert!(baseline > 0);

    let out = env
        .cmd()
        .args([
            "--json",
            "catalog",
            "remove",
            "sample-plugin-catalog",
            "--force",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "expected exit 0, got {:?}, stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr),
    );

    // JSON record includes the cascade array with REAL per-plugin counts.
    let v: Value = serde_json::from_slice(&out.stdout).expect("parse JSON");
    let cascade = v["removed"]["cascade"].as_array().expect("cascade array");
    assert!(!cascade.is_empty(), "cascade array should be non-empty");
    assert_eq!(cascade[0]["plugin"], "sample-plugin-catalog/plugin-alpha");
    let dropped = cascade[0]["skills_dropped"]
        .as_u64()
        .expect("skills_dropped is a number");
    assert_eq!(
        dropped as i64, baseline,
        "skills_dropped on the sole enabled plugin should equal the pre-cascade row count",
    );
    assert!(
        dropped > 0,
        "skills_dropped must be the real count, not zero",
    );

    // Catalog removed from config.
    let cfg_text = std::fs::read_to_string(&paths.global_config_file).unwrap();
    assert!(
        !cfg_text.contains("sample-plugin-catalog"),
        "config should no longer reference the removed catalog, got: {cfg_text}",
    );
    // Skill rows dropped.
    assert_eq!(count_skill_rows(&paths, "sample-plugin-catalog"), 0);
}

#[test]
fn no_enabled_plugins_keeps_phase_1_behaviour() {
    // No library API needed — just register a catalog via the CLI binary
    // (no `tome plugin enable`), then remove with --force. Exits 0; no
    // cascade fires; behaviour matches Phase 1.
    let fix = Fixture::build_sample();
    let env = ToolEnv::new();
    env.cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();

    let out = env
        .cmd()
        .args(["--json", "catalog", "remove", "sample-experts", "--force"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr),
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("parse JSON");
    // The cascade array is `skip_serializing_if = "Vec::is_empty"`, so it
    // should not appear on a no-enabled-plugins remove.
    assert!(
        v["removed"]["cascade"].is_null()
            || v["removed"]["cascade"]
                .as_array()
                .is_some_and(|a| a.is_empty()),
        "no cascade array expected when no plugins enabled, got: {v}",
    );
    assert_eq!(v["removed"]["name"], "sample-experts");
}

// ---- Phase 3 / US3.b — cascade + reference-count integration -------------

/// Cascade-remove from a workspace that shares the catalog URL with the
/// global scope. Workspace's plugins drop; workspace's config loses the
/// entry; global's config keeps it; the on-disk clone survives because
/// global still references it.
///
/// Phase 4 / F2a collapses both scopes onto the same central
/// `config.toml` + `index.db`. The Phase 3 cross-scope isolation
/// invariant this test asserts is precisely what F11 reintroduces via
/// the `workspace_catalogs` junction table. Ignored until F11 lands.
#[test]
#[ignore = "F11: per-workspace isolation moves to workspace_catalogs junction table"]
fn cascade_remove_in_workspace_does_not_remove_shared_clone() {
    use common::sample_plugin_catalog_fixture;
    use sha2::{Digest, Sha256};

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // Opt the user into the workspace registry so `reference_count`
    // can see the workspace.
    std::fs::create_dir_all(&paths.logs_dir).unwrap();
    std::fs::File::create(&paths.global_config_file).unwrap();

    // Build a real git fixture from sample-plugin-catalog so the CLI's
    // `catalog add` succeeds (it needs a cloneable repo).
    let fix = Fixture::build_from(sample_plugin_catalog_fixture());

    // Add the catalog to GLOBAL first via CLI binary (real clone).
    let add_g = env
        .cmd()
        .args(["--global", "catalog", "add", &fix.url])
        .output()
        .unwrap();
    assert!(
        add_g.status.success(),
        "global add failed: {}",
        String::from_utf8_lossy(&add_g.stderr),
    );

    // Init a workspace and add the SAME URL into it (reuses cache).
    let ws_root = env.home_path().join("project");
    std::fs::create_dir_all(&ws_root).unwrap();
    let init = env
        .cmd()
        .args(["workspace", "init", ws_root.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "workspace init failed: {}",
        String::from_utf8_lossy(&init.stderr),
    );
    let ws = std::fs::canonicalize(&ws_root).unwrap();

    let add_w = env
        .cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "add",
            &fix.url,
        ])
        .output()
        .unwrap();
    assert!(
        add_w.status.success(),
        "workspace add failed: {}",
        String::from_utf8_lossy(&add_w.stderr),
    );

    // Enable plugin-alpha in the WORKSPACE scope via library API +
    // StubEmbedder. We have to construct LifecycleDeps with the
    // workspace config the CLI just wrote.
    let workspace_scope = tome::workspace::Scope::Workspace(ws.clone());
    let workspace_config_path = ws.join(".tome/config.toml");
    let workspace_config: tome::config::Config =
        toml::from_str(&std::fs::read_to_string(&workspace_config_path).unwrap()).unwrap();
    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &workspace_scope,
        config: &workspace_config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        allow_model_download: false,
    };
    let plugin_id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    lifecycle::enable(&plugin_id, &deps).expect("enable in workspace");

    // Compute the cache directory.
    let cache_dir = {
        let mut h = Sha256::new();
        h.update(fix.url.as_bytes());
        env.catalogs_dir().join(hex::encode(h.finalize()))
    };
    assert!(cache_dir.is_dir());

    // Workspace's index has rows for the plugin.
    let ws_db = paths.index_db.clone();
    let _ = &workspace_scope;
    let pre_rows: i64 = {
        let conn = index::open(
            &ws_db,
            &OpenOptions {
                embedder: stub_embedder_seed(),
                reranker: stub_reranker_seed(),
            },
        )
        .unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM skills WHERE catalog = 'sample-plugin-catalog'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert!(
        pre_rows > 0,
        "workspace index should have rows before remove"
    );

    // Cascade-remove from the workspace.
    let rm = env
        .cmd()
        .args([
            "--workspace",
            ws.to_str().unwrap(),
            "catalog",
            "remove",
            "sample-plugin-catalog",
            "--force",
        ])
        .output()
        .unwrap();
    assert!(
        rm.status.success(),
        "cascade remove failed: {}",
        String::from_utf8_lossy(&rm.stderr),
    );

    // Workspace config no longer has it.
    let ws_body = std::fs::read_to_string(&workspace_config_path).unwrap();
    assert!(!ws_body.contains("sample-plugin-catalog"), "{ws_body}");

    // Workspace index rows for the catalog dropped.
    let post_rows: i64 = {
        let conn = index::open(
            &ws_db,
            &OpenOptions {
                embedder: stub_embedder_seed(),
                reranker: stub_reranker_seed(),
            },
        )
        .unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM skills WHERE catalog = 'sample-plugin-catalog'",
            [],
            |r| r.get(0),
        )
        .unwrap()
    };
    assert_eq!(post_rows, 0, "workspace skills should be dropped");

    // Global config STILL references the catalog.
    let g_body = std::fs::read_to_string(&paths.global_config_file).unwrap();
    assert!(
        g_body.contains("sample-plugin-catalog"),
        "global config lost the catalog: {g_body}",
    );

    // Cache directory SURVIVES — global still references it.
    assert!(
        cache_dir.is_dir(),
        "cache directory deleted despite global still referencing the URL",
    );
}

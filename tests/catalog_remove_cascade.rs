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
    std::fs::create_dir_all(&paths.data_dir).unwrap();
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
    let cfg_text = std::fs::read_to_string(&paths.config_file).unwrap();
    assert!(cfg_text.contains("sample-plugin-catalog"));
    // Skill rows NOT dropped.
    assert!(count_skill_rows(&paths, "sample-plugin-catalog") > 0);
}

#[test]
fn force_cascades_disable_and_removes_catalog() {
    let tmp = TempDir::new().unwrap();
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.data_dir).unwrap();
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
    let cfg_text = std::fs::read_to_string(&paths.config_file).unwrap();
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

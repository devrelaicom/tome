//! Phase 9 / US7 — `tome catalog remove` cascade semantics.
//!
//! - Refuse case: enabled plugins in the catalog → exit 53.
//! - Cascade case: `--force` drops rows then removes the catalog.
//! - No-enabled case: behaves identically to the Phase 1 catalog-remove flow.
//!
//! Enable goes through the library API + StubEmbedder so we don't need to
//! load ONNX models in CI. The remove path is driven by the CLI binary —
//! it doesn't construct a FastembedEmbedder, the cascade is pure deletion.

use crate::common::{
    Fixture, ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_models, paths_for,
    stub_embedder_seed, stub_reranker_seed, stub_summariser_seed, write_config_for_cli,
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
        scope: &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        config,
        embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
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
            summariser: stub_summariser_seed(),
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

/// Count workspace_skills enrolments for one (workspace, catalog). The F11a
/// cascade semantics drop junction rows for the resolved workspace; the
/// underlying skills rows are retained per FR-383.
fn count_workspace_enrolments(paths: &tome::paths::Paths, workspace: &str, catalog: &str) -> i64 {
    if !paths.index_db.is_file() {
        return 0;
    }
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
        "SELECT COUNT(*) FROM workspace_skills AS ws
         JOIN skills     AS s ON s.id = ws.skill_id
         JOIN workspaces AS w ON w.id = ws.workspace_id
         WHERE w.name = ?1 AND s.catalog = ?2",
        rusqlite::params![workspace, catalog],
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
    // Catalog enrolment NOT removed from the DB (workspace_catalogs still has it).
    assert!(
        has_workspace_enrolment(&paths, "global", "sample-plugin-catalog"),
        "workspace_catalogs must still have the catalog row after a refused remove",
    );
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

    // Catalog removed from workspace_catalogs (F11b: enrolment lives
    // in the central DB, no longer in config.toml).
    use crate::common::has_global_enrolment;
    assert!(
        !has_global_enrolment(&paths, "sample-plugin-catalog"),
        "enrolment should no longer exist in workspace_catalogs",
    );
    // Phase 4 / F11a: workspace_skills enrolments for the resolved
    // workspace are gone; the underlying skill rows are retained per
    // FR-383 so other workspaces (post-F11b multi-workspace catalog
    // enrolment) keep working.
    assert_eq!(
        count_workspace_enrolments(&paths, "global", "sample-plugin-catalog"),
        0,
        "cascade should clear workspace_skills enrolments for the global workspace",
    );
    assert!(
        count_skill_rows(&paths, "sample-plugin-catalog") > 0,
        "F11a retention rule: cascade must keep `skills` rows alive (FR-383)",
    );
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

// ---- Phase 4 / F11b — cascade + workspace_catalogs refcount integration --

/// Cascade-remove from a workspace that shares the catalog URL with the
/// `global` workspace. Workspace's `workspace_skills` enrolments drop;
/// workspace's `workspace_catalogs` row disappears; `global`'s rows
/// stay; the shared on-disk clone survives because `global` still
/// references the URL (FR-361).
///
/// Phase 4 / F11b reintroduces the Phase 3 cross-scope isolation
/// invariant via the central `workspace_catalogs` junction table — the
/// per-workspace `.tome/` directories Phase 3 used are gone.
#[test]
fn cascade_remove_in_workspace_does_not_remove_shared_clone() {
    use crate::common::{cache_dir_for, seed_workspace};

    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // Real git fixture from sample-plugin-catalog (the CLI add path
    // wants a cloneable repo). The on-disk clone lands at
    // `paths.cache_dir_for(&url)` — same for both workspaces.
    let fix = Fixture::build_from(crate::common::sample_plugin_catalog_fixture());

    // Step 1: add the catalog under the privileged `global` workspace
    // via the CLI binary. This bootstraps the central DB + stamps meta
    // with REGISTRY seeds (matching what the second CLI invocation
    // below will use). We can't switch to stub seeds without breaking
    // the CLI side.
    let add_g = env
        .cmd()
        .args(["catalog", "add", &fix.url])
        .output()
        .unwrap();
    assert!(
        add_g.status.success(),
        "global add failed: {}",
        String::from_utf8_lossy(&add_g.stderr),
    );

    // Step 2: seed a `second` workspace directly into the central DB.
    // `tome workspace add` (US2) will own this seam once it ships;
    // until then the test bridges by inserting a `workspaces` row.
    seed_workspace(&paths, "second");

    // Step 3: enrol the SAME URL in the `second` workspace via the CLI
    // binary. F11b's add path detects the existing cache (refcount > 0)
    // and REUSES it — same on-disk clone shared across workspaces.
    let add_w = env
        .cmd()
        .args(["--workspace", "second", "catalog", "add", &fix.url])
        .output()
        .unwrap();
    assert!(
        add_w.status.success(),
        "second-workspace add failed: {}",
        String::from_utf8_lossy(&add_w.stderr),
    );

    // Step 4: enable plugin-alpha in the `second` workspace via
    // library API. The CLI binary would load FastembedEmbedder (real
    // ONNX); the library API can use StubEmbedder. The catalog clone
    // the CLI add wrote is at `paths.cache_dir_for(&fix.url)` — but
    // lifecycle::resolve_plugin_dir consults the Config struct's
    // CatalogEntry.path (Phase 1 carryover). We synthesise a Config
    // pointing at the cache dir so resolve_plugin_dir succeeds.
    let cache_dir = cache_dir_for(&env, &fix.url);
    assert!(cache_dir.is_dir(), "cache dir should exist post-add");

    // LifecycleDeps.config is vestigial (not read by the lifecycle); pass
    // the default so we don't need the removed Config.catalogs field.
    let synthetic_config = tome::config::Config::default();

    let second_scope = tome::workspace::Scope(
        tome::workspace::WorkspaceName::parse("second").expect("valid workspace name"),
    );
    let embedder = StubEmbedder::new();
    // Library-API enable uses stub seeds; meta was stamped with
    // registry seeds by the CLI add above. The lifecycle reads
    // `OpenOptions.embedder.name`/`version` only to forward into the
    // initial `index::open` — they're no-ops on subsequent opens since
    // meta is "first writer wins". But the lifecycle's drift check
    // would fire on a mismatch. To sidestep, we pass REGISTRY seeds to
    // the lifecycle so drift-check passes against the CLI-stamped
    // meta.
    let (reg_e, reg_r, reg_s) = {
        let pick = |kind| {
            let entry = tome::embedding::registry::MODEL_REGISTRY
                .iter()
                .find(|m| std::mem::discriminant(&m.kind) == std::mem::discriminant(&kind))
                .unwrap();
            tome::index::MetaSeed {
                name: entry.name.to_owned(),
                version: entry.version.to_owned(),
            }
        };
        (
            pick(tome::embedding::registry::ModelKind::Embedder),
            pick(tome::embedding::registry::ModelKind::Reranker),
            pick(tome::embedding::registry::ModelKind::Summariser),
        )
    };
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &second_scope,
        config: &synthetic_config,
        embedder: &embedder,
        embedder_seed: reg_e,
        reranker_seed: reg_r,
        summariser_seed: reg_s,
        allow_model_download: false,
    };
    let plugin_id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    lifecycle::enable(&plugin_id, &deps).expect("enable in `second`");

    // Sanity: `second` has workspace_skills enrolments now.
    let pre_ws_rows = count_workspace_enrolments(&paths, "second", "sample-plugin-catalog");
    assert!(pre_ws_rows > 0, "second should have enrolments pre-remove");

    // Step 5: cascade-remove from `second` via the CLI binary.
    let rm = env
        .cmd()
        .args([
            "--workspace",
            "second",
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

    // ---- Post-conditions ---------------------------------------------------
    // `second`'s workspace_catalogs row is gone.
    assert!(
        !has_workspace_enrolment(&paths, "second", "sample-plugin-catalog"),
        "second's workspace_catalogs row should be removed",
    );
    // `global`'s workspace_catalogs row survives.
    assert!(
        crate::common::has_global_enrolment(&paths, "sample-plugin-catalog"),
        "global's workspace_catalogs row must survive a sibling-workspace cascade",
    );
    // `second`'s workspace_skills enrolments are gone.
    assert_eq!(
        count_workspace_enrolments(&paths, "second", "sample-plugin-catalog"),
        0,
        "second's workspace_skills enrolments should be cleared",
    );
    // The shared `skills` rows survive per FR-383.
    assert!(
        count_skill_rows(&paths, "sample-plugin-catalog") > 0,
        "shared skill rows must survive — global still references them (FR-383)",
    );
    // Cache directory survives — `global` still references the URL
    // (FR-361 refcount > 0 → no cleanup).
    assert!(
        cache_dir.is_dir(),
        "cache directory deleted despite global still referencing the URL",
    );
}

/// Helper local to this test file: does the central DB hold a
/// `(workspace, catalog)` enrolment?
fn has_workspace_enrolment(paths: &tome::paths::Paths, workspace: &str, catalog: &str) -> bool {
    if !paths.index_db.is_file() {
        return false;
    }
    let conn = tome::index::open_read_only(&paths.index_db).expect("open ro");
    tome::index::workspace_catalogs::find(&conn, workspace, catalog)
        .map(|opt| opt.is_some())
        .unwrap_or(false)
}

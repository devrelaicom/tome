//! Phase 7 / US5 — `tome catalog update` re-embeds only changed skills and
//! auto-disables plugins whose upstream is gone (FR-033).
//!
//! These tests drive the library API path (`commands::catalog::update::
//! reindex_catalog_plugins`) so they can run with the deterministic
//! `StubEmbedder` and assert the cheap-skip invariant via `call_count()`.
//! The CLI binary path constructs `FastembedEmbedder` and cannot run in
//! CI — same boundary as `tome plugin enable`.

use crate::common::{
    config_with_catalog, copy_sample_plugin_catalog, enrol_catalog_symlinked, fabricate_models,
    lifecycle_paths, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use tome::commands::catalog::update::reindex_catalog_plugins;
use tome::embedding::stub::StubEmbedder;
use tome::error::TomeError;
use tome::index::{OpenOptions, acquire_lock, enabled_plugins_for_catalog};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

fn count_skills(paths: &tome::paths::Paths, catalog: &str, plugin: &str) -> (i64, i64) {
    let conn = tome::index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open index");
    conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(CASE WHEN ws.skill_id IS NOT NULL THEN 1 ELSE 0 END), 0)
         FROM skills AS s
         LEFT JOIN workspace_skills AS ws
                ON ws.skill_id = s.id
               AND ws.workspace_id = (SELECT id FROM workspaces WHERE name = 'global')
         WHERE s.catalog = ?1 AND s.plugin = ?2",
        rusqlite::params![catalog, plugin],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("count")
}

fn enable_alpha(
    paths: &tome::paths::Paths,
    config: &tome::config::Config,
    embedder: &StubEmbedder,
) -> usize {
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
    lifecycle::enable(&id, &deps).expect("initial enable");
    embedder.call_count()
}

#[test]
fn reindex_after_update_re_embeds_only_modified_skill() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog + symlink the cache dir onto the on-disk tree so
    // `resolve_plugin_dir` (DB-backed) sees in-place reindex mutations.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    let baseline = enable_alpha(&paths, &config, &embedder);
    // sample-plugin-catalog/plugin-alpha has 5 skills, one with malformed
    // YAML body is skipped — initial enable embeds 4.
    assert_eq!(baseline, 4, "initial enable embedded 4 skills");

    // Mutate one SKILL.md upstream — change the description so the
    // content_hash changes.
    let skill_b = catalog_root
        .join("plugin-alpha")
        .join("skills")
        .join("skill-b")
        .join("SKILL.md");
    std::fs::write(
        &skill_b,
        "---\nname: skill-b\ndescription: a fresh new description for slice 2\n---\nbody\n",
    )
    .unwrap();

    let deps = LifecycleDeps {
        paths: &paths,
        scope: &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let outcome =
        reindex_catalog_plugins("sample-plugin-catalog", &["plugin-alpha".to_owned()], &deps)
            .expect("reindex pass");

    assert_eq!(outcome.plugins.len(), 1);
    let change = &outcome.plugins[0];
    let summary = change.summary.expect("plugin-alpha reindexed cleanly");
    assert_eq!(summary.modified, 1, "exactly one skill modified");
    assert_eq!(summary.added, 0);
    assert_eq!(summary.removed, 0);
    assert_eq!(summary.unchanged, 3, "three other skills unchanged");
    assert_eq!(
        embedder.call_count() - baseline,
        1,
        "exactly one embed call should fire for the modified skill",
    );
}

#[test]
fn update_auto_disables_plugin_whose_upstream_directory_is_gone() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog + symlink the cache dir onto the on-disk tree so
    // `resolve_plugin_dir` (DB-backed) sees in-place reindex mutations.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha(&paths, &config, &embedder);
    assert_eq!(
        count_skills(&paths, "sample-plugin-catalog", "plugin-alpha"),
        (4, 4)
    );

    // Simulate upstream deletion: remove the plugin dir entirely.
    std::fs::remove_dir_all(catalog_root.join("plugin-alpha")).unwrap();
    // The tome-catalog.toml still lists plugin-alpha — that mirrors a state
    // where upstream forgot to update the manifest, OR an intermediate
    // state during a refresh. Either way the resolver returns
    // PluginNotFound, which triggers auto-disable.
    // To force the resolver into the manifest-path branch we also drop the
    // plugin from the manifest.
    let manifest_path = catalog_root.join("tome-catalog.toml");
    let mut manifest = std::fs::read_to_string(&manifest_path).unwrap();
    manifest = manifest.replace(
        "[[plugins]]\nname = \"plugin-alpha\"\nsource = \"./plugin-alpha\"\n\n",
        "",
    );
    std::fs::write(&manifest_path, manifest).unwrap();

    let deps = LifecycleDeps {
        paths: &paths,
        scope: &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let outcome =
        reindex_catalog_plugins("sample-plugin-catalog", &["plugin-alpha".to_owned()], &deps)
            .expect("reindex pass tolerates missing plugin");

    assert_eq!(outcome.plugins.len(), 1);
    let change = &outcome.plugins[0];
    let reason = change
        .auto_disabled
        .as_ref()
        .expect("plugin-alpha auto-disabled");
    assert!(
        reason.contains("missing") || reason.contains("malformed"),
        "reason describes the missing manifest: {reason}",
    );
    assert!(change.summary.is_none());

    // Every row for plugin-alpha is gone.
    assert_eq!(
        count_skills(&paths, "sample-plugin-catalog", "plugin-alpha"),
        (0, 0)
    );

    // enabled_plugins_for_catalog should now return an empty list.
    let conn = tome::index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open index");
    let enabled = enabled_plugins_for_catalog(&conn, "global", "sample-plugin-catalog").unwrap();
    assert!(enabled.is_empty(), "no enabled plugins remain");
}

#[test]
fn reindex_pass_unchanged_skills_does_no_embed_work() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog + symlink the cache dir onto the on-disk tree so
    // `resolve_plugin_dir` (DB-backed) sees in-place reindex mutations.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    let baseline = enable_alpha(&paths, &config, &embedder);

    let deps = LifecycleDeps {
        paths: &paths,
        scope: &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let outcome =
        reindex_catalog_plugins("sample-plugin-catalog", &["plugin-alpha".to_owned()], &deps)
            .expect("reindex pass");

    let summary = outcome.plugins[0].summary.expect("clean reindex");
    assert_eq!(summary.unchanged, 4);
    assert_eq!(summary.modified, 0);
    assert_eq!(
        embedder.call_count(),
        baseline,
        "no embed call should fire when nothing changed",
    );
}

/// Regression test for issue #512: `reindex_catalog_plugins` must NOT be
/// called while the advisory lock is held externally.
///
/// The original buggy fix for #512 held `acquire_lock` across the entire
/// `refresh_one` + reindex span in `update::run`. But `reindex_catalog_plugins`
/// → `lifecycle::reindex_plugin_with_entry_bar` → `lifecycle::reindex_plugin`
/// each call `acquire_lock` internally on the same lockfile. Because the lock is
/// non-reentrant (`File::try_lock`, per-fd semantics), a second `acquire_lock`
/// while the outer lock is held returns `WouldBlock` → `TomeError::IndexBusy`
/// (exit 50).
///
/// This test makes the invariant explicit: it FIRST acquires the lock, then
/// calls `reindex_catalog_plugins`, and asserts the nested acquire returns
/// `IndexBusy`. It then releases the outer lock and verifies the same call
/// succeeds — proving that the outer lock must be released before the reindex
/// call, which is exactly what the corrected `update::run` does.
#[test]
fn reindex_catalog_plugins_must_not_be_called_under_outer_lock() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    let embedder = StubEmbedder::new();
    enable_alpha(&paths, &config, &embedder);

    let deps = LifecycleDeps {
        paths: &paths,
        scope: &tome::workspace::Scope(tome::workspace::WorkspaceName::global()),
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let enabled = vec!["plugin-alpha".to_owned()];

    // --- Part 1: holding the outer lock → reindex returns IndexBusy (exit 50).
    //
    // This mirrors what the PRE-FIX `update::run` would have done: acquire the
    // lock before `refresh_one` and hold it through the reindex call.
    {
        let _outer = acquire_lock(&paths.index_lock).expect("acquire outer lock");
        let err = reindex_catalog_plugins("sample-plugin-catalog", &enabled, &deps)
            .expect_err("expected IndexBusy when outer lock is held");
        assert!(
            matches!(err, TomeError::IndexBusy),
            "expected IndexBusy (exit 50) but got: {err:?}",
        );
        // `_outer` is released here when the block ends.
    }

    // --- Part 2: WITHOUT the outer lock → reindex succeeds.
    //
    // This mirrors the POST-FIX `update::run`: the lock is released before
    // calling `reindex_catalog_plugins`, so the internal per-plugin lock
    // acquisition in `lifecycle::reindex_plugin` succeeds.
    let outcome = reindex_catalog_plugins("sample-plugin-catalog", &enabled, &deps)
        .expect("reindex must succeed once the outer lock is released");
    assert_eq!(outcome.plugins.len(), 1, "plugin-alpha was reindexed");
    assert!(
        outcome.plugins[0].auto_disabled.is_none(),
        "plugin-alpha must not be auto-disabled",
    );
}

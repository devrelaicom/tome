//! Phase 4 / F11c-1 — `tome catalog update` cross-workspace reindex
//! pass (FR-365).
//!
//! `commands::catalog::update::run` refreshes catalogs enrolled by ANY
//! workspace (`workspace_catalogs::distinct_urls`), then for each
//! refreshed URL visits every workspace that enrols it
//! (`workspace_catalogs::workspaces_with_catalog_url`), reindexing every
//! enabled plugin per workspace.
//!
//! Driving the full `update::run` from a test is impractical — it calls
//! `Paths::resolve()` from `$HOME` and constructs `FastembedEmbedder`,
//! both of which we'd have to monkey-patch process-globally. Instead we
//! compose the SAME public surface `run` composes:
//!
//!   - `workspace_catalogs::workspaces_with_catalog_url` for dispatch,
//!   - `enabled_plugins_for_catalog` for per-workspace enabled set,
//!   - `reindex_catalog_plugins` for the reindex itself,
//!
//! and assert both workspaces' skill rows reflect the upstream change.
//! If the dispatch surface regresses, this test fails before the CLI
//! binary sees the bug.

mod common;

use common::{
    copy_sample_plugin_catalog, fabricate_models, lifecycle_paths, seed_workspace,
    stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use tome::commands::catalog::update::reindex_catalog_plugins;
use tome::config::{CatalogEntry, Config};
use tome::embedding::stub::StubEmbedder;
use tome::index::workspace_catalogs;
use tome::index::{self, OpenOptions, enabled_plugins_for_catalog};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{Scope, WorkspaceName};

fn config_pointing_at(catalog_name: &str, catalog_root: &std::path::Path) -> Config {
    use std::collections::BTreeMap;
    let mut catalogs = BTreeMap::new();
    #[allow(deprecated)]
    catalogs.insert(
        catalog_name.to_owned(),
        CatalogEntry {
            name: catalog_name.to_owned(),
            url: format!("file://{}", catalog_root.display()),
            ref_: "main".into(),
            path: catalog_root.to_path_buf(),
            last_synced: time::OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap(),
        },
    );
    #[allow(deprecated)]
    Config { catalogs }
}

fn enable_in(paths: &tome::paths::Paths, scope: &Scope, config: &Config, embedder: &StubEmbedder) {
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let deps = LifecycleDeps {
        paths,
        scope,
        config,
        embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    lifecycle::enable(&id, &deps).expect("enable plugin-alpha");
}

/// Hash a `(workspace, skill_name)` to its current `content_hash` row
/// in `skills`. Returns None if no row exists. The `workspace` parameter
/// is only used to confirm that workspace has the enrolment via
/// `workspace_skills` — the hash itself is shared across workspaces
/// (FR-383: skill rows are shared).
fn skill_content_hash_for(
    paths: &tome::paths::Paths,
    workspace: &str,
    catalog: &str,
    plugin: &str,
    skill_name: &str,
) -> Option<String> {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .ok()?;
    conn.query_row(
        "SELECT s.content_hash
         FROM skills AS s
         JOIN workspace_skills AS ws ON ws.skill_id = s.id
         JOIN workspaces       AS w  ON w.id = ws.workspace_id
         WHERE w.name = ?1 AND s.catalog = ?2 AND s.plugin = ?3 AND s.name = ?4",
        rusqlite::params![workspace, catalog, plugin, skill_name],
        |row| row.get::<_, String>(0),
    )
    .ok()
}

/// Mirror of the production `workspace_catalogs::insert` for tests —
/// inserts an enrolment row for each (workspace, catalog_name, url).
/// We don't go through the CLI binary because that would clone the
/// fixture into the catalogs/ cache; for this test we want the
/// catalog directory to remain in-place under the TempDir so we can
/// mutate it after enable.
fn insert_enrolment(paths: &tome::paths::Paths, workspace: &str, name: &str, url: &str) {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open for enrolment insert");
    workspace_catalogs::insert(&conn, workspace, name, url, "main").expect("insert enrolment");
}

#[test]
fn catalog_update_reindexes_every_workspace_enabled_set() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    // Catalog on disk — we KEEP this directory as the source of truth
    // for upstream mutation. The Config points each workspace's
    // `lifecycle::resolve_plugin_dir` at this path.
    let catalog_root = copy_sample_plugin_catalog(&tmp, "sample-plugin-catalog");
    let url = format!("file://{}", catalog_root.display());
    let config = config_pointing_at("sample-plugin-catalog", &catalog_root);

    // Seed BOTH workspaces in the central DB. `global` is bootstrapped
    // automatically on first `index::open`; `second` we add.
    let global_scope = Scope(WorkspaceName::global());
    let second_scope = Scope(WorkspaceName::parse("second").unwrap());

    let embedder = StubEmbedder::new();

    // Enable in `global` first (bootstraps + stamps meta with stub seeds).
    enable_in(&paths, &global_scope, &config, &embedder);
    let calls_after_global = embedder.call_count();
    assert_eq!(calls_after_global, 4, "global enable embeds 4 skills");

    // Now seed `second` and enable there too. Cheap re-enable: same
    // skill rows → zero embed calls.
    seed_workspace(&paths, "second");
    enable_in(&paths, &second_scope, &config, &embedder);
    assert_eq!(
        embedder.call_count(),
        calls_after_global,
        "cross-workspace enable must NOT invoke the embedder",
    );

    // Seed enrolment rows for both workspaces (catalog add normally
    // does this via CLI). Order matters — we already used `index::open`
    // above, so meta is stamped with stub seeds; subsequent opens are
    // no-ops on meta.
    insert_enrolment(&paths, "global", "sample-plugin-catalog", &url);
    insert_enrolment(&paths, "second", "sample-plugin-catalog", &url);

    // Capture the pre-mutation content hash for skill-b in BOTH
    // workspaces. They must be identical (same shared skills row).
    let hash_before_global = skill_content_hash_for(
        &paths,
        "global",
        "sample-plugin-catalog",
        "plugin-alpha",
        "skill-b",
    )
    .expect("skill-b row in global");
    let hash_before_second = skill_content_hash_for(
        &paths,
        "second",
        "sample-plugin-catalog",
        "plugin-alpha",
        "skill-b",
    )
    .expect("skill-b row in second");
    assert_eq!(
        hash_before_global, hash_before_second,
        "shared skills row → identical content_hash across workspaces",
    );

    // ---- Upstream mutation -------------------------------------------------
    // Change skill-b's description. The lifecycle's content_hash is over
    // (name, description), so this MUST produce a different hash.
    let skill_b = catalog_root
        .join("plugin-alpha")
        .join("skills")
        .join("skill-b")
        .join("SKILL.md");
    std::fs::write(
        &skill_b,
        "---\nname: skill-b\ndescription: fresh new cross-workspace description\n---\nbody\n",
    )
    .unwrap();

    // ---- Cross-workspace dispatch (mirror of update::run) -----------------
    // For each workspace that enrols this URL, reindex its enabled
    // plugins. This is exactly what `commands::catalog::update::run`
    // does after refreshing per-URL.
    let conn = index::open_read_only(&paths.index_db).expect("open ro");
    let affected = workspace_catalogs::workspaces_with_catalog_url(&conn, &url)
        .expect("workspaces_with_catalog_url");
    drop(conn);
    assert_eq!(
        affected.len(),
        2,
        "both `global` and `second` must be in the dispatch list, got {affected:?}",
    );

    // Track aggregated modified+unchanged across both workspaces. The
    // FIRST workspace to reindex sees `modified=1, unchanged=3` (it
    // updates the shared skills row); the SECOND workspace observes
    // the row already reflects the new hash and reports
    // `modified=0, unchanged=4`. Both outcomes are correct — the
    // dispatch invariant we're testing is that BOTH workspaces are
    // visited and both end up pointing at the post-mutation hash.
    let mut modified_total = 0u32;
    let mut unchanged_total = 0u32;
    for (workspace, catalog_name) in affected {
        let scope = if workspace == "global" {
            Scope(WorkspaceName::global())
        } else {
            Scope(WorkspaceName::parse(&workspace).unwrap())
        };
        let conn = index::open_read_only(&paths.index_db).expect("open ro");
        let enabled = enabled_plugins_for_catalog(&conn, &workspace, &catalog_name)
            .expect("enabled_plugins_for_catalog");
        drop(conn);
        assert!(
            !enabled.is_empty(),
            "{workspace} should have enabled plugins"
        );

        let deps = LifecycleDeps {
            paths: &paths,
            scope: &scope,
            config: &config,
            embedder: &embedder,
            embedder_seed: stub_embedder_seed(),
            reranker_seed: stub_reranker_seed(),
            summariser_seed: stub_summariser_seed(),
            allow_model_download: false,
        };
        let outcome = reindex_catalog_plugins(&catalog_name, &enabled, &deps)
            .unwrap_or_else(|e| panic!("reindex {workspace}: {e}"));
        assert_eq!(
            outcome.plugins.len(),
            1,
            "plugin-alpha is the only enabled plugin in {workspace}",
        );
        let summary = outcome.plugins[0]
            .summary
            .expect("reindex must produce a summary, not an auto-disable");
        modified_total += summary.modified;
        unchanged_total += summary.unchanged;
    }
    // Across both workspaces, exactly ONE reindex must observe the
    // skill-b modification — whichever ran first. The other sees the
    // shared row already up-to-date.
    assert_eq!(
        modified_total, 1,
        "exactly one workspace reindex should report skill-b modified \
         (first writer wins; second observes shared row up-to-date)",
    );
    // Each workspace has 4 enabled skills → 8 total slots. 1 modified
    // + 7 unchanged covers them all.
    assert_eq!(
        unchanged_total, 7,
        "remaining skills (3 in first reindex + 4 in second) should be unchanged",
    );

    // ---- Post-condition: both workspaces see the new content hash ---------
    let hash_after_global = skill_content_hash_for(
        &paths,
        "global",
        "sample-plugin-catalog",
        "plugin-alpha",
        "skill-b",
    )
    .expect("skill-b row in global post-reindex");
    let hash_after_second = skill_content_hash_for(
        &paths,
        "second",
        "sample-plugin-catalog",
        "plugin-alpha",
        "skill-b",
    )
    .expect("skill-b row in second post-reindex");
    assert_ne!(
        hash_after_global, hash_before_global,
        "global must see the post-mutation hash",
    );
    assert_ne!(
        hash_after_second, hash_before_second,
        "second must see the post-mutation hash",
    );
    assert_eq!(
        hash_after_global, hash_after_second,
        "shared skills row → identical post-reindex hash across workspaces",
    );
}

//! Phase 4 / F11a — cheap re-enable invariant (FR-006) across workspaces.
//!
//! After F11a's scope-lift, enabling the same plugin in a second
//! workspace MUST NOT invoke the embedder again. The underlying skills
//! rows + embeddings are shared via the `workspace_skills` junction —
//! the second enable only UPSERTs the new `(workspace, skill)` row.
//!
//! The first user-visible surface for multi-workspace work is US2's
//! `tome workspace add`. Until that ships, we seed a second workspace
//! row directly into the central index DB and drive `enable_plugin_atomic`
//! against it. The semantics under test are real production behaviour —
//! the only test-only detail is how we get a non-`global` workspace
//! present in the `workspaces` table.

use crate::common::{
    config_with_catalog, copy_sample_plugin_catalog, enrol_catalog_symlinked, fabricate_models,
    lifecycle_paths, seed_workspace, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
};
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::index::{self, OpenOptions};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{Scope, WorkspaceName};

fn workspace_skill_count(paths: &tome::paths::Paths, workspace: &str) -> i64 {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open index");
    conn.query_row(
        "SELECT COUNT(*) FROM workspace_skills AS ws
         JOIN workspaces AS w ON w.id = ws.workspace_id
         WHERE w.name = ?1",
        rusqlite::params![workspace],
        |row| row.get(0),
    )
    .unwrap_or(0)
}

#[test]
fn re_enable_same_plugin_in_second_workspace_does_not_invoke_embedder() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = copy_sample_plugin_catalog(&tmp, "catalog");
    let config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    // FF1: enrol the catalog (+ cache symlink) for `global`; the `second`
    // workspace's enrolment is added once it is seeded below.
    enrol_catalog_symlinked(&paths, "global", "sample-plugin-catalog", &catalog_root);

    // Single embedder instance — its `call_count()` is the assertion target
    // across both enables.
    let embedder = StubEmbedder::new();

    // ---- enable in workspace `global` ----------------------------------
    let global_scope = Scope(WorkspaceName::global());
    let deps_global = LifecycleDeps {
        paths: &paths,
        scope: &global_scope,
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    let first = lifecycle::enable(&id, &deps_global).expect("first enable in global");
    assert_eq!(first.summary.total_skills, 4);
    assert_eq!(first.summary.newly_embedded, 4);
    let calls_after_first_enable = embedder.call_count();
    assert_eq!(
        calls_after_first_enable, 4,
        "first enable must invoke embedder once per skill",
    );

    // ---- seed a second workspace + enable into it ----------------------
    // US2's `tome workspace add` will own this step in production.
    seed_workspace(&paths, "second");
    enrol_catalog_symlinked(&paths, "second", "sample-plugin-catalog", &catalog_root);
    let second_name = WorkspaceName::parse("second").expect("valid workspace name");
    let second_scope = Scope(second_name);
    let deps_second = LifecycleDeps {
        paths: &paths,
        scope: &second_scope,
        config: &config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let second = lifecycle::enable(&id, &deps_second).expect("second enable in `second`");
    assert_eq!(second.summary.total_skills, 4);
    assert_eq!(
        second.summary.newly_embedded, 0,
        "cross-workspace re-enable must report zero newly-embedded skills",
    );
    assert_eq!(
        embedder.call_count(),
        calls_after_first_enable,
        "cross-workspace cheap re-enable must not invoke the embedder",
    );

    // ---- both workspaces hold their own enrolment rows -----------------
    assert_eq!(
        workspace_skill_count(&paths, "global"),
        4,
        "global workspace must keep its 4 enrolments",
    );
    assert_eq!(
        workspace_skill_count(&paths, "second"),
        4,
        "second workspace must have 4 enrolments mirroring the same skills",
    );
}

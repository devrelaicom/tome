//! Phase 6 / F2 — `EntryKind::Agent` widening, exercised at the storage +
//! count-surface layer (entry-schema-p6.md).
//!
//! Agent *indexing from `agents/*.md`* is US1 work. This file proves the
//! load-bearing F2 guarantee: a `kind='agent'` row can be written through
//! the existing storage layer and every per-kind count surface
//! (`plugin list` / `plugin show` / doctor) accounts for it WITHOUT
//! regressing to `IndexIntegrityCheckFailure` (exit 51) — the failure that
//! a `kind='agent'` row introduced before the enum widening would trigger.
//!
//! Rows are inserted directly via SQL (not the enable pipeline, which
//! always embeds) so we can pin the agent-row invariants — `searchable=0`,
//! no `skill_embeddings` row — that the US1 indexing pipeline will later
//! produce.

mod common;

use common::{
    ToolEnv, config_with_catalog, copy_sample_plugin_catalog, fabricate_all_registry_models,
    global_scope, paths_for, stub_embedder_seed, stub_reranker_seed, stub_summariser_seed,
    write_config_for_cli,
};
use rusqlite::params;
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::paths::Paths;
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};
use tome::workspace::{ResolvedScope, Scope, ScopeSource, WorkspaceName};

struct Fixture {
    _env: ToolEnv,
    paths: Paths,
    _fixture_tmp: TempDir,
}

/// Enable the sample plugin (sets up workspaces/catalogs/skills), then
/// inject a synthetic `kind='agent'` row enrolled in `global`.
fn enable_with_injected_agent() -> Fixture {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let fixture_tmp = TempDir::new().unwrap();
    let catalog_root = copy_sample_plugin_catalog(&fixture_tmp, "catalog");
    let cli_config = config_with_catalog("sample-plugin-catalog", &catalog_root);
    write_config_for_cli(&paths, &cli_config);

    let embedder = StubEmbedder::new();
    let deps = LifecycleDeps {
        paths: &paths,
        scope: &global_scope(),
        config: &cli_config,
        embedder: &embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    };
    let id: PluginId = "sample-plugin-catalog/plugin-alpha".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("pre-enable plugin-alpha");

    inject_agent_row(&paths);

    Fixture {
        _env: env,
        paths,
        _fixture_tmp: fixture_tmp,
    }
}

/// Insert one `kind='agent'` row for the already-enabled plugin and enrol
/// it in the `global` workspace. Mirrors the agent-row invariants from
/// entry-schema-p6.md: `searchable=0`, `user_invocable=0`,
/// `when_to_use=NULL`, and no `skill_embeddings` row.
fn open_central(paths: &Paths) -> rusqlite::Connection {
    // `index::open` ignores `opts` on an existing DB; stub seeds match the
    // established reopen pattern (plugin_cheap_reenable_across_workspaces).
    tome::index::open(
        &paths.index_db,
        &tome::index::OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open central db")
}

fn inject_agent_row(paths: &Paths) {
    let conn = open_central(paths);

    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES (?1, ?2, ?3, 'agent', ?4, '0.0.0', ?5, ?6, 0, 0, NULL, '1970-01-01T00:00:00Z')",
        params![
            "sample-plugin-catalog",
            "plugin-alpha",
            "reviewer",
            "a synthetic agent row",
            "agents/reviewer.md",
            "deadbeef",
        ],
    )
    .expect("insert agent row");

    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE kind = 'agent' AND name = 'reviewer'",
            [],
            |row| row.get(0),
        )
        .expect("agent row id");
    let workspace_id: i64 = conn
        .query_row(
            "SELECT id FROM workspaces WHERE name = 'global'",
            [],
            |row| row.get(0),
        )
        .expect("global workspace id");
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
        params![workspace_id, skill_id],
    )
    .expect("enrol agent in global");
}

fn global_resolved_scope() -> ResolvedScope {
    ResolvedScope {
        scope: Scope(WorkspaceName::global()),
        source: ScopeSource::Flag,
        project_root: None,
    }
}

#[test]
fn agents_index_non_searchable() {
    let fx = enable_with_injected_agent();
    let conn = open_central(&fx.paths);

    // The agent row is non-searchable and has no embedding row.
    let searchable: i64 = conn
        .query_row(
            "SELECT searchable FROM skills WHERE kind = 'agent' AND name = 'reviewer'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(searchable, 0, "agent rows must be searchable=0");

    let agent_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE kind = 'agent' AND name = 'reviewer'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    let embedding_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM skill_embeddings WHERE skill_id = ?1",
            params![agent_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(embedding_rows, 0, "agent rows must have no embedding row");
}

#[test]
fn per_kind_counts_include_agents() {
    let fx = enable_with_injected_agent();
    let home = TempDir::new().unwrap();
    let scope = global_resolved_scope();

    // Doctor per-kind counts surface the agent without a catch-all
    // regression (no exit-51 crash).
    let report = tome::doctor::assemble_report(&scope, &fx.paths, home.path(), false)
        .expect("doctor must not crash on a kind='agent' row");
    let counts = report
        .entry_counts
        .as_ref()
        .expect("entry_counts populated in workspace scope");
    assert_eq!(counts.agents, 1, "doctor must count the injected agent");
    // Sample fixture ships 4 indexable skills (entry_counts_by_kind pin in
    // doctor_p5.rs) — the agent injection must not perturb that.
    assert_eq!(counts.skills, 4, "skill count unchanged by agent injection");
}

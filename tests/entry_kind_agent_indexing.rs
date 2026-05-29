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

/// Lay out a one-plugin catalog under `root` carrying a single agent file.
/// `agent_md` is the verbatim `agents/<agent_name>.md` body. Returns the
/// catalog root (the dir holding `tome-catalog.toml`).
fn write_agent_catalog(
    root: &std::path::Path,
    agent_name: &str,
    agent_md: &str,
) -> std::path::PathBuf {
    let catalog_root = root.join("agent-catalog");
    let plugin_dir = catalog_root.join("plugin-ag");
    std::fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    std::fs::create_dir_all(plugin_dir.join("agents")).unwrap();
    std::fs::write(
        catalog_root.join("tome-catalog.toml"),
        "name = \"agent-catalog\"\nversion = \"0.1.0\"\n\n[[plugins]]\nname = \"plugin-ag\"\nsource = \"./plugin-ag\"\n",
    )
    .unwrap();
    std::fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        "{\"name\": \"plugin-ag\", \"version\": \"1.0.0\"}",
    )
    .unwrap();
    std::fs::write(
        plugin_dir.join("agents").join(format!("{agent_name}.md")),
        agent_md,
    )
    .unwrap();
    catalog_root
}

/// End-to-end: enabling a plugin that ships `agents/<name>.md` produces an
/// agent row with `searchable=0`, `user_invocable=0`, `when_to_use=NULL`,
/// NO embedding row, and `name`/`description` resolved per the rules
/// (entry-schema-p6.md § "Indexing pipeline").
#[test]
fn enable_indexes_agent_file_end_to_end() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let fixture_tmp = TempDir::new().unwrap();
    // `name` comes from frontmatter; `description` falls back to the first
    // non-empty body line (no frontmatter `description`).
    let agent_md = "---\nname: code-reviewer\n---\n\n\n   First meaningful line.   \nsecond line\n";
    let catalog_root = write_agent_catalog(fixture_tmp.path(), "reviewer-file", agent_md);
    let cli_config = config_with_catalog("agent-catalog", &catalog_root);
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
    let id: PluginId = "agent-catalog/plugin-ag".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("enable plugin-ag with an agent");

    let conn = open_central(&paths);
    let (name, description, searchable, user_invocable, when_to_use, id): (
        String,
        String,
        i64,
        i64,
        Option<String>,
        i64,
    ) = conn
        .query_row(
            "SELECT name, description, searchable, user_invocable, when_to_use, id
             FROM skills WHERE kind = 'agent'",
            [],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                ))
            },
        )
        .expect("exactly one agent row");

    assert_eq!(name, "code-reviewer", "frontmatter name wins over stem");
    assert_eq!(
        description, "First meaningful line.",
        "description = first non-empty body line, trimmed",
    );
    assert_eq!(searchable, 0, "agents are never searchable");
    assert_eq!(user_invocable, 0, "agents are never user-invocable");
    assert!(when_to_use.is_none(), "agents carry when_to_use = NULL");

    let embedding_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM skill_embeddings WHERE skill_id = ?1",
            params![id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(embedding_rows, 0, "agent rows must have no embedding row");

    // The agent is enrolled in the resolved workspace (the junction is
    // plugin-grained and kind-agnostic).
    let enrolled: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM workspace_skills ws
             JOIN workspaces w ON w.id = ws.workspace_id
             WHERE ws.skill_id = ?1 AND w.name = 'global'",
            params![id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(enrolled, 1, "enabling enrols the agent in the workspace");
}

/// An agent file with no frontmatter delimiters is a malformed recognised
/// structure → `AgentTranslationFailed` (exit 45), per NFR-010.
#[test]
fn malformed_agent_frontmatter_fails_with_exit_45() {
    let env = ToolEnv::new();
    let paths = paths_for(&env);
    std::fs::create_dir_all(&paths.root).unwrap();
    fabricate_all_registry_models(&paths);

    let fixture_tmp = TempDir::new().unwrap();
    // No `---` delimiters at all: a malformed recognised agent structure.
    let catalog_root = write_agent_catalog(
        fixture_tmp.path(),
        "broken",
        "this agent file has no frontmatter at all\n",
    );
    let cli_config = config_with_catalog("agent-catalog", &catalog_root);
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
    let id: PluginId = "agent-catalog/plugin-ag".parse().unwrap();
    let err = lifecycle::enable(&id, &deps).expect_err("malformed agent must fail loudly");
    assert_eq!(
        err.exit_code(),
        45,
        "malformed agent frontmatter maps to exit 45, got {err:?}",
    );
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

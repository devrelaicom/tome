//! Phase 5 / US1.a — kind-discriminated entry indexing.
//!
//! Exercises `lifecycle::enable` against a hand-rolled fixture plugin
//! that ships BOTH `skills/*/SKILL.md` and `commands/*.md`. Verifies the
//! schema-v3 column shape, identity-tuple widening, and the
//! `when_to_use`-aware embedding/content-hash composition.

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::params;
use tempfile::TempDir;
use tome::embedding::stub::StubEmbedder;
use tome::index::{self, OpenOptions};
use tome::plugin::PluginId;
use tome::plugin::lifecycle::{self, LifecycleDeps};

use common::{
    config_with_catalog, fabricate_models, lifecycle_paths, stub_embedder_seed, stub_reranker_seed,
    stub_summariser_seed,
};

fn build_deps<'a>(
    paths: &'a tome::paths::Paths,
    config: &'a tome::config::Config,
    embedder: &'a StubEmbedder,
    scope: &'a tome::workspace::Scope,
) -> LifecycleDeps<'a> {
    LifecycleDeps {
        paths,
        scope,
        config,
        embedder,
        embedder_seed: stub_embedder_seed(),
        reranker_seed: stub_reranker_seed(),
        summariser_seed: stub_summariser_seed(),
        allow_model_download: false,
    }
}

/// Lay out a plugin on disk under `catalog_root/<plugin>/` with the
/// supplied skills and commands. Both lists are `(name, contents)`.
/// Skills go under `skills/<name>/SKILL.md`; commands go under
/// `commands/<name>.md`.
fn write_plugin_with_kinds(
    catalog_root: &Path,
    plugin_name: &str,
    skills: &[(&str, &str)],
    commands: &[(&str, &str)],
) -> PathBuf {
    let plugin_dir = catalog_root.join(plugin_name);
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        format!(r#"{{"name": "{plugin_name}", "version": "1.0.0"}}"#),
    )
    .unwrap();

    for (name, body) in skills {
        let dir = plugin_dir.join("skills").join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("SKILL.md"), body).unwrap();
    }
    if !commands.is_empty() {
        let cmd_dir = plugin_dir.join("commands");
        fs::create_dir_all(&cmd_dir).unwrap();
        for (name, body) in commands {
            fs::write(cmd_dir.join(format!("{name}.md")), body).unwrap();
        }
    }

    plugin_dir
}

fn good_skill(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\nbody\n")
}

fn good_command(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\ncommand body\n")
}

fn count_by_kind(paths: &tome::paths::Paths, kind: &str) -> i64 {
    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open");
    conn.query_row(
        "SELECT COUNT(*) FROM skills WHERE kind = ?1",
        params![kind],
        |row| row.get(0),
    )
    .expect("count")
}

#[test]
fn both_directories_index_to_unified_table() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);
    write_plugin_with_kinds(
        &catalog_root,
        "plug",
        &[
            ("alpha", &good_skill("alpha", "first skill")),
            ("beta", &good_skill("beta", "second skill")),
        ],
        &[
            ("run", &good_command("run", "run the thing")),
            ("clean", &good_command("clean", "clean the thing")),
        ],
    );

    let embedder = StubEmbedder::new();
    let scope = tome::workspace::Scope(tome::workspace::WorkspaceName::global());
    let deps = build_deps(&paths, &config, &embedder, &scope);
    let id: PluginId = "acme/plug".parse().unwrap();
    let outcome = lifecycle::enable(&id, &deps).expect("enable both kinds");

    assert_eq!(
        outcome.summary.total_skills, 4,
        "two skills + two commands = four entries",
    );
    assert_eq!(count_by_kind(&paths, "skill"), 2);
    assert_eq!(count_by_kind(&paths, "command"), 2);
}

#[test]
fn same_name_different_kind_produces_two_rows() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);
    write_plugin_with_kinds(
        &catalog_root,
        "plug",
        &[("widget", &good_skill("widget", "skill version"))],
        &[("widget", &good_command("widget", "command version"))],
    );

    let embedder = StubEmbedder::new();
    let scope = tome::workspace::Scope(tome::workspace::WorkspaceName::global());
    let deps = build_deps(&paths, &config, &embedder, &scope);
    let id: PluginId = "acme/plug".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("enable both kinds");

    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .unwrap();
    let mut stmt = conn
        .prepare("SELECT kind, name FROM skills ORDER BY kind, name")
        .unwrap();
    let rows: Vec<(String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            ("command".to_owned(), "widget".to_owned()),
            ("skill".to_owned(), "widget".to_owned()),
        ],
    );
}

#[test]
fn enable_synchronises_both_kinds_into_workspace_skills_junction() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);
    write_plugin_with_kinds(
        &catalog_root,
        "plug",
        &[("only-skill", &good_skill("only-skill", "skill"))],
        &[("only-cmd", &good_command("only-cmd", "command"))],
    );

    let embedder = StubEmbedder::new();
    let scope = tome::workspace::Scope(tome::workspace::WorkspaceName::global());
    let deps = build_deps(&paths, &config, &embedder, &scope);
    let id: PluginId = "acme/plug".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("enable");

    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .unwrap();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*)
             FROM workspace_skills ws
             JOIN skills s ON s.id = ws.skill_id
             JOIN workspaces w ON w.id = ws.workspace_id
             WHERE w.name = 'global' AND s.catalog = 'acme' AND s.plugin = 'plug'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        count, 2,
        "workspace_skills must enrol BOTH the skill row and the command row",
    );
}

#[test]
fn when_to_use_contributes_to_embedding_text() {
    use tome::index::skills::embedding_text;
    let with = embedding_text("foo", "desc", Some("Use it well"));
    let without = embedding_text("foo", "desc", None);
    assert!(
        with.contains("When to use: Use it well"),
        "embedding_text must include the `When to use:` line when present; got {with:?}",
    );
    assert!(
        !without.contains("When to use:"),
        "embedding_text must omit the `When to use:` line when absent; got {without:?}",
    );
    assert_eq!(without, "foo\n\ndesc");
}

#[test]
fn content_hash_invalidates_when_when_to_use_changes() {
    use tome::index::skills::content_hash;
    let a = content_hash("foo", "desc", None);
    let b = content_hash("foo", "desc", Some("Use it well"));
    let c = content_hash("foo", "desc", Some("Different guidance"));
    assert_ne!(a, b, "adding when_to_use must change content_hash");
    assert_ne!(b, c, "changing when_to_use must change content_hash");
}

#[test]
fn frontmatter_when_to_use_round_trips_through_enable_to_db() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);
    let plugin_dir = catalog_root.join("plug");
    fs::create_dir_all(plugin_dir.join(".claude-plugin")).unwrap();
    fs::write(
        plugin_dir.join(".claude-plugin").join("plugin.json"),
        r#"{"name": "plug", "version": "1.0.0"}"#,
    )
    .unwrap();
    let skill_dir = plugin_dir.join("skills").join("guided");
    fs::create_dir_all(&skill_dir).unwrap();
    fs::write(
        skill_dir.join("SKILL.md"),
        "---\n\
         name: guided\n\
         description: a guided skill\n\
         when_to_use: When the user asks for guidance\n\
         ---\n\
         body\n",
    )
    .unwrap();

    let embedder = StubEmbedder::new();
    let scope = tome::workspace::Scope(tome::workspace::WorkspaceName::global());
    let deps = build_deps(&paths, &config, &embedder, &scope);
    let id: PluginId = "acme/plug".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("enable");

    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .unwrap();
    let when_to_use: Option<String> = conn
        .query_row(
            "SELECT when_to_use FROM skills WHERE catalog='acme' AND plugin='plug' AND name='guided'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        when_to_use.as_deref(),
        Some("When the user asks for guidance"),
        "when_to_use must round-trip from frontmatter into the DB column",
    );
}

#[test]
fn searchable_and_user_invocable_defaults_per_kind_persist_to_db() {
    let tmp = TempDir::new().unwrap();
    let paths = lifecycle_paths(tmp.path());
    fs::create_dir_all(&paths.root).unwrap();
    fabricate_models(&paths);

    let catalog_root = tmp.path().join("catalog");
    fs::create_dir_all(&catalog_root).unwrap();
    let config = config_with_catalog("acme", &catalog_root);
    write_plugin_with_kinds(
        &catalog_root,
        "plug",
        &[("my-skill", &good_skill("my-skill", "skill"))],
        &[("my-cmd", &good_command("my-cmd", "command"))],
    );

    let embedder = StubEmbedder::new();
    let scope = tome::workspace::Scope(tome::workspace::WorkspaceName::global());
    let deps = build_deps(&paths, &config, &embedder, &scope);
    let id: PluginId = "acme/plug".parse().unwrap();
    lifecycle::enable(&id, &deps).expect("enable");

    let conn = index::open(
        &paths.index_db,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .unwrap();
    let mut stmt = conn
        .prepare("SELECT kind, searchable, user_invocable FROM skills ORDER BY kind")
        .unwrap();
    let rows: Vec<(String, i64, i64)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        rows,
        vec![
            // Commands default to user_invocable=1.
            ("command".to_owned(), 1, 1),
            // Skills default to user_invocable=0.
            ("skill".to_owned(), 1, 0),
        ],
    );
}

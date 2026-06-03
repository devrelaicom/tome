//! SC-001 one-off gate (task T054): the filtered-KNN recall invariant
//! (`min(top_k, total matching entries)`) holds against the **real** BGE
//! embedding model on a realistically-populated, multi-workspace index — not
//! just the deterministic stub used by `tests/search_knn_recall.rs`.
//!
//! ## Why `#[ignore]` and not part of fast CI
//!
//! The real `bge-small-en-v1.5` embedder (~45 MB INT8) plus the
//! `bge-reranker-base` (~280 MB INT8) are downloaded from Hugging Face on
//! first run (~325 MB total, network-bound). Fast CI runs the stub-embedder
//! suite (`search_knn_recall`) which proves the over-fetch + widen mechanism
//! deterministically; this test is the *one-time* confirmation that the same
//! invariant survives real-model neighbour orderings (where excluded rows can
//! legitimately be nearer than a genuine cross-workspace / cross-catalog
//! match). It is run manually before a release:
//!
//! ```sh
//! cargo test --test search_knn_recall_realmodel -- --ignored --nocapture
//! ```
//!
//! On the first run it downloads ~325 MB into a temp dir and discards it when
//! the test ends.

use crate::common::{lifecycle_paths, stub_reranker_seed, stub_summariser_seed};
use rusqlite::{Connection, params};
use tempfile::TempDir;
use tome::embedding::Embedder;
use tome::embedding::download::download_model;
use tome::embedding::fastembed::FastembedEmbedder;
use tome::embedding::registry::{MODEL_REGISTRY, ModelEntry, ModelKind};
use tome::index::query::QueryFilters;
use tome::index::{self, MetaSeed, OpenOptions, knn};
use tome::paths::Paths;

/// Locate a registry entry by kind. Panics with a clear message if the
/// registry shape drifts (the test is a release gate; a missing entry is a
/// hard error, not a skip).
fn registry_entry(kind: ModelKind) -> &'static ModelEntry {
    MODEL_REGISTRY
        .iter()
        .find(|e| e.kind == kind)
        .unwrap_or_else(|| panic!("no {kind:?} entry in MODEL_REGISTRY"))
}

/// Download the real embedder into `paths.models_dir`, surfacing progress so
/// the operator sees the ~325 MB transfer is alive on a cold cache.
fn download_real_embedder(paths: &Paths) -> &'static ModelEntry {
    let entry = registry_entry(ModelKind::Embedder);
    eprintln!(
        "downloading {} (~{} MB) — first run only",
        entry.name,
        entry.size_bytes / 1_000_000,
    );
    let progress = |bytes: u64, total: u64| {
        if total > 0 && bytes % (25 * 1024 * 1024) < 64 * 1024 {
            eprintln!("  ... {} / {} MB", bytes / 1_000_000, total / 1_000_000);
        }
    };
    download_model(entry, &paths.models_dir, Some(&progress)).expect("download embedder");
    entry
}

/// `MetaSeed` for the real embedder so the index `meta` identity matches the
/// model actually used to populate it (re-opens are first-writer-wins).
fn embedder_seed(entry: &ModelEntry) -> MetaSeed {
    MetaSeed {
        name: entry.name.to_owned(),
        version: entry.version.to_owned(),
    }
}

/// Insert one real-embedded skill row and enrol it in the named workspace.
/// `searchable` toggles the post-JOIN exclusion the recall invariant must
/// see through.
#[allow(clippy::too_many_arguments)]
fn insert_real_row(
    conn: &Connection,
    embedder: &FastembedEmbedder,
    ws_id: i64,
    catalog: &str,
    plugin: &str,
    name: &str,
    text: &str,
    searchable: bool,
) {
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES (?1, ?2, ?3, 'skill', ?4, '0.0.0', ?5, ?6, ?7, 0, NULL, 0)",
        params![
            catalog,
            plugin,
            name,
            text,
            format!("skills/{name}/SKILL.md"),
            format!("hash-{catalog}-{plugin}-{name}"),
            searchable as i64,
        ],
    )
    .expect("insert skill");
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind='skill' AND name=?3",
            params![catalog, plugin, name],
            |r| r.get(0),
        )
        .expect("skill id");

    let vec = embedder.embed(text).expect("embed");
    let mut bytes = Vec::with_capacity(vec.len() * 4);
    for f in &vec {
        bytes.extend_from_slice(&f.to_le_bytes());
    }
    conn.execute(
        "INSERT INTO skill_embeddings (skill_id, embedding) VALUES (?1, ?2)",
        params![skill_id, bytes],
    )
    .expect("insert embedding");
    conn.execute(
        "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
        params![ws_id, skill_id],
    )
    .expect("enrol skill");
}

fn workspace_id(conn: &Connection, name: &str) -> i64 {
    conn.query_row(
        "SELECT id FROM workspaces WHERE name = ?1",
        params![name],
        |r| r.get(0),
    )
    .unwrap_or_else(|e| panic!("workspace `{name}` id: {e}"))
}

/// Seed a non-`global` workspace row (US2 will own this seam; until then the
/// test inserts it directly, matching `tests/crate::common::seed_workspace`).
fn seed_workspace(conn: &Connection, name: &str) {
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    conn.execute(
        "INSERT INTO workspaces (name, created_at, last_used_at) VALUES (?1, ?2, ?2)",
        params![name, now],
    )
    .expect("seed workspace");
}

#[test]
#[ignore = "SC-001 release gate: downloads ~325 MB of real BGE models; run with --ignored"]
fn real_model_filtered_knn_returns_min_top_k_matches() {
    let tmp = TempDir::new().expect("tempdir");
    let paths = lifecycle_paths(tmp.path());
    std::fs::create_dir_all(&paths.models_dir).expect("create models_dir");

    let entry = download_real_embedder(&paths);
    let embedder = FastembedEmbedder::load(entry, &paths.models_dir.join(entry.name))
        .expect("load real embedder");

    let db_path = paths.index_db.clone();
    let conn = index::open(
        &db_path,
        &OpenOptions {
            embedder: embedder_seed(entry),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
        },
    )
    .expect("open index");

    // Two workspaces: the privileged `global` (auto-seeded by bootstrap) and
    // a second `project-x`. The recall invariant is scoped per workspace, so
    // `project-x` rows must never leak into a `global` query and vice versa.
    seed_workspace(&conn, "project-x");
    let global = workspace_id(&conn, "global");
    let project_x = workspace_id(&conn, "project-x");

    // Target: a small set of genuine matches in the `target` catalog,
    // enrolled in `global`, semantically about "database migrations".
    let target_texts = [
        (
            "apply-migration",
            "apply a pending database schema migration",
        ),
        (
            "rollback-migration",
            "roll back the most recent database migration",
        ),
        (
            "migration-status",
            "report the database migration status and history",
        ),
    ];
    for (name, text) in target_texts {
        insert_real_row(
            &conn, &embedder, global, "target", "db-tools", name, text, true,
        );
    }

    // Nearer-or-comparable distractors that the filters must exclude:
    //  - same topic, WRONG catalog (`other`) → excluded by `--catalog target`
    //  - same topic, searchable = 0          → excluded by the searchable gate
    //  - same topic, enrolled only in `project-x` → excluded by workspace
    for i in 0..8 {
        insert_real_row(
            &conn,
            &embedder,
            global,
            "other",
            "rival",
            &format!("rival-migrate-{i}"),
            "perform a database migration step in a rival catalog",
            true,
        );
        insert_real_row(
            &conn,
            &embedder,
            global,
            "target",
            "hidden",
            &format!("hidden-migrate-{i}"),
            "internal database migration helper, not searchable",
            false,
        );
        insert_real_row(
            &conn,
            &embedder,
            project_x,
            "target",
            "elsewhere",
            &format!("ws-migrate-{i}"),
            "database migration tool bound to another workspace",
            true,
        );
    }

    let top_k = 3u32;
    let filters = QueryFilters {
        catalog: Some("target"),
        plugin: None,
    };
    let query_vec = embedder
        .embed("how do I run a database migration")
        .expect("embed query");

    let hits = knn(&conn, "global", &query_vec, top_k, &filters).expect("knn");

    // min(top_k, total matches) = min(3, 3) = 3, despite same-topic rivals
    // sitting at comparable real-model distances.
    assert_eq!(
        hits.len(),
        top_k as usize,
        "real-model recall short: expected {top_k}, got {} ({hits:#?})",
        hits.len(),
    );
    // No excluded class leaked: every hit is in `target`, searchable, and a
    // genuine `db-tools` match (the rival/hidden/elsewhere plugins are out).
    for c in &hits {
        assert_eq!(c.catalog, "target", "wrong-catalog row leaked: {}", c.name);
        assert_eq!(
            c.plugin, "db-tools",
            "non-target plugin `{}` leaked into result",
            c.plugin,
        );
    }
    let names: std::collections::HashSet<&str> = hits.iter().map(|c| c.name.as_str()).collect();
    for (name, _) in target_texts {
        assert!(
            names.contains(name),
            "genuine match `{name}` missing from real-model result {names:?}",
        );
    }
}

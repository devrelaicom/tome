//! Regression suite for FR-001 / F-KNN: filtered KNN must return exactly
//! `min(top_k, total matching entries)`, regardless of how many *nearer*
//! vectors are excluded by the workspace / `searchable` / `--catalog` /
//! `--plugin` post-JOIN filters.
//!
//! ## Schema v6 / Phase p11 update
//!
//! `skill_embeddings` is now a plain BLOB table; `knn` uses
//! `vec_distance_cosine()` scalar rather than the old `vec0 MATCH k = ?`
//! virtual-table scan. Because the catalog / `searchable` / workspace
//! filters are applied in the JOIN *before* `LIMIT ?3`, the old over-fetch /
//! widen hazard no longer exists at the SQL level — the LIMIT applies to
//! the already-filtered result set. The tests below verify the end-to-end
//! correctness property (the right rows come back) not the widen mechanism.
//!
//! ## Fixture strategy (no real model, fully deterministic)
//!
//! We bypass the stub embedder's text→vector hashing and write embeddings
//! DIRECTLY to control each row's cosine-distance rank exactly.
//! `vec_distance_cosine` requires non-zero vectors (it returns NULL for
//! zero-magnitude inputs). Fixture uses two orthogonal axes:
//!
//! * **Decoys** → axis-0 unit vectors: cosine distance from the axis-0
//!   query ≈ 0 (nearer / more similar).
//! * **Matches** → axis-1 unit vectors: cosine distance from the axis-0
//!   query = 1.0 (orthogonal / farther).
//!
//! Decoys are uniformly nearer than every match in cosine space, so the
//! SQL must correctly apply the catalog filter *before* the LIMIT to return
//! any matches at all.

use crate::common::{stub_embedder_seed, stub_reranker_seed, stub_summariser_seed};
use rusqlite::{Connection, params};
use tempfile::TempDir;
use tome::index::query::{Candidate, QueryFilters};
use tome::index::{self, OpenOptions, knn};
use tome::plugin::identity::EntryKind;

const DIM: usize = 384;
const MATCH_CATALOG: &str = "match-cat";
const DECOY_CATALOG: &str = "decoy-cat";

/// Encode a 384-dim **decoy** vector: unit vector along axis-0.
/// Cosine distance from the axis-0 query vector = 0 (identical direction).
/// All decoys share the same direction; magnitude > 0 to avoid NULL from
/// `vec_distance_cosine`.
fn decoy_vector_bytes() -> Vec<u8> {
    let mut v = vec![0.0f32; DIM];
    v[0] = 1.0;
    let mut out = Vec::with_capacity(DIM * 4);
    for f in &v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// Encode a 384-dim **match** vector: unit vector along axis-1.
/// Cosine distance from the axis-0 query vector = 1.0 (orthogonal).
/// All matches share the same direction so they all lie at the same
/// distance from the query, farther than every decoy.
fn match_vector_bytes() -> Vec<u8> {
    let mut v = vec![0.0f32; DIM];
    v[1] = 1.0;
    let mut out = Vec::with_capacity(DIM * 4);
    for f in &v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// The query vector: unit vector along axis-0. Decoys are near (distance ≈ 0);
/// matches are orthogonal (distance = 1.0). Both are strictly non-zero so
/// `vec_distance_cosine` never returns NULL.
fn query_vector() -> Vec<f32> {
    let mut v = vec![0.0f32; DIM];
    v[0] = 1.0;
    v
}

/// Open a fresh on-disk index (full schema + seeded `global` workspace) and
/// return the live connection plus the temp dir that must outlive it.
fn fresh_index() -> (TempDir, Connection) {
    let tmp = TempDir::new().expect("tempdir");
    let db_path = tmp.path().join("index.sqlite3");
    let conn = index::open(
        &db_path,
        &OpenOptions {
            embedder: stub_embedder_seed(),
            reranker: stub_reranker_seed(),
            summariser: stub_summariser_seed(),
            profile: None,
        },
    )
    .expect("open index");
    (tmp, conn)
}

fn global_ws_id(conn: &Connection) -> i64 {
    conn.query_row("SELECT id FROM workspaces WHERE name = 'global'", [], |r| {
        r.get(0)
    })
    .expect("global workspace id")
}

/// Insert one skill row + its embedding at L2 distance `magnitude` from the
/// origin query, and (optionally) enrol it in the `global` workspace.
///
/// `searchable` and `enrol` are independent so a single helper can fabricate
/// every exclusion class the post-JOIN filters care about: wrong catalog
/// (via `catalog`), `searchable = 0`, and "not enrolled in the workspace".
/// `kind` is the `s.kind` column literal (`skill`/`command`/`agent`) so the
/// `--kind` (`IN (...)`) filter can be exercised across kinds.
#[allow(clippy::too_many_arguments)]
fn insert_row(
    conn: &Connection,
    ws_id: i64,
    catalog: &str,
    plugin: &str,
    name: &str,
    kind: &str,
    embedding: Vec<u8>,
    searchable: bool,
    enrol: bool,
) {
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES (?1, ?2, ?3, ?7, 'desc', '0.0.0', ?4, ?5, ?6, 0, NULL, 0)",
        params![
            catalog,
            plugin,
            name,
            format!("skills/{name}/SKILL.md"),
            // Unique content_hash per row keeps the (intentionally loose)
            // hash index honest; not load-bearing for the query.
            format!("hash-{catalog}-{name}"),
            searchable as i64,
            kind,
        ],
    )
    .expect("insert skill");
    let skill_id: i64 = conn
        .query_row(
            "SELECT id FROM skills WHERE catalog=?1 AND plugin=?2 AND kind=?4 AND name=?3",
            params![catalog, plugin, name, kind],
            |r| r.get(0),
        )
        .expect("skill id");
    conn.execute(
        "INSERT INTO skill_embeddings (skill_id, embedding) VALUES (?1, ?2)",
        params![skill_id, embedding],
    )
    .expect("insert embedding");
    if enrol {
        conn.execute(
            "INSERT INTO workspace_skills (workspace_id, skill_id, enabled_at) VALUES (?1, ?2, 0)",
            params![ws_id, skill_id],
        )
        .expect("enrol skill");
    }
}

fn total_embeddings(conn: &Connection) -> i64 {
    conn.query_row("SELECT COUNT(*) FROM skill_embeddings", [], |r| r.get(0))
        .expect("count embeddings")
}

/// Build a corpus of `n_decoys` *nearer* rows excluded by the catalog filter
/// plus `n_matches` *farther* rows that DO satisfy every filter. Decoys are
/// axis-0 unit vectors (cosine distance ≈ 0 from the query); matches are
/// axis-1 unit vectors (cosine distance = 1.0, orthogonal to query). With
/// `vec_distance_cosine` the decoys are uniformly nearer than the matches.
///
/// Returns the set of match names for membership assertions.
fn build_corpus(conn: &Connection, n_decoys: usize, n_matches: usize) -> Vec<String> {
    let ws = global_ws_id(conn);

    // Nearer-than-everything decoys, all in the WRONG catalog → excluded by
    // `QueryFilters{ catalog: MATCH_CATALOG }`. Enrolled + searchable so the
    // *only* thing keeping them out of the result is the catalog filter.
    // Axis-0 vectors → cosine distance ≈ 0 (near the query).
    for i in 0..n_decoys {
        insert_row(
            conn,
            ws,
            DECOY_CATALOG,
            "decoy-plugin",
            &format!("decoy-{i}"),
            "skill",
            decoy_vector_bytes(),
            true,
            true,
        );
    }

    // Genuine matches, all farther than every decoy (axis-1 → cosine dist = 1.0).
    let mut names = Vec::with_capacity(n_matches);
    for i in 0..n_matches {
        let name = format!("match-{i}");
        insert_row(
            conn,
            ws,
            MATCH_CATALOG,
            "match-plugin",
            &name,
            "skill",
            match_vector_bytes(),
            true,
            true,
        );
        names.push(name);
    }
    names
}

fn run_knn(conn: &Connection, top_k: u32) -> Vec<Candidate> {
    let filters = QueryFilters {
        catalogs: vec![MATCH_CATALOG],
        ..Default::default()
    };
    knn(conn, "global", &query_vector(), top_k, &filters).expect("knn")
}

/// CORE REGRESSION (FR-001). The corpus has 60 decoys (axis-0, cosine dist ≈ 0)
/// nearer than 5 genuine matches (axis-1, cosine dist = 1.0), all filtered by
/// catalog. With schema-v6 JOIN-level filtering, `vec_distance_cosine` orders
/// all 65 vectors and the catalog filter is applied before LIMIT, so all 5
/// matches are always returned regardless of decoy count.
#[test]
fn filtered_knn_returns_top_k_despite_many_nearer_excluded_vectors() {
    let (_tmp, conn) = fresh_index();
    let top_k = 5u32;
    let n_decoys = 60; // >> top_k * 4 (= 20): defeats the fixed over-fetch
    let match_names = build_corpus(&conn, n_decoys, top_k as usize);

    let hits = run_knn(&conn, top_k);

    // min(top_k, total matches) = min(5, 5) = 5.
    assert_eq!(
        hits.len(),
        top_k as usize,
        "expected exactly min(top_k, matches)={top_k} hits, got {} — \
         the nearer decoys starved the candidate window",
        hits.len(),
    );
    // Every genuine match must be present; no decoy may leak in.
    let returned: std::collections::HashSet<&str> = hits.iter().map(|c| c.name.as_str()).collect();
    for name in &match_names {
        assert!(
            returned.contains(name.as_str()),
            "genuine match `{name}` missing from result {returned:?}",
        );
    }
    for c in &hits {
        assert_eq!(
            c.catalog, MATCH_CATALOG,
            "catalog filter breached — decoy `{}` leaked in",
            c.name,
        );
    }
}

/// MONOTONICITY: the recovered count must NOT shrink as the corpus of nearer
/// excluded decoys grows. With a fixed multiplier, more decoys = fewer (then
/// zero) surviving matches; the widen loop keeps the count pinned at
/// `min(top_k, matches)` no matter how deep the match is buried.
#[test]
fn recall_does_not_shrink_as_excluded_corpus_grows() {
    let top_k = 5u32;
    let mut last: Option<usize> = None;
    for n_decoys in [10usize, 50, 120, 250] {
        let (_tmp, conn) = fresh_index();
        let match_names = build_corpus(&conn, n_decoys, top_k as usize);
        let hits = run_knn(&conn, top_k);

        assert_eq!(
            hits.len(),
            top_k as usize,
            "with {n_decoys} nearer decoys the count dropped to {} (expected {top_k})",
            hits.len(),
        );
        let returned: std::collections::HashSet<&str> =
            hits.iter().map(|c| c.name.as_str()).collect();
        for name in &match_names {
            assert!(
                returned.contains(name.as_str()),
                "match `{name}` lost at decoy count {n_decoys}",
            );
        }
        if let Some(prev) = last {
            assert!(
                hits.len() >= prev,
                "recall shrank from {prev} to {} as the corpus grew",
                hits.len(),
            );
        }
        last = Some(hits.len());
    }
}

/// WIDEN CEILING (task T053). A genuinely small match set — fewer matches
/// than `top_k`, behind nearer decoys — must return the TRUE smaller set:
/// no error, no padding, and no leakage of excluded/global rows even though
/// the widen loop scans every vector before exhausting the candidate
/// universe.
#[test]
fn widen_terminates_at_true_smaller_match_set_without_leakage() {
    let (_tmp, conn) = fresh_index();
    let top_k = 10u32;
    let n_decoys = 40usize; // nearer, wrong-catalog
    let n_matches = 3usize; // < top_k
    let match_names = build_corpus(&conn, n_decoys, n_matches);

    // Sanity: the universe is fully populated, so the widen ceiling is real.
    assert_eq!(total_embeddings(&conn), (n_decoys + n_matches) as i64);

    let hits = run_knn(&conn, top_k);

    // min(top_k, matches) = min(10, 3) = 3 — the TRUE set, not top_k.
    assert_eq!(
        hits.len(),
        n_matches,
        "exhausted widen must return the true smaller match set ({n_matches}), got {}",
        hits.len(),
    );
    let returned: std::collections::HashSet<&str> = hits.iter().map(|c| c.name.as_str()).collect();
    for name in &match_names {
        assert!(returned.contains(name.as_str()), "match `{name}` missing");
    }
    // No decoy leaked despite the loop scanning the entire virtual table.
    for c in &hits {
        assert_eq!(c.catalog, MATCH_CATALOG, "decoy `{}` leaked in", c.name);
    }
}

/// WIDEN CEILING via the `searchable = 0` exclusion class, plus an empty
/// result. Confirms exhaustion returns `[]` (never an error) when the only
/// rows that would match are filtered out — here every candidate row is
/// `searchable = 0`, so the true match set is empty.
#[test]
fn widen_exhaustion_with_zero_matches_returns_empty_not_error() {
    let (_tmp, conn) = fresh_index();
    let ws = global_ws_id(&conn);
    // 30 enrolled rows in the MATCH catalog but all searchable = 0.
    for i in 0..30 {
        insert_row(
            &conn,
            ws,
            MATCH_CATALOG,
            "p",
            &format!("hidden-{i}"),
            "skill",
            match_vector_bytes(),
            false, // searchable = 0 → excluded by `s.searchable = 1`
            true,
        );
    }

    let hits = run_knn(&conn, 5);
    assert!(
        hits.is_empty(),
        "all-unsearchable corpus must yield zero hits (no error), got {hits:?}",
    );
}

// ---- #319: repeatable --catalog/--plugin + --kind filters ----------------
//
// These exercise the multi-value `IN (...)` filters end-to-end against a real
// on-disk index: insert enrolled, searchable rows spanning several plugins and
// kinds (all at the same cosine distance so ordering never hides a row), then
// assert each filter dimension narrows to exactly the expected set. The rows
// share the MATCH_CATALOG so the catalog dimension is neutral here.

/// Insert an enrolled, searchable match-vector row of `kind` in `plugin`.
fn insert_kind_row(conn: &Connection, ws: i64, plugin: &str, name: &str, kind: &str) {
    insert_row(
        conn,
        ws,
        MATCH_CATALOG,
        plugin,
        name,
        kind,
        match_vector_bytes(),
        true,
        true,
    );
}

fn kinds_of(hits: &[Candidate]) -> std::collections::HashSet<&'static str> {
    hits.iter().map(|c| c.kind.as_str()).collect()
}

/// `--kind skill` returns only skills; a corpus with skills + commands narrows
/// to the skills alone.
#[test]
fn kind_filter_single_returns_only_that_kind() {
    let (_tmp, conn) = fresh_index();
    let ws = global_ws_id(&conn);
    insert_kind_row(&conn, ws, "p", "sk-1", "skill");
    insert_kind_row(&conn, ws, "p", "sk-2", "skill");
    insert_kind_row(&conn, ws, "p", "cmd-1", "command");

    let filters = QueryFilters {
        kinds: vec![EntryKind::Skill],
        ..Default::default()
    };
    let hits = knn(&conn, "global", &query_vector(), 10, &filters).expect("knn");
    assert_eq!(
        hits.len(),
        2,
        "only the two skills must survive, got {hits:?}"
    );
    assert_eq!(
        kinds_of(&hits),
        std::collections::HashSet::from(["skill"]),
        "every returned row must be a skill",
    );
}

/// `--kind skill --kind command` returns both kinds (OR within the dimension).
#[test]
fn kind_filter_multi_returns_the_union_of_kinds() {
    let (_tmp, conn) = fresh_index();
    let ws = global_ws_id(&conn);
    insert_kind_row(&conn, ws, "p", "sk-1", "skill");
    insert_kind_row(&conn, ws, "p", "cmd-1", "command");
    // Agents are indexed with searchable = 0 in production; to prove the
    // union is exactly {skill, command} we simply do not insert an agent row.

    let filters = QueryFilters {
        kinds: vec![EntryKind::Skill, EntryKind::Command],
        ..Default::default()
    };
    let hits = knn(&conn, "global", &query_vector(), 10, &filters).expect("knn");
    assert_eq!(hits.len(), 2, "both the skill and the command must survive");
    assert_eq!(
        kinds_of(&hits),
        std::collections::HashSet::from(["skill", "command"]),
        "the union of the two requested kinds must come back",
    );
}

/// A `--kind` value with no matching searchable rows returns empty, NOT an
/// error — e.g. `--kind agent` over a searchable corpus that holds no agents.
#[test]
fn kind_filter_with_no_matching_rows_returns_empty_not_error() {
    let (_tmp, conn) = fresh_index();
    let ws = global_ws_id(&conn);
    insert_kind_row(&conn, ws, "p", "sk-1", "skill");
    insert_kind_row(&conn, ws, "p", "cmd-1", "command");

    let filters = QueryFilters {
        kinds: vec![EntryKind::Agent],
        ..Default::default()
    };
    let hits = knn(&conn, "global", &query_vector(), 10, &filters).expect("knn");
    assert!(
        hits.is_empty(),
        "a kind with no searchable rows must yield [] (no error), got {hits:?}",
    );
}

/// `--plugin a --plugin b` returns rows from EITHER plugin (OR within the
/// plugin dimension), and excludes a third plugin.
#[test]
fn plugin_filter_multi_returns_union_of_named_plugins() {
    let (_tmp, conn) = fresh_index();
    let ws = global_ws_id(&conn);
    insert_kind_row(&conn, ws, "plugin-a", "a-1", "skill");
    insert_kind_row(&conn, ws, "plugin-b", "b-1", "skill");
    insert_kind_row(&conn, ws, "plugin-c", "c-1", "skill"); // excluded

    let filters = QueryFilters {
        plugins: vec!["plugin-a", "plugin-b"],
        ..Default::default()
    };
    let hits = knn(&conn, "global", &query_vector(), 10, &filters).expect("knn");
    let plugins: std::collections::HashSet<&str> = hits.iter().map(|c| c.plugin.as_str()).collect();
    assert_eq!(
        plugins,
        std::collections::HashSet::from(["plugin-a", "plugin-b"]),
        "only the two named plugins may appear, got {plugins:?}",
    );
    assert_eq!(hits.len(), 2, "exactly one row per named plugin");
}

/// A SINGLE `--plugin` value behaves exactly as the pre-#319 `= ?` filter did:
/// only that plugin's rows come back. Back-compat guard for the `IN (?4)` shape.
#[test]
fn plugin_filter_single_value_is_back_compatible() {
    let (_tmp, conn) = fresh_index();
    let ws = global_ws_id(&conn);
    insert_kind_row(&conn, ws, "plugin-a", "a-1", "skill");
    insert_kind_row(&conn, ws, "plugin-b", "b-1", "skill");

    let filters = QueryFilters {
        plugins: vec!["plugin-a"],
        ..Default::default()
    };
    let hits = knn(&conn, "global", &query_vector(), 10, &filters).expect("knn");
    assert_eq!(
        hits.len(),
        1,
        "a single --plugin filters to exactly that plugin"
    );
    assert_eq!(hits[0].plugin, "plugin-a");
}

/// The three dimensions AND together: `--plugin` ∩ `--kind` narrows on both.
#[test]
fn combined_plugin_and_kind_filters_and_together() {
    let (_tmp, conn) = fresh_index();
    let ws = global_ws_id(&conn);
    insert_kind_row(&conn, ws, "plugin-a", "a-skill", "skill");
    insert_kind_row(&conn, ws, "plugin-a", "a-command", "command");
    insert_kind_row(&conn, ws, "plugin-b", "b-skill", "skill");

    // plugin-a AND kind=skill → exactly the one row.
    let filters = QueryFilters {
        plugins: vec!["plugin-a"],
        kinds: vec![EntryKind::Skill],
        ..Default::default()
    };
    let hits = knn(&conn, "global", &query_vector(), 10, &filters).expect("knn");
    assert_eq!(
        hits.len(),
        1,
        "plugin-a ∩ skill must be exactly one row, got {hits:?}"
    );
    assert_eq!(hits[0].name, "a-skill");
    assert_eq!(hits[0].plugin, "plugin-a");
    assert_eq!(hits[0].kind, EntryKind::Skill);
}

/// Multi-catalog: `--catalog x --catalog y` returns rows from either catalog
/// and excludes a third. (`insert_kind_row` pins MATCH_CATALOG, so insert the
/// distinct catalogs directly via `insert_row`.)
#[test]
fn catalog_filter_multi_returns_union_of_named_catalogs() {
    let (_tmp, conn) = fresh_index();
    let ws = global_ws_id(&conn);
    for (cat, name) in [("cat-a", "a-1"), ("cat-b", "b-1"), ("cat-c", "c-1")] {
        insert_row(
            &conn,
            ws,
            cat,
            "p",
            name,
            "skill",
            match_vector_bytes(),
            true,
            true,
        );
    }

    let filters = QueryFilters {
        catalogs: vec!["cat-a", "cat-b"],
        ..Default::default()
    };
    let hits = knn(&conn, "global", &query_vector(), 10, &filters).expect("knn");
    let catalogs: std::collections::HashSet<&str> =
        hits.iter().map(|c| c.catalog.as_str()).collect();
    assert_eq!(
        catalogs,
        std::collections::HashSet::from(["cat-a", "cat-b"]),
        "only the two named catalogs may appear, got {catalogs:?}",
    );
}

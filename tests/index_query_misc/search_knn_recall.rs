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
#[allow(clippy::too_many_arguments)]
fn insert_row(
    conn: &Connection,
    ws_id: i64,
    catalog: &str,
    plugin: &str,
    name: &str,
    embedding: Vec<u8>,
    searchable: bool,
    enrol: bool,
) {
    conn.execute(
        "INSERT INTO skills
            (catalog, plugin, name, kind, description, plugin_version,
             path, content_hash, searchable, user_invocable, when_to_use, indexed_at)
         VALUES (?1, ?2, ?3, 'skill', 'desc', '0.0.0', ?4, ?5, ?6, 0, NULL, 0)",
        params![
            catalog,
            plugin,
            name,
            format!("skills/{name}/SKILL.md"),
            // Unique content_hash per row keeps the (intentionally loose)
            // hash index honest; not load-bearing for the query.
            format!("hash-{catalog}-{name}"),
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
        catalog: Some(MATCH_CATALOG),
        plugin: None,
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

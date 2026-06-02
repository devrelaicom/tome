//! Regression suite for FR-001 / F-KNN: filtered KNN must return exactly
//! `min(top_k, total matching entries)`, regardless of how many *nearer*
//! vectors are excluded by the workspace / `searchable` / `--catalog` /
//! `--plugin` post-JOIN filters.
//!
//! ## The hazard under test
//!
//! `index::query::knn` binds the `sqlite-vec` virtual-table `k` limit and
//! ONLY THEN applies the JOIN/WHERE filters. If `>= k` vectors that are
//! *nearer* than a genuine match get excluded by those filters, the match
//! never enters the candidate window and the result is silently short. A
//! naive fixed-multiplier over-fetch (`k = top_k * 8`) papers over small
//! cases but still loses a match pushed far enough down the neighbour
//! ordering — only a geometric *widen* loop, bounded by the global
//! embeddings count, is correct.
//!
//! ## Fixture strategy (no real model, fully deterministic)
//!
//! We bypass the stub embedder's text→vector hashing and write embeddings
//! DIRECTLY, so we control each row's neighbour rank exactly. The schema
//! declares `embedding FLOAT[384]` with no `distance_metric`, so `vec0`
//! uses its default **L2** metric. Every fixture vector is all-zeros
//! except component[0]; the query is the zero vector, so a row with
//! `component[0] = v` sits at L2 distance `|v|` from the query. Assigning
//! decoys small `v` and real matches large `v` gives a precise, metric-
//! agnostic ordering: decoys are uniformly nearer than every match.

mod common;

use common::{stub_embedder_seed, stub_reranker_seed, stub_summariser_seed};
use rusqlite::{Connection, params};
use tempfile::TempDir;
use tome::index::query::{Candidate, QueryFilters};
use tome::index::{self, OpenOptions, knn};

const DIM: usize = 384;
const MATCH_CATALOG: &str = "match-cat";
const DECOY_CATALOG: &str = "decoy-cat";

/// Encode a 384-dim vector whose only non-zero component is index 0, set to
/// `magnitude`. Little-endian f32, matching `query::vector_to_bytes`.
fn axis_vector_bytes(magnitude: f32) -> Vec<u8> {
    let mut v = vec![0.0f32; DIM];
    v[0] = magnitude;
    let mut out = Vec::with_capacity(DIM * 4);
    for f in &v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

/// The query vector: the origin, so a row at `component[0] = m` is exactly
/// L2 distance `|m|` away.
fn query_vector() -> Vec<f32> {
    vec![0.0f32; DIM]
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
    magnitude: f32,
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
        params![skill_id, axis_vector_bytes(magnitude)],
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
/// plus `n_matches` *farther* rows that DO satisfy every filter. Decoys
/// occupy L2 distances `1.0 ..= n_decoys`; matches occupy a band starting at
/// `decoy_max + 100.0` so they are unambiguously behind every decoy.
///
/// Returns the set of match names for membership assertions.
fn build_corpus(conn: &Connection, n_decoys: usize, n_matches: usize) -> Vec<String> {
    let ws = global_ws_id(conn);

    // Nearer-than-everything decoys, all in the WRONG catalog → excluded by
    // `QueryFilters{ catalog: MATCH_CATALOG }`. Enrolled + searchable so the
    // *only* thing keeping them out of the result is the catalog filter.
    for i in 0..n_decoys {
        let mag = 1.0 + i as f32; // 1.0, 2.0, ... strictly < any match
        insert_row(
            conn,
            ws,
            DECOY_CATALOG,
            "decoy-plugin",
            &format!("decoy-{i}"),
            mag,
            true,
            true,
        );
    }

    // Genuine matches, all farther than every decoy.
    let base = 1.0 + n_decoys as f32 + 100.0;
    let mut names = Vec::with_capacity(n_matches);
    for i in 0..n_matches {
        let name = format!("match-{i}");
        insert_row(
            conn,
            ws,
            MATCH_CATALOG,
            "match-plugin",
            &name,
            base + i as f32,
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

/// CORE REGRESSION (FR-001). The corpus is sized so that the 5 genuine
/// matches sit at neighbour ranks 61..=65 behind 60 nearer decoys. A naive
/// `k = top_k * 8 = 40` over-fetch never reaches them → 0 matches survive
/// the catalog filter under the buggy implementation. Only the widen loop,
/// growing `k` past the 65-vector universe, recovers all five.
#[test]
fn filtered_knn_returns_top_k_despite_many_nearer_excluded_vectors() {
    let (_tmp, conn) = fresh_index();
    let top_k = 5u32;
    let n_decoys = 60; // >> top_k * 8 (= 40): defeats any fixed multiplier
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
            1.0 + i as f32,
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

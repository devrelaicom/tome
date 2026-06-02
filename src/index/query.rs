//! KNN query over `skill_embeddings`, joined with the `skills` table.
//!
//! Reranking lives in the embedding crate (slice 5). This layer returns the
//! top-K rows by cosine distance, filtered to skills enrolled in the
//! resolved workspace (via `workspace_skills`) and optionally to a single
//! catalog / plugin so the caller can implement `--catalog` and `--plugin`
//! flags on `tome query`. Phase 4 / F11a swapped the F9 hard-coded
//! `'global'` join for a runtime `workspace_name` parameter sourced from
//! the resolved scope.
//!
//! Spec: data-model.md §10 (`QueryResult`), contracts/query.md, FR-024.

use rusqlite::Connection;
use rusqlite::params_from_iter;
use rusqlite::types::ToSqlOutput;

use crate::error::TomeError;
use crate::plugin::identity::EntryKind;

/// One hit returned by [`knn`]. The caller composes the user-facing
/// [`crate::index::query::QueryResult`] from these plus the scoring stage
/// information from the reranker.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub skill_id: i64,
    pub catalog: String,
    pub plugin: String,
    pub name: String,
    /// Phase 5: entry kind discriminator. `search_skills` surfaces this
    /// in result rows so agents can distinguish skills from commands.
    pub kind: EntryKind,
    pub description: String,
    pub plugin_version: String,
    pub path: String,
    /// Cosine distance from `sqlite-vec`. Lower is better.
    pub distance: f32,
}

/// Optional pre-filters for [`knn`]. Both filters are AND-joined.
#[derive(Debug, Clone, Default)]
pub struct QueryFilters<'a> {
    pub catalog: Option<&'a str>,
    pub plugin: Option<&'a str>,
}

/// Initial over-fetch factor applied to `top_k` when binding the `vec0`
/// `k` limit (see [`knn`] for the recall hazard this addresses).
const OVER_FETCH_MULTIPLIER: u32 = 4;

/// Geometric growth factor for the widen loop when an over-fetch pass does
/// not yield `min(top_k, total matches)` survivors.
const WIDEN_GROWTH: u32 = 4;

/// Return the top `top_k` enabled skills closest to `query_vec`, scoped to
/// the workspace named `workspace_name`. `query_vec` must have length 384
/// (matches the `FLOAT[384]` virtual table column); shorter / longer vectors
/// surface as [`TomeError::IndexIntegrityCheckFailure`].
///
/// The result is exactly `min(top_k, total matching entries)` rows, ordered
/// by ascending distance, *regardless* of how many nearer vectors are
/// excluded by the workspace / `searchable` / `--catalog` / `--plugin`
/// filters.
///
/// # Why over-fetch + widen
///
/// `vec0` applies its `k` limit BEFORE we JOIN to `skills` /
/// `workspace_skills` and apply the post-filters — the virtual table has no
/// visibility into those columns. So binding `k = top_k` directly means: if
/// `>= top_k` vectors nearer than a genuine match are excluded by the
/// filters, the match never enters the candidate window and the result is
/// silently short. We over-fetch a multiple of `top_k`, apply the filters,
/// and if fewer than `min(top_k, total)` survive we re-query with a
/// geometrically larger `k`. The candidate universe is bounded by the global
/// `skill_embeddings` count (`vec0 MATCH` scans the whole table), so the
/// loop terminates: either `top_k` survivors are collected, or `k` reaches
/// that count and we have scanned every vector — at which point the true
/// (smaller) match set is returned. No schema change; the only knob is `k`.
pub fn knn(
    conn: &Connection,
    workspace_name: &str,
    query_vec: &[f32],
    top_k: u32,
    filters: &QueryFilters<'_>,
) -> Result<Vec<Candidate>, TomeError> {
    if query_vec.len() != 384 {
        return Err(TomeError::IndexIntegrityCheckFailure(format!(
            "query vector length {} must equal 384",
            query_vec.len()
        )));
    }
    if top_k == 0 {
        return Ok(Vec::new());
    }

    // Candidate-universe ceiling: `vec0 MATCH` scans the entire virtual
    // table, so the most candidates any pass can surface is the total
    // embeddings count. Beyond this, widening cannot reveal new rows — it is
    // the loop's hard termination bound. An empty index short-circuits
    // (vec0 rejects `k < 1`).
    let total: u32 = conn
        .query_row("SELECT COUNT(*) FROM skill_embeddings", [], |r| {
            r.get::<_, i64>(0)
        })
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("count skill_embeddings: {e}")))?
        .try_into()
        .unwrap_or(u32::MAX);
    if total == 0 {
        return Ok(Vec::new());
    }

    let sql = build_knn_sql(filters);
    let query_bytes = vector_to_bytes(query_vec);

    // Start at the over-fetch window, capped at the universe. `top_k * MULT`
    // uses saturating arithmetic so a pathologically large `top_k` cannot
    // overflow into a tiny `k`.
    let mut k = top_k.saturating_mul(OVER_FETCH_MULTIPLIER).clamp(1, total);
    loop {
        let candidates = run_knn_pass(conn, &sql, &query_bytes, k, workspace_name, filters)?;

        // Collected enough, or we have scanned the whole table (the true
        // match set is now fully known — return it even if it is smaller
        // than `top_k`).
        if candidates.len() as u32 >= top_k || k >= total {
            return Ok(truncate(candidates, top_k));
        }

        // Widen geometrically toward the ceiling. Saturating mul guards the
        // overflow; the clamp guarantees forward progress (k strictly grows
        // until it reaches `total`, where the loop above exits).
        k = k.saturating_mul(WIDEN_GROWTH).clamp(k + 1, total);
    }
}

/// Truncate to `top_k` rows. The candidate vector is already ordered by
/// ascending distance, so this keeps the nearest matches.
fn truncate(mut candidates: Vec<Candidate>, top_k: u32) -> Vec<Candidate> {
    candidates.truncate(top_k as usize);
    candidates
}

/// Build the (parameterised) KNN SQL for the given filters. `?1` is the
/// query vector, `?2` the `vec0` `k` limit, `?3` the workspace name; the
/// optional `--catalog` / `--plugin` filters take the next positional
/// indices. The string is filter-shaped only — `k` is rebound per widen
/// iteration without re-deriving it.
fn build_knn_sql(filters: &QueryFilters<'_>) -> String {
    // Phase 5: `search_skills` covers both kinds (skills + commands) but
    // honours the `searchable` flag. Entries with
    // `disable-model-invocation: true` are excluded.
    let mut sql = String::from(
        "SELECT s.id, s.catalog, s.plugin, s.name, s.kind, s.description,
                s.plugin_version, s.path, e.distance
         FROM skill_embeddings AS e
         JOIN skills AS s ON s.id = e.skill_id
         JOIN workspace_skills AS ws ON ws.skill_id = s.id
                                    AND ws.workspace_id = (SELECT id FROM workspaces WHERE name = ?3)
         WHERE e.embedding MATCH ?1 AND k = ?2 AND s.searchable = 1",
    );
    // Positional indices continue after the three fixed params (?1..?3).
    let mut next = 4;
    if filters.catalog.is_some() {
        sql.push_str(&format!(" AND s.catalog = ?{next}"));
        next += 1;
    }
    if filters.plugin.is_some() {
        sql.push_str(&format!(" AND s.plugin = ?{next}"));
    }
    sql.push_str(" ORDER BY e.distance");
    sql
}

/// Run one KNN pass for a concrete `k`, returning the filtered, distance-
/// ordered candidates. Caller drives the over-fetch / widen loop.
fn run_knn_pass(
    conn: &Connection,
    sql: &str,
    query_bytes: &[u8],
    k: u32,
    workspace_name: &str,
    filters: &QueryFilters<'_>,
) -> Result<Vec<Candidate>, TomeError> {
    // Params in positional order: query bytes (?1), k (?2), workspace (?3),
    // then the optional catalog / plugin filters in the order `build_knn_sql`
    // emitted them.
    let mut params: Vec<ToSqlOutput<'_>> = Vec::with_capacity(5);
    params.push(ToSqlOutput::from(query_bytes.to_vec()));
    params.push(ToSqlOutput::from(k as i64));
    params.push(ToSqlOutput::from(workspace_name.to_owned()));
    if let Some(c) = filters.catalog {
        params.push(ToSqlOutput::from(c.to_owned()));
    }
    if let Some(p) = filters.plugin {
        params.push(ToSqlOutput::from(p.to_owned()));
    }

    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("prepare knn: {e}")))?;

    let rows = stmt
        .query_map(params_from_iter(params.iter()), |row| {
            let kind_text: String = row.get(4)?;
            let kind = kind_text.parse::<EntryKind>().map_err(|msg| {
                rusqlite::Error::FromSqlConversionFailure(
                    4,
                    rusqlite::types::Type::Text,
                    Box::new(std::io::Error::other(msg)),
                )
            })?;
            Ok(Candidate {
                skill_id: row.get(0)?,
                catalog: row.get(1)?,
                plugin: row.get(2)?,
                name: row.get(3)?,
                kind,
                description: row.get(5)?,
                plugin_version: row.get(6)?,
                path: row.get(7)?,
                distance: row.get::<_, f64>(8)? as f32,
            })
        })
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("query knn: {e}")))?;

    rows.collect::<Result<_, _>>()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("collect knn rows: {e}")))
}

fn vector_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(std::mem::size_of_val(v));
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

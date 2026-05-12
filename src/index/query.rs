//! KNN query over `skill_embeddings`, joined with the `skills` table.
//!
//! Reranking lives in the embedding crate (slice 5). This layer returns the
//! top-K rows by cosine distance, filtered to `enabled = 1` and optionally
//! to a single catalog / plugin so the caller can implement `--catalog` and
//! `--plugin` flags on `tome query`.
//!
//! Spec: data-model.md §10 (`QueryResult`), contracts/query.md, FR-024.

use rusqlite::Connection;
use rusqlite::params_from_iter;
use rusqlite::types::ToSqlOutput;

use crate::error::TomeError;

/// One hit returned by [`knn`]. The caller composes the user-facing
/// [`crate::index::query::QueryResult`] from these plus the scoring stage
/// information from the reranker.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub skill_id: i64,
    pub catalog: String,
    pub plugin: String,
    pub name: String,
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

/// Return the top `top_k` enabled skills closest to `query_vec` in cosine
/// space. `query_vec` must have length 384 (matches the `FLOAT[384]` virtual
/// table column); shorter / longer vectors surface as
/// [`TomeError::IndexIntegrityCheckFailure`].
pub fn knn(
    conn: &Connection,
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

    let mut sql = String::from(
        "SELECT s.id, s.catalog, s.plugin, s.name, s.description,
                s.plugin_version, s.path, e.distance
         FROM skill_embeddings AS e
         JOIN skills AS s ON s.id = e.skill_id
         WHERE e.embedding MATCH ?1 AND k = ?2 AND s.enabled = 1",
    );

    // Collect params in order: query bytes, k, [catalog], [plugin].
    let query_bytes = vector_to_bytes(query_vec);
    let mut params: Vec<ToSqlOutput<'_>> = Vec::with_capacity(4);
    params.push(ToSqlOutput::from(query_bytes));
    params.push(ToSqlOutput::from(top_k as i64));
    if let Some(c) = filters.catalog {
        sql.push_str(" AND s.catalog = ?");
        sql.push_str(&format!("{}", params.len() + 1));
        params.push(ToSqlOutput::from(c.to_owned()));
    }
    if let Some(p) = filters.plugin {
        sql.push_str(" AND s.plugin = ?");
        sql.push_str(&format!("{}", params.len() + 1));
        params.push(ToSqlOutput::from(p.to_owned()));
    }
    sql.push_str(" ORDER BY e.distance");

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("prepare knn: {e}")))?;

    let rows = stmt
        .query_map(params_from_iter(params.iter()), |row| {
            Ok(Candidate {
                skill_id: row.get(0)?,
                catalog: row.get(1)?,
                plugin: row.get(2)?,
                name: row.get(3)?,
                description: row.get(4)?,
                plugin_version: row.get(5)?,
                path: row.get(6)?,
                distance: row.get::<_, f64>(7)? as f32,
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

//! KNN query over `skill_embeddings`, joined with the `skills` table.
//!
//! Reranking lives in the embedding crate (slice 5). This layer returns the
//! top-K rows by cosine distance, filtered to skills enrolled in the
//! resolved workspace (via `workspace_skills`) and optionally to a single
//! catalog / plugin so the caller can implement `--catalog` and `--plugin`
//! flags on `tome query`. Phase 4 / F11a swapped the F9 hard-coded
//! `'global'` join for a runtime `workspace_name` parameter sourced from
//! the resolved scope. Phase p11 / schema v6: queries now use
//! `vec_distance_cosine()` scalar over plain BLOB embeddings; the vec0
//! over-fetch / widen loop is removed because the plain JOIN filters at
//! SQL level, so the LIMIT is applied after filtering.
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

/// Top-`top_k` enabled, searchable skills closest to `query_vec` in the
/// workspace, by cosine distance. `query_vec` is embedded by the active
/// profile's embedder; its length must equal the stored vectors' length
/// (guaranteed by the embedder-drift→reindex invariant). Filters are applied
/// in the same statement, so the result is exactly min(top_k, matches) rows.
pub fn knn(
    conn: &Connection,
    workspace_name: &str,
    query_vec: &[f32],
    top_k: u32,
    filters: &QueryFilters<'_>,
) -> Result<Vec<Candidate>, TomeError> {
    if top_k == 0 || query_vec.is_empty() {
        return Ok(Vec::new());
    }
    let sql = build_knn_sql(filters);
    let query_bytes = vector_to_bytes(query_vec);

    let mut params: Vec<ToSqlOutput<'_>> = Vec::with_capacity(5);
    params.push(ToSqlOutput::from(query_bytes));          // ?1 query vector
    params.push(ToSqlOutput::from(workspace_name.to_owned())); // ?2 workspace
    params.push(ToSqlOutput::from(i64::from(top_k)));     // ?3 LIMIT
    if let Some(c) = filters.catalog { params.push(ToSqlOutput::from(c.to_owned())); }
    if let Some(p) = filters.plugin  { params.push(ToSqlOutput::from(p.to_owned())); }

    let mut stmt = conn.prepare(&sql)
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("prepare knn: {e}")))?;
    let rows = stmt.query_map(params_from_iter(params.iter()), |row| {
        let kind_text: String = row.get(4)?;
        let kind = kind_text.parse::<EntryKind>().map_err(|msg| {
            rusqlite::Error::FromSqlConversionFailure(
                4, rusqlite::types::Type::Text, Box::new(std::io::Error::other(msg)))
        })?;
        Ok(Candidate {
            skill_id: row.get(0)?, catalog: row.get(1)?, plugin: row.get(2)?,
            name: row.get(3)?, kind, description: row.get(5)?,
            plugin_version: row.get(6)?, path: row.get(7)?,
            distance: row.get::<_, f64>(8)? as f32,
        })
    })
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("query knn: {e}")))?;
    rows.collect::<Result<_, _>>()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("collect knn rows: {e}")))
}

/// Build the (parameterised) KNN SQL for the given filters. `?1` is the
/// query vector BLOB, `?2` the workspace name, `?3` the LIMIT; optional
/// `--catalog` / `--plugin` filters take the next positional indices.
/// `vec_distance_cosine` is applied as a scalar in the SELECT list and the
/// ORDER BY, so filtering is done at JOIN level before the LIMIT is applied —
/// no over-fetch / widen loop needed.
fn build_knn_sql(filters: &QueryFilters<'_>) -> String {
    let mut sql = String::from(
        "SELECT s.id, s.catalog, s.plugin, s.name, s.kind, s.description,
                s.plugin_version, s.path,
                vec_distance_cosine(e.embedding, ?1) AS distance
         FROM skill_embeddings AS e
         JOIN skills AS s ON s.id = e.skill_id
         JOIN workspace_skills AS ws ON ws.skill_id = s.id
                AND ws.workspace_id = (SELECT id FROM workspaces WHERE name = ?2)
         WHERE s.searchable = 1",
    );
    let mut next = 4;
    if filters.catalog.is_some() { sql.push_str(&format!(" AND s.catalog = ?{next}")); next += 1; }
    if filters.plugin.is_some()  { sql.push_str(&format!(" AND s.plugin = ?{next}")); }
    sql.push_str(" ORDER BY distance LIMIT ?3");
    sql
}

fn vector_to_bytes(v: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(std::mem::size_of_val(v));
    for f in v {
        out.extend_from_slice(&f.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_knn_sql_no_filters() {
        let sql = build_knn_sql(&QueryFilters::default());
        assert!(sql.contains("vec_distance_cosine(e.embedding, ?1)"), "must use cosine scalar");
        assert!(sql.contains("WHERE name = ?2"), "workspace param is ?2");
        assert!(sql.contains("LIMIT ?3"), "limit param is ?3");
        assert!(!sql.contains("?4"), "no extra params when no filters");
        assert!(!sql.contains("MATCH"), "must not use vec0 MATCH syntax");
    }

    #[test]
    fn build_knn_sql_with_catalog_filter() {
        let filters = QueryFilters { catalog: Some("my-catalog"), plugin: None };
        let sql = build_knn_sql(&filters);
        assert!(sql.contains("AND s.catalog = ?4"), "catalog filter is ?4");
        assert!(!sql.contains("?5"), "no plugin param when only catalog is set");
    }

    #[test]
    fn build_knn_sql_with_plugin_filter() {
        let filters = QueryFilters { catalog: None, plugin: Some("my-plugin") };
        let sql = build_knn_sql(&filters);
        // Without catalog, plugin is the first optional param: ?4
        assert!(sql.contains("AND s.plugin = ?4"), "plugin-only filter is ?4");
    }

    #[test]
    fn build_knn_sql_with_both_filters() {
        let filters = QueryFilters { catalog: Some("c"), plugin: Some("p") };
        let sql = build_knn_sql(&filters);
        assert!(sql.contains("AND s.catalog = ?4"), "catalog filter is ?4");
        assert!(sql.contains("AND s.plugin = ?5"), "plugin filter is ?5");
    }

    #[test]
    fn knn_returns_empty_for_zero_top_k() {
        crate::index::vec_ext::register_globally().unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let qv = vec![1.0f32; 4];
        let result = knn(&conn, "global", &qv, 0, &QueryFilters::default()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn knn_returns_empty_for_empty_query_vec() {
        crate::index::vec_ext::register_globally().unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let result = knn(&conn, "global", &[], 5, &QueryFilters::default()).unwrap();
        assert!(result.is_empty());
    }
}

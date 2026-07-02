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

/// Optional pre-filters for [`knn`]. The three dimensions are AND-joined; each
/// non-empty list is an OR set within its dimension (`s.<col> IN (…)`). An empty
/// list means "no filter for that dimension". A single-element list is exactly
/// `s.<col> = <v>` semantically, preserving the pre-#319 single-value behaviour.
#[derive(Debug, Clone, Default)]
pub struct QueryFilters<'a> {
    pub catalogs: Vec<&'a str>,
    pub plugins: Vec<&'a str>,
    pub kinds: Vec<EntryKind>,
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

    // LOCKSTEP INVARIANT: the params pushed here from `?4` onward MUST match, in
    // count and order, the `?N` placeholders `build_knn_sql` assigns. Both walk
    // the filter dimensions in the SAME fixed order — catalogs, then plugins,
    // then kinds — so `?4..` line up positionally. Changing the order in one
    // place without the other silently mis-filters (an off-by-one binds a
    // catalog value to a plugin placeholder). Keep the two loops in sync; the
    // `knn_param_count_matches_placeholders` unit test guards the invariant.
    let mut params: Vec<ToSqlOutput<'_>> = Vec::with_capacity(
        3 + filters.catalogs.len() + filters.plugins.len() + filters.kinds.len(),
    );
    params.push(ToSqlOutput::from(query_bytes)); // ?1 query vector
    params.push(ToSqlOutput::from(workspace_name.to_owned())); // ?2 workspace
    params.push(ToSqlOutput::from(i64::from(top_k))); // ?3 LIMIT
    for c in &filters.catalogs {
        params.push(ToSqlOutput::from((*c).to_owned())); // ?4.. catalogs
    }
    for p in &filters.plugins {
        params.push(ToSqlOutput::from((*p).to_owned())); // then plugins
    }
    for k in &filters.kinds {
        params.push(ToSqlOutput::from(k.as_str().to_owned())); // then kinds
    }

    let mut stmt = conn
        .prepare(&sql)
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

/// Build the (parameterised) KNN SQL for the given filters. `?1` is the query
/// vector BLOB, `?2` the workspace name, `?3` the LIMIT; the optional
/// `--catalog` / `--plugin` / `--kind` filters take the positional indices
/// `?4` onward, in that FIXED order (catalogs, plugins, kinds), one placeholder
/// per value in each non-empty list. `vec_distance_cosine` is applied as a
/// scalar in the SELECT list and the ORDER BY, so filtering is done at JOIN
/// level before the LIMIT is applied — no over-fetch / widen loop needed.
///
/// LOCKSTEP INVARIANT: `knn` binds params in the exact same dimension order
/// (catalogs → plugins → kinds). The two must be changed together.
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
    // Shared counter across all three IN-clauses: filter placeholders start at
    // ?4 (?1/?2/?3 are the vector / workspace / LIMIT). Fixed order to match the
    // `knn` param push: catalogs, then plugins, then kinds.
    let mut next = 4;
    push_in_clause(&mut sql, "s.catalog", filters.catalogs.len(), &mut next);
    push_in_clause(&mut sql, "s.plugin", filters.plugins.len(), &mut next);
    push_in_clause(&mut sql, "s.kind", filters.kinds.len(), &mut next);
    sql.push_str(" ORDER BY distance LIMIT ?3");
    sql
}

/// Append `AND <col> IN (?N, ?N+1, …)` for `count` values, advancing `next`
/// past the placeholders it consumed. A no-op when `count == 0` (empty list =
/// no filter for that dimension). Extracted so the three dimensions share one
/// placeholder-numbering path — the single source of the `?N` sequencing that
/// `knn`'s param push mirrors.
fn push_in_clause(sql: &mut String, col: &str, count: usize, next: &mut u32) {
    if count == 0 {
        return;
    }
    sql.push_str(&format!(" AND {col} IN ("));
    for i in 0..count {
        if i > 0 {
            sql.push_str(", ");
        }
        sql.push_str(&format!("?{}", *next));
        *next += 1;
    }
    sql.push(')');
}

/// Count the entries [`knn`] would actually search in `workspace_name`: the
/// enabled, `searchable = 1` skills joined into the resolved workspace via
/// `workspace_skills`. This is the SAME universe the KNN's FROM/JOIN/WHERE
/// defines — minus the vector distance / ORDER BY / LIMIT — so a caller can
/// tell "nothing is searchable in THIS scope" (count `== 0`, the fix is to
/// reindex / enable a plugin for the scope) apart from "the scope has
/// searchable content but nothing matched the query" (count `> 0`, the fix
/// is to rephrase).
///
/// Distinct from the whole-index `SELECT COUNT(*) FROM skill_embeddings`
/// that feeds the bucketed telemetry corpus size: THAT counts every scope
/// and ignores `searchable`, so it cannot answer the scope-effective
/// empty-vs-populated question the `search_skills` empty-result signal
/// needs. Cheap (one indexed `COUNT(*)`); callers only need `> 0` vs `== 0`.
pub fn scope_searchable_count(conn: &Connection, workspace_name: &str) -> Result<u64, TomeError> {
    let sql = "SELECT COUNT(*)
         FROM skill_embeddings AS e
         JOIN skills AS s ON s.id = e.skill_id
         JOIN workspace_skills AS ws ON ws.skill_id = s.id
                AND ws.workspace_id = (SELECT id FROM workspaces WHERE name = ?1)
         WHERE s.searchable = 1";
    conn.query_row(sql, [workspace_name], |r| r.get::<_, i64>(0))
        .map(|n| n.max(0) as u64)
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("scope searchable count: {e}")))
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

    /// The number of bind params [`knn`] pushes for `filters`, mirroring the
    /// push loop: 3 fixed (vector / workspace / LIMIT) + one per filter value.
    /// Used to assert the SQL placeholder count and the param count agree.
    fn expected_param_count(filters: &QueryFilters<'_>) -> usize {
        3 + filters.catalogs.len() + filters.plugins.len() + filters.kinds.len()
    }

    /// Count `?N` occurrences with N >= 4 in the SQL — i.e. the FILTER
    /// placeholders (?1/?2/?3 are the fixed vector/workspace/LIMIT). This is the
    /// SQL side of the lockstep; the param side is `filter value count`.
    fn filter_placeholder_count(sql: &str) -> usize {
        let mut n = 0;
        let bytes = sql.as_bytes();
        for (i, &b) in bytes.iter().enumerate() {
            if b == b'?' {
                // Read the following digits.
                let mut j = i + 1;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                if j > i + 1 {
                    let num: u32 = sql[i + 1..j].parse().unwrap();
                    if num >= 4 {
                        n += 1;
                    }
                }
            }
        }
        n
    }

    #[test]
    fn build_knn_sql_no_filters() {
        let sql = build_knn_sql(&QueryFilters::default());
        assert!(
            sql.contains("vec_distance_cosine(e.embedding, ?1)"),
            "must use cosine scalar"
        );
        assert!(sql.contains("WHERE name = ?2"), "workspace param is ?2");
        assert!(sql.contains("LIMIT ?3"), "limit param is ?3");
        assert!(!sql.contains("?4"), "no extra params when no filters");
        assert!(!sql.contains("MATCH"), "must not use vec0 MATCH syntax");
    }

    #[test]
    fn build_knn_sql_with_single_catalog_filter() {
        let filters = QueryFilters {
            catalogs: vec!["my-catalog"],
            ..Default::default()
        };
        let sql = build_knn_sql(&filters);
        // A single value is `IN (?4)` — equivalent to `= ?4`, back-compatible.
        assert!(
            sql.contains("AND s.catalog IN (?4)"),
            "single catalog filter is `IN (?4)`, got: {sql}"
        );
        assert!(
            !sql.contains("?5"),
            "no plugin/kind param when only one catalog is set"
        );
    }

    #[test]
    fn build_knn_sql_with_multi_catalog_filter() {
        let filters = QueryFilters {
            catalogs: vec!["a", "b", "c"],
            ..Default::default()
        };
        let sql = build_knn_sql(&filters);
        assert!(
            sql.contains("AND s.catalog IN (?4, ?5, ?6)"),
            "three catalogs must number ?4..?6, got: {sql}"
        );
        assert_eq!(
            filter_placeholder_count(&sql),
            3,
            "three catalog values → three filter placeholders"
        );
    }

    #[test]
    fn build_knn_sql_with_single_plugin_filter() {
        let filters = QueryFilters {
            plugins: vec!["my-plugin"],
            ..Default::default()
        };
        let sql = build_knn_sql(&filters);
        // Without catalog, plugin is the first optional dimension: ?4.
        assert!(
            sql.contains("AND s.plugin IN (?4)"),
            "plugin-only filter is `IN (?4)`, got: {sql}"
        );
    }

    #[test]
    fn build_knn_sql_with_multi_plugin_filter() {
        let filters = QueryFilters {
            plugins: vec!["a", "b"],
            ..Default::default()
        };
        let sql = build_knn_sql(&filters);
        assert!(
            sql.contains("AND s.plugin IN (?4, ?5)"),
            "two plugins must number ?4..?5, got: {sql}"
        );
    }

    #[test]
    fn build_knn_sql_with_kind_only_filter() {
        let filters = QueryFilters {
            kinds: vec![EntryKind::Skill, EntryKind::Command],
            ..Default::default()
        };
        let sql = build_knn_sql(&filters);
        // Kinds are the LAST dimension; with nothing before them they start ?4.
        assert!(
            sql.contains("AND s.kind IN (?4, ?5)"),
            "two kinds (no other filters) must number ?4..?5, got: {sql}"
        );
    }

    #[test]
    fn build_knn_sql_fixed_dimension_order_catalogs_plugins_kinds() {
        // The placeholder numbering follows the fixed dimension order:
        // catalogs (?4..), then plugins, then kinds. This is the SQL half of
        // the lockstep the `knn` param push mirrors.
        let filters = QueryFilters {
            catalogs: vec!["c1", "c2"],
            plugins: vec!["p1"],
            kinds: vec![EntryKind::Skill],
        };
        let sql = build_knn_sql(&filters);
        assert!(
            sql.contains("AND s.catalog IN (?4, ?5)"),
            "catalogs first at ?4..?5, got: {sql}"
        );
        assert!(
            sql.contains("AND s.plugin IN (?6)"),
            "plugin next at ?6, got: {sql}"
        );
        assert!(
            sql.contains("AND s.kind IN (?7)"),
            "kind last at ?7, got: {sql}"
        );
        // The `s.catalog IN` clause must appear before `s.plugin IN`, which
        // must appear before `s.kind IN` (ordering in the emitted SQL text).
        let cat = sql.find("s.catalog IN").unwrap();
        let plug = sql.find("s.plugin IN").unwrap();
        let kind = sql.find("s.kind IN").unwrap();
        assert!(
            cat < plug && plug < kind,
            "clause order must be cat<plug<kind"
        );
    }

    #[test]
    fn knn_param_count_matches_placeholders() {
        // The lockstep, asserted directly: for every combination of filter
        // widths, the number of filter placeholders in the SQL equals the
        // number of filter params `knn` would push (total params minus the 3
        // fixed). A drift here is exactly the off-by-one mis-filter bug.
        let combos = [
            QueryFilters::default(),
            QueryFilters {
                catalogs: vec!["c"],
                ..Default::default()
            },
            QueryFilters {
                plugins: vec!["p1", "p2"],
                ..Default::default()
            },
            QueryFilters {
                kinds: vec![EntryKind::Agent],
                ..Default::default()
            },
            QueryFilters {
                catalogs: vec!["c1", "c2"],
                plugins: vec!["p1"],
                kinds: vec![EntryKind::Skill, EntryKind::Command],
            },
        ];
        for f in &combos {
            let sql = build_knn_sql(f);
            let placeholders = filter_placeholder_count(&sql);
            let params = expected_param_count(f) - 3;
            assert_eq!(
                placeholders, params,
                "filter placeholder count ({placeholders}) must equal filter param count \
                 ({params}) for {f:?}; SQL: {sql}"
            );
        }
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

    #[test]
    fn scope_searchable_count_zero_for_empty_scope() {
        // A scope with no workspace / no enrolled skills counts 0 — the
        // subquery `(SELECT id FROM workspaces WHERE name = ?1)` resolves to
        // NULL and the join yields no rows. This is the #285 `index_empty`
        // discriminant.
        crate::index::vec_ext::register_globally().unwrap();
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for stmt in crate::index::schema::CREATE_STATEMENTS {
            conn.execute(stmt, []).unwrap();
        }
        let count = scope_searchable_count(&conn, "no-such-workspace").unwrap();
        assert_eq!(
            count, 0,
            "an unknown/empty scope must count 0 searchable rows"
        );
    }
}

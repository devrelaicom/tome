//! SQLite schema for the Tome index database (Phase 4 / schema v2).
//!
//! Phase 4 / F9 collapses the per-workspace databases into a single central
//! `<root>/index.db`. The on-disk schema gains four new tables —
//! `workspaces`, `workspace_skills`, `workspace_catalogs`, `workspace_projects`
//! — that move enablement and catalog enrolment off the workspace-specific
//! filesystem and onto the central DB. The Phase 2/3 `skills.enabled` column
//! is dropped: enablement is now expressed by the presence of a
//! `workspace_skills` row joining a `(workspace, skill)` pair.
//!
//! Bootstrap of a fresh DB emits schema v2 directly. The schema v1 → v2
//! migration in [`crate::index::migrations`] applies only to upgrades from
//! a Phase 2/3 DB on disk (in practice: synthetic-fixture tests; Phase 3's
//! FR-304 wipe guarantees no Phase 3 user DB is ever opened by Phase 4).
//!
//! Spec: [data-model.md §4](../../specs/004-phase-4-refactor-harnesses/data-model.md)
//! and [contracts/schema-migration-p4.md](../../specs/004-phase-4-refactor-harnesses/contracts/schema-migration-p4.md).

use rusqlite::{Connection, params};
use time::OffsetDateTime;

use crate::error::TomeError;

/// The schema version Tome's compiled-in code understands. Phase 5 bumped
/// this to 3 (entry-kind unification: `skills.kind` column + widened
/// unique constraint plus new `searchable` / `user_invocable` /
/// `when_to_use` columns). Phase 6 bumps to 4 via a *marker-only*
/// migration (entry-schema-p6.md): the `kind` column is free-text TEXT so
/// admitting `'agent'` needs no DDL — the migration exists solely to keep
/// the schema version monotonic and auditable so doctor's schema check and
/// the migration registry agree the `kind` domain widened. Phase 11 bumps
/// to 5: `workspace_skills.tier` column added (tiered skill routing). A
/// matching `Migration` row in [`crate::index::migrations`] handles
/// existing databases. Phase p11 / model tiering bumps to 6:
/// `skill_embeddings` rebuilt from `vec0(embedding FLOAT[384])` virtual
/// table into a plain `(skill_id INTEGER PRIMARY KEY, embedding BLOB)`
/// table; KNN uses `vec_distance_cosine()` scalar so no dimension is
/// baked into the schema (switching embedder models needs only a re-embed).
pub const SCHEMA_VERSION: u32 = 6;

/// The privileged seeded workspace name, present after every bootstrap and
/// migration. Phase 4's lifecycle paths route un-bound operations through
/// this workspace until the user opts into named workspaces (US2).
pub const GLOBAL_WORKSPACE: &str = "global";

/// Embedder / reranker / summariser identification stored in `meta` at
/// bootstrap. The caller (typically `db::open`) supplies the configured
/// names so the index can later detect drift against a future-different
/// runtime config.
#[derive(Debug, Clone)]
pub struct MetaSeed {
    pub name: String,
    pub version: String,
}

/// CREATE statements applied in order for a fresh schema-v2 database. Each
/// statement is one element so a failure mid-bootstrap surfaces with the
/// exact statement that broke. STRICT typing on `meta` is retained as
/// defence in depth; the workspace + skills tables are non-STRICT so the
/// schema-v1 → v2 migration's `INSERT INTO skills_new SELECT * FROM skills`
/// path can carry TEXT-stored `indexed_at` values from v1 into the new
/// `INTEGER`-declared column without an explicit conversion (data-model
/// §4 declares INTEGER; the value semantics shift to unix-seconds in a
/// future migration).
pub const CREATE_STATEMENTS: &[&str] = &[
    "CREATE TABLE meta (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    ) STRICT",
    "CREATE TABLE workspaces (
        id            INTEGER PRIMARY KEY AUTOINCREMENT,
        name          TEXT UNIQUE NOT NULL,
        created_at    INTEGER NOT NULL,
        last_used_at  INTEGER NOT NULL
    )",
    // Phase 5 / schema v3: the `skills` table now carries `kind`
    // (`skill` | `command`), `searchable`, `user_invocable`, and
    // `when_to_use`. The unique constraint widens to include `kind` so a
    // plugin can ship `skills/foo/SKILL.md` and `commands/foo.md`
    // side-by-side without name clashes.
    "CREATE TABLE skills (
        id              INTEGER PRIMARY KEY AUTOINCREMENT,
        catalog         TEXT NOT NULL,
        plugin          TEXT NOT NULL,
        name            TEXT NOT NULL,
        kind            TEXT NOT NULL DEFAULT 'skill',
        description     TEXT NOT NULL,
        plugin_version  TEXT NOT NULL,
        path            TEXT NOT NULL,
        content_hash    TEXT NOT NULL,
        searchable      INTEGER NOT NULL DEFAULT 1,
        user_invocable  INTEGER NOT NULL DEFAULT 0,
        when_to_use     TEXT,
        indexed_at      INTEGER NOT NULL
    )",
    "CREATE INDEX idx_skills_catalog_plugin ON skills(catalog, plugin)",
    "CREATE INDEX idx_skills_content_hash   ON skills(content_hash)",
    // Phase 5: widened identity tuple. Named `skills_unique` so the
    // schema-v2 → v3 migration's DROP/CREATE statements target the same
    // index name a fresh-bootstrap DB uses.
    "CREATE UNIQUE INDEX skills_unique ON skills (catalog, plugin, kind, name)",
    // Phase p11 / schema v6: dimension-free vector storage. The embedding is a
    // raw little-endian f32 BLOB of arbitrary length; KNN runs via the sqlite-vec
    // scalar vec_distance_cosine(). The dimension is NOT in the schema, so
    // switching embedder models (different dims) needs only a re-embed, never a
    // schema migration.
    "CREATE TABLE skill_embeddings (
        skill_id   INTEGER PRIMARY KEY,
        embedding  BLOB NOT NULL
    )",
    // Phase 11 / schema v5: `tier` column added (tiered skill routing).
    // Fresh bootstraps land here directly; existing DBs gain the column
    // via the phase_11_v4_to_v5 migration.
    "CREATE TABLE workspace_skills (
        workspace_id  INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
        skill_id      INTEGER NOT NULL REFERENCES skills(id)     ON DELETE CASCADE,
        enabled_at    INTEGER NOT NULL,
        tier          INTEGER NOT NULL DEFAULT 3,
        PRIMARY KEY (workspace_id, skill_id)
    )",
    "CREATE TABLE workspace_catalogs (
        workspace_id  INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
        catalog_name  TEXT NOT NULL,
        url           TEXT NOT NULL,
        pinned_ref    TEXT NOT NULL,
        PRIMARY KEY (workspace_id, catalog_name)
    )",
    "CREATE TABLE workspace_projects (
        project_path  TEXT PRIMARY KEY NOT NULL,
        workspace_id  INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
        bound_at      INTEGER NOT NULL
    )",
    "CREATE INDEX idx_workspace_projects_workspace ON workspace_projects(workspace_id)",
    "CREATE INDEX idx_workspace_skills_skill       ON workspace_skills(skill_id)",
    "CREATE INDEX idx_workspace_catalogs_url       ON workspace_catalogs(url)",
];

/// Apply every [`CREATE_STATEMENTS`] row, seed the `meta` table (including
/// the new summariser identity rows), and insert the privileged `global`
/// workspace. Runs inside a single transaction: a partial bootstrap is
/// never observable on disk thanks to WAL atomicity.
///
/// The `summariser` parameter is the third runtime identity row stored in
/// `meta`, alongside the embedder and reranker. Phase 4 / F6 ships with a
/// placeholder summariser registry entry whose SHA-256 is intentionally
/// all-zero (US4.a flips it to the real digest); the placeholder name +
/// version is still recorded here so drift detection and the doctor surface
/// know what the bootstrap committed to.
/// …`profile` is the value stamped as `model_profile` in the fresh `meta` row.
/// Callers that have no config preference pass `Profile::DEFAULT`; callers that
/// want config-driven seeding pass their resolved value. This is the ONLY site
/// that writes `model_profile`.
pub fn bootstrap(
    conn: &mut Connection,
    embedder: &MetaSeed,
    reranker: &MetaSeed,
    summariser: &MetaSeed,
    profile: crate::embedding::profile::Profile,
) -> Result<(), TomeError> {
    let tx = conn
        .transaction()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("begin bootstrap tx: {e}")))?;

    for stmt in CREATE_STATEMENTS {
        tx.execute_batch(stmt).map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!(
                "bootstrap statement failed ({e}): {stmt}"
            ))
        })?;
    }

    let now_unix = OffsetDateTime::now_utc().unix_timestamp();
    let now_rfc = now_unix.to_string();
    let schema_version = SCHEMA_VERSION.to_string();

    let rows: [(&str, &str); 9] = [
        ("schema_version", schema_version.as_str()),
        ("embedder_name", embedder.name.as_str()),
        ("embedder_version", embedder.version.as_str()),
        ("reranker_name", reranker.name.as_str()),
        ("reranker_version", reranker.version.as_str()),
        ("summariser_name", summariser.name.as_str()),
        ("summariser_version", summariser.version.as_str()),
        ("model_profile", profile.as_str()),
        ("created_at", now_rfc.as_str()),
    ];
    for (k, v) in rows {
        tx.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)",
            params![k, v],
        )
        .map_err(|e| {
            TomeError::IndexIntegrityCheckFailure(format!("seed meta `{k}` failed: {e}"))
        })?;
    }

    // Seed the privileged `global` workspace (FR-323). created_at and
    // last_used_at both reflect bootstrap time; subsequent write-path
    // commands bump last_used_at (FR-411).
    tx.execute(
        "INSERT INTO workspaces (name, created_at, last_used_at)
         VALUES (?1, ?2, ?2)",
        params![GLOBAL_WORKSPACE, now_unix],
    )
    .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("seed global workspace: {e}")))?;

    tx.commit()
        .map_err(|e| TomeError::IndexIntegrityCheckFailure(format!("commit bootstrap: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod schema_tests {
    use super::*;
    use crate::index::schema::MetaSeed;

    #[test]
    fn bootstrap_creates_blob_embeddings_table_and_seeds_profile() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::index::vec_ext::register_globally().unwrap();
        let seed = |n: &str, v: &str| MetaSeed {
            name: n.into(),
            version: v.into(),
        };
        bootstrap(
            &mut conn,
            &seed("e", "1"),
            &seed("r", "1"),
            &seed("s", "1"),
            crate::embedding::profile::Profile::DEFAULT,
        )
        .unwrap();
        // skill_embeddings is a plain table whose DDL has no FLOAT[N] / vec0.
        let ddl: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='skill_embeddings'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(ddl.contains("BLOB"), "embedding column must be BLOB: {ddl}");
        assert!(
            !ddl.to_lowercase().contains("vec0"),
            "must not be a vec0 table: {ddl}"
        );
        let profile: String = conn
            .query_row(
                "SELECT value FROM meta WHERE key='model_profile'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(profile, "medium");
    }
}

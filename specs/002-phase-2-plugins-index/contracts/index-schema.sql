-- Tome — Phase 2 index database schema, version 1.
-- Applied at first open of ${XDG_DATA_HOME}/tome/index.db.
-- A clean install jumps from schema_version=0 (no DB) to schema_version=1
-- via this script; future Phase-2 patches add migrations under
-- migrations/v2.sql etc. and append to MIGRATIONS in src/index/migrations.rs.

PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

-- ---------------------------------------------------------------------------
-- meta — key/value store, Tome-owned. Closed set of keys; unknown keys are
-- logged as warnings on read and are never written by Tome itself.
-- ---------------------------------------------------------------------------
CREATE TABLE meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
) STRICT;

-- Bootstrapped on first open:
--   ('schema_version',   '1')
--   ('embedder_name',    'bge-small-en-v1.5')
--   ('embedder_version', '1.5')
--   ('reranker_name',    'bge-reranker-base')
--   ('reranker_version', 'base')
--   ('created_at',       '<RFC3339>')

-- ---------------------------------------------------------------------------
-- skills — one row per indexed skill. Identity = (catalog, plugin, name).
-- plugin_version is recorded for diagnostics and the query output column,
-- but is intentionally NOT part of identity (FR-013).
-- ---------------------------------------------------------------------------
CREATE TABLE skills (
  id              INTEGER PRIMARY KEY,
  catalog         TEXT NOT NULL,
  plugin          TEXT NOT NULL,
  name            TEXT NOT NULL,
  description     TEXT NOT NULL,
  plugin_version  TEXT NOT NULL,
  path            TEXT NOT NULL,
  content_hash    TEXT NOT NULL,
  enabled         INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
  indexed_at      TEXT NOT NULL,                                  -- RFC 3339 UTC
  UNIQUE (catalog, plugin, name)
) STRICT;

CREATE INDEX idx_skills_catalog_plugin ON skills(catalog, plugin);
CREATE INDEX idx_skills_enabled        ON skills(enabled);
CREATE INDEX idx_skills_content_hash   ON skills(content_hash);

-- ---------------------------------------------------------------------------
-- skill_embeddings — 384-dim FLOAT vector, one row per skills.id.
-- Backed by the sqlite-vec extension, compiled in via build.rs.
-- ---------------------------------------------------------------------------
CREATE VIRTUAL TABLE skill_embeddings USING vec0(
  skill_id   INTEGER PRIMARY KEY,
  embedding  FLOAT[384]
);

-- ---------------------------------------------------------------------------
-- Notes:
--  * Skill deletion (drop a plugin's rows) must delete from both `skills`
--    and `skill_embeddings`; vec0 virtual tables do not auto-cascade.
--  * `tome reindex --force` performs a DELETE FROM both tables for the scope,
--    then re-INSERTs inside a single transaction.
--  * Schema version mismatch handling:
--      - stored == compiled: proceed
--      - stored  > compiled: refuse with SchemaTooNew (exit code 12)
--      - stored  < compiled: run forward migrations under the advisory lock
--    See src/index/migrations.rs.

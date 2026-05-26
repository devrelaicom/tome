# Phase 5 — Entry schema

Authoritative shape of the unified `EntryRow` (the underlying type for what was called a "skill row" through Phase 4). Two kinds, one table, one identity discriminator.

## Identity

```
(catalog, plugin, kind, name)
```

Unique across the entire central index. Per the v3 schema migration, the unique constraint widens from Phase 4's `(catalog, plugin, name)` to include `kind`.

A plugin shipping both `skills/foo/SKILL.md` and `commands/foo.md` produces two rows:
- `(midnight-expert, compact-dev, "skill", foo)`
- `(midnight-expert, compact-dev, "command", foo)`

Neither shadows the other (FR-003). The middle-tier tool disambiguates by an explicit `kind` parameter; the read tool and prompts surface default to `kind = "skill"` (FR-084, FR-100).

## Kind values

```
skill | command
```

Encoded in SQL as TEXT (matches the `EntryKind` Rust serde discriminator).

| Kind | Origin | Default `searchable` | Default `user_invocable` |
|---|---|---|---|
| `skill` | `<plugin>/skills/<name>/SKILL.md` | `1` (true) | `0` (false) |
| `command` | `<plugin>/commands/<name>.md` | `1` (true) | `1` (true) |

Defaults overridable via `disable-model-invocation` (flips `searchable`) and `user-invocable` (the literal frontmatter flag).

## Columns (after Phase 5 migration v3)

| Column | Type | Nullable | Notes |
|---|---|---|---|
| `catalog` | TEXT | NO | Existing — catalog name |
| `plugin` | TEXT | NO | Existing — plugin name |
| `name` | TEXT | NO | Existing — entry name (sanitised filename stem or `name` from frontmatter) |
| `kind` | TEXT | NO | NEW — `"skill"` or `"command"`; default `"skill"` for pre-migration rows |
| `description` | TEXT | YES | Existing — frontmatter `description` OR fallback to first 500 chars of body |
| `when_to_use` | TEXT | YES | NEW — frontmatter `when_to_use`; null if absent |
| `path` | TEXT | NO | Existing — absolute path to entry file |
| `content_hash` | TEXT | NO | Existing — SHA-256 of body + relevant frontmatter contributing to embedding text |
| `searchable` | INTEGER | NO | NEW — 1 if entry surfaced in `search_skills` results; 0 if `disable-model-invocation: true` |
| `user_invocable` | INTEGER | NO | NEW — 1 if entry surfaced in `prompts/list`; 0 otherwise |
| `enabled` | INTEGER | NO | Existing — workspace-side enrolment flag (via `workspace_skills` junction in Phase 4; this column may be redundant after Phase 4 — keep for now) |
| `indexed_at` | TEXT | NO | Existing — RFC 3339 timestamp |

## Embedding text composition (per R-12)

```
{name}

{description}

When to use: {when_to_use}
```

The "When to use:" line + preceding blank line appears only when `when_to_use` is non-empty.

Existing pre-migration rows have `when_to_use = NULL`; their `embedding_text` follows the Phase 4 shape (`{name}\n\n{description}`). On their next reindex, if frontmatter declares `when_to_use`, the recomposed string differs from the indexed version → content_hash changes → re-embedding triggered.

Embedding model unchanged: bge-small-en-v1.5 INT8 ONNX, 384 dimensions.

## Content hash

`content_hash = sha256(embedding_text + "\n--BODY--\n" + body)`. The format ensures any change to either contributes to the hash; the existing Phase 4 algorithm is preserved with the embedding_text inputs widened to include `when_to_use`.

## Insertion rules (`index::skills::upsert_skill`)

At `tome plugin enable` or `tome reindex`:

1. Parse frontmatter via `src/plugin/frontmatter.rs` (lenient).
2. Compute resolved values:
   - `kind`: known from directory walk (`skills/` vs `commands/`).
   - `searchable`: `!frontmatter.disable_model_invocation.unwrap_or(false)`.
   - `user_invocable`: `frontmatter.user_invocable.unwrap_or(match kind { Skill => false, Command => true })`.
3. Compute `embedding_text` and `content_hash`.
4. UPSERT into `skills`:
   ```sql
   INSERT INTO skills (catalog, plugin, name, kind, ...)
   VALUES (?, ?, ?, ?, ...)
   ON CONFLICT (catalog, plugin, kind, name) DO UPDATE SET
     description=excluded.description,
     when_to_use=excluded.when_to_use,
     path=excluded.path,
     content_hash=excluded.content_hash,
     searchable=excluded.searchable,
     user_invocable=excluded.user_invocable,
     indexed_at=excluded.indexed_at
   ```
5. If `content_hash` changed: delete-then-insert in `skill_embeddings` (per Phase 4's discipline: `INSERT OR REPLACE` doesn't work on `sqlite-vec` virtual tables).

## Lookup rules

By identity:
```sql
SELECT * FROM skills
WHERE catalog = ? AND plugin = ? AND kind = ? AND name = ? AND enabled = 1
```

By workspace user-invocable set:
```sql
SELECT s.* FROM skills s
JOIN workspace_skills ws ON ws.catalog = s.catalog AND ws.plugin = s.plugin AND ws.name = s.name
WHERE ws.workspace_id = ? AND s.user_invocable = 1 AND s.enabled = 1
```

By workspace searchable set (for `search_skills`):
```sql
SELECT s.* FROM skills s
JOIN workspace_skills ws ON ws.catalog = s.catalog AND ws.plugin = s.plugin AND ws.name = s.name
JOIN skill_embeddings se ON se.rowid = s.rowid
WHERE ws.workspace_id = ? AND s.searchable = 1 AND s.enabled = 1
ORDER BY <KNN distance>
```

The `workspace_skills` junction's FK reference is `(catalog, plugin, name)` — Phase 5 does NOT widen this FK to include `kind` because workspace_skills represents "which plugins are enabled in this workspace" (plugin-grained), not "which individual entries". Enabling a plugin enables all its entries (both kinds); disabling removes both kinds.

## Discriminator policy

Default `kind = "skill"` chosen for:
- Backwards compatibility (all pre-migration rows are skill-kind).
- The agent-callable read tool's default (`get_skill` without `kind` parameter → `skill`).
- The middle-tier tool's default (`get_skill_info` without `kind` parameter → `skill`).

A plugin-author wanting an entry with kind `command` must place it under `commands/`. There is no frontmatter flag to override kind — kind is directory-rooted (FR-002).

## What does NOT change

- `workspace_skills` junction table structure.
- `workspace_catalogs` junction table.
- `workspaces` table.
- `meta` table.
- `skill_embeddings` virtual table.

## Tests

| Behaviour | Test |
|---|---|
| Both kinds insert into single table | `tests/entry_kind_indexing.rs::both_directories_index_to_unified_table` |
| Same name across kinds produces two rows | `tests/entry_kind_indexing.rs::same_name_different_kind_produces_two_rows` |
| Default kind = `skill` for pre-migration rows | `tests/schema_migration_v3.rs::existing_rows_become_skill_kind` |
| Searchable filter applied to search_skills | `tests/mcp_search_skills_truncation.rs::disable_model_invocation_excluded` |
| User_invocable filter applied to prompts/list | `tests/mcp_prompts.rs::list_excludes_non_invocable` |
| `when_to_use` indexed in embedding_text | `tests/entry_kind_indexing.rs::when_to_use_contributes_to_embedding_text` |
| Re-embedding triggered when when_to_use newly appears | `tests/entry_kind_indexing.rs::when_to_use_change_invalidates_content_hash` |
| Workspace_skills junction widening (both kinds enrol) | `tests/entry_kind_indexing.rs::enable_synchronises_both_kinds_into_junction` |

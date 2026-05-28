# Phase 6 — Entry schema (agent kind)

How Phase 6 widens the unified entry model to admit a third kind, `agent`, with **no new columns and no new tables**. The only storage-layer change is widening the free-text `kind` domain; the only load-bearing Rust change is widening the `EntryKind` enum and every exhaustive match over it (FR-070a, research R-11). Builds directly on Phase 5's entry schema (`entry-schema-p5.md`).

## EntryKind widening (the load-bearing F2 change)

```
enum EntryKind { Skill, Command, Agent }   // Agent is new
```

- `FromStr` / `Display` gain the `"agent"` mapping (matches the SQL TEXT discriminator).
- **Every exhaustive match over `EntryKind` MUST be widened** (FR-070a): the per-kind count aggregation behind `tome plugin list` / `tome plugin show`, the doctor entry-count surface, and any MCP dispatch. No match may regress to a catch-all (`_ =>`) — the canonical-enum-dispatch discipline established in Phase 5 is preserved. A catch-all would re-hide schema drift, exactly the failure Phase 5 hardened against.
- This is **load-bearing**: a `kind='agent'` row introduced *without* the enum widening would make those count surfaces fail with the index-integrity error code (51). F2 lands the widening + per-kind-count tests **before any agent row is written**, so no later slice can introduce a crashing row.

## Identity and kind values

Identity is unchanged from Phase 5:

```
(catalog, plugin, kind, name)
```

Kind values widen to:

```
skill | command | agent
```

| Kind | Origin | Default `searchable` | Default `user_invocable` |
|---|---|---|---|
| `skill` | `<plugin>/skills/<name>/SKILL.md` | `1` | `0` |
| `command` | `<plugin>/commands/<name>.md` | `1` | `1` |
| `agent` | `<plugin>/agents/<name>.md` | **`0` always** | **`0` always** |

Agent rows are **never** searchable (excluded from `search_skills` in this version, FR-070) and are **not** prompts — `user_invocable = 0` always; personas are a separate specialised MCP-prompt path (agent-personas.md), not driven by `user_invocable`. There is no frontmatter flag to override an agent's `searchable`/`user_invocable`.

## Storage

- The `skills.kind` column (TEXT, free-text) admits `'agent'`. **No new columns, no new tables, no constraint change** (FR-071).
- Agent rows reuse the existing `(catalog, plugin, kind, name)` uniqueness constraint. A plugin shipping a skill `foo` and an agent `foo` produces two non-shadowing rows, exactly as skill+command do in Phase 5.
- The `workspace_skills` junction is unchanged (its FK is `(catalog, plugin, name)`, plugin-grained); enabling a plugin enrols all its entries of every kind, disabling removes them all.

Agent rows carry:

| Field | Source | Notes |
|---|---|---|
| `name` | frontmatter `name`, else filename stem | drives the cross-plugin clash set (FR-072) |
| `description` | frontmatter `description`, else first non-empty body line | informational |
| `content_hash` | SHA-256 of the agent `.md` | reindex diffing |
| `searchable` | `0` | never in `search_skills` (FR-070) |
| `user_invocable` | `0` | not a prompt |
| `when_to_use` | NULL | agents do not contribute embedding text |
| embedding | (skipped) | agents are not embedded in this version |

## Schema migration (marker-only, research R-11)

The `kind` column is free-text TEXT, so admitting `'agent'` needs **no DDL and no data migration**. However, Phase 6 **registers a marker-only migration** that bumps the schema version, so doctor's schema check and the migration registry agree the `kind` domain widened and the schema version stays monotonic and auditable.

- Precedent: Phase 3 shipped the migration framework (`index::migrations::apply_pending`, `Migration { from, to, name, apply }`); Phase 4 registered the **first real migration**; Phase 5 added the v2→v3 migration. Phase 6's migration follows the same framework — a registered `Migration` whose `apply` does no data transformation (it is a no-op marker that exists solely to advance the version).
- A failure of this migration surfaces as `SchemaMigrationFailed` (74); an unknown future schema on read-only open surfaces as `SchemaVersionTooNew` (73) — both carryover, see exit-codes-p6.md.
- No `kind`-column backfill is required: pre-Phase-6 rows are already `'skill'`/`'command'`; the migration introduces no `'agent'` rows itself (those arrive at plugin enable / reindex).

## Indexing pipeline

The plugin scan (Phase 5 walks `skills/*/SKILL.md` and `commands/*.md`) now **also walks** `agents/*.md` (`src/plugin/components.rs`), indexing each as `kind='agent'`:

1. Parse the agent's YAML frontmatter via the lenient third-party parser (`src/plugin/frontmatter.rs`), failing loudly only on malformed recognised structures (NFR-010); malformed frontmatter surfaces as code 45.
2. Resolve `name` = frontmatter `name`, else filename stem.
3. Resolve `description` = frontmatter `description`, else the first non-empty body line.
4. Compute `content_hash` over the agent `.md`.
5. UPSERT into `skills` with `kind='agent'`, `searchable=0`, `user_invocable=0`, `when_to_use=NULL`.
6. **Skip embedding** — no `skill_embeddings` row is written for agent rows.

Hooks and guardrails files (`hooks/hooks.json`, `hooks/GUARDRAILS.md`) are **not indexed** (FR-073). They are read during `tome harness sync`, not during indexing, consistent with the filesystem-inferred, no-sidecar model. Agent indexing exists only for provenance / enabled-state tracking and for the cheap cross-plugin name-collision query (FR-072): the clash set is the set of `<name>` values held by ≥ 2 agent-kind rows enabled in the resolved workspace, computed once per sync, governing filename display-name prefixing, harness-facing displayed-name prefixing, and persona naming identically across the workspace's bound projects.

## What does NOT change

- `workspace_skills`, `workspace_catalogs`, `workspaces`, `meta` table structures.
- `skill_embeddings` virtual table (agents simply never get a row there).
- The embedding model, embedding-text composition, and content-hash format for skills/commands.

## Tests

| Behaviour | Test |
|---|---|
| `EntryKind::Agent` round-trips via FromStr/Display | `tests/entry_kind_agent_indexing.rs::agent_kind_round_trips` |
| `agents/*.md` indexes with `kind='agent'`, `searchable=0` | `tests/entry_kind_agent_indexing.rs::agents_index_non_searchable` |
| Agent rows are never embedded | `tests/entry_kind_agent_indexing.rs::agent_rows_have_no_embedding` |
| Same name across skill+agent produces two rows | `tests/entry_kind_agent_indexing.rs::same_name_skill_and_agent_produce_two_rows` |
| Per-kind counts account for agent rows (no catch-all regression) | `tests/entry_kind_agent_indexing.rs::per_kind_counts_include_agents` |
| Marker migration bumps schema version | `tests/schema_migration_p6.rs::kind_domain_marker_bumps_version` |
| Agents excluded from `search_skills` | `tests/entry_kind_agent_indexing.rs::agents_absent_from_search` |

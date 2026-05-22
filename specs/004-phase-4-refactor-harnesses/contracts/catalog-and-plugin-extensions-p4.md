# Catalog + Plugin Extensions — Phase 4

**Spec source**: [spec.md FR-360 through FR-367 + FR-380 through FR-385](../spec.md)

Phase 1's catalog management and Phase 2's plugin enable/disable behaviours carry forward in surface but route storage through Phase 4's central DB and workspace junctions. CLI commands are unchanged in shape; semantics shift below the surface.

## Catalog refcount via `workspace_catalogs`

The Phase 3 `workspaces.txt` opt-in registry is **deleted** in Phase 4. The sole source of truth for catalog references is the central DB's `workspace_catalogs` table.

### `tome catalog add <source> [--name <name>]`

Operates against the resolved workspace.

**Algorithm**:

1. Resolve scope (Phase 4 resolution).
2. Acquire central DB advisory lockfile.
3. Parse `<source>` as a Git URL or local path; derive cache hash.
4. Check `workspace_catalogs` for any row with the same URL (any workspace):
   - **If a row exists**: cache directory already present. Re-parse `tome-catalog.toml` from the cache to refresh manifest. Insert `workspace_catalogs` row for the resolved workspace.
   - **If no row exists**: clone the URL into `<root>/catalogs/<hash>/` via system Git (Phase 1 shell-out pattern). Parse manifest. Insert `workspace_catalogs` row.
5. Commit lockfile release.

The reuse-existing-clone path skips Git operations entirely.

### `tome catalog remove <name>`

Operates against the resolved workspace.

**Algorithm**:

1. Resolve scope, acquire lockfile.
2. Check the resolved workspace's enabled plugins from this catalog. If any exist and `--force` not set, exit 53 (`CatalogHasEnabledPlugins`, Phase 2 variant unchanged).
3. With `--force`: cascade-disable every enabled plugin from this catalog in this workspace (drop `workspace_skills` rows; skills rows themselves remain, see plugin section). Same single-lock-per-batch pattern as Phase 2.
4. Delete the `workspace_catalogs` row.
5. Inside the same lockfile window, refcount-check (FR-366): enumerate `workspace_catalogs` for other rows with this URL.
   - If any other row references the URL: leave the cache directory in place.
   - If no other row references: delete `<root>/catalogs/<hash>/` via `fs::remove_dir_all`.
6. Commit lockfile release.

The refcount check and the cache deletion run inside the lockfile (FR-366) — concurrent `catalog remove` invocations from two workspaces serialise; the second one observes the URL row already removed by its own delete (idempotent).

### `tome catalog list`

Reports the resolved workspace's enrolled catalogs from `workspace_catalogs`. Other workspaces' enrolments do NOT appear.

### `tome catalog update [<name>]`

Refreshes catalog clones. Per FR-365, refreshes every catalog that ANY workspace enrols (not just the resolved workspace's). The reindex pass triggered by manifest changes covers every workspace's enabled plugins for any updated catalog.

### `tome catalog show <name>`

Unchanged in shape; reads from `workspace_catalogs` for the resolved workspace.

## Plugin enable/disable via `workspace_skills`

Phase 2's `skills.enabled` column is replaced by the `workspace_skills` junction (Phase 4 schema v2).

### `tome plugin enable <id>`

**Algorithm**:

1. Resolve scope, acquire lockfile.
2. Parse `<id>` as `<catalog>/<plugin>`.
3. Locate the plugin in the resolved workspace's catalogs (via `workspace_catalogs` × the cached manifest).
4. Parse each `SKILL.md` file under the plugin's directory; compute `content_hash` per skill.
5. Inside one DB transaction:
   - For each skill: UPSERT `skills` row keyed on `(catalog, plugin, name)`. If `content_hash` changed (or the row didn't exist), recompute embedding and UPSERT `skill_embeddings` for the skill id.
   - For each skill: INSERT `workspace_skills` row for `(resolved_workspace_id, skill_id, now)`. If already present (rebind / re-enable), the row's `enabled_at` updates via UPSERT semantics.
6. Trigger summary regeneration for the resolved workspace (see [summariser.md](./summariser.md) for the trigger contract).
7. Trigger integration sync if the resolved scope is from a bound project (the project's `[harnesses]` list is honoured; without a project, no sync).
8. Commit lockfile release.

**Cheap re-enable (Phase 3 FR-006 carries forward)**: if the existing `skills` row's `content_hash` equals the parsed hash, embedding is reused (zero embedder calls); the `workspace_skills` row UPSERT is the only write.

### `tome plugin disable <id>`

**Algorithm**:

1. Resolve scope, acquire lockfile.
2. Locate the plugin's skills.
3. Inside one DB transaction:
   - DELETE `workspace_skills` rows for the (resolved_workspace_id, skill_id) pairs.
   - `skills` rows are NOT deleted — another workspace may still enable the same skill.
4. Trigger summary regeneration.
5. Trigger integration sync.
6. Commit lockfile release.

### `tome plugin list`

Reports the resolved workspace's enabled plugins from `workspace_skills` × `skills` × `workspaces`. Other workspaces' enablements do NOT appear.

### `tome plugin reindex [<plugin>] [--force]`

**Algorithm**:

1. Resolve scope, acquire lockfile.
2. For each `workspace_skills` row in the resolved workspace (filtered by `<plugin>` if named):
   - Parse the underlying SKILL.md; compute fresh `content_hash`.
   - If hash differs OR `--force`: UPDATE `skills` row; recompute embedding; UPSERT `skill_embeddings`.
   - If hash unchanged AND no `--force`: skip.
3. If any skill's hash changed: trigger summary regeneration. Otherwise: no summariser invocation.
4. Trigger integration sync if applicable.
5. Commit lockfile release.

## Skills-row retention (FR-383)

A `skills` row whose every `workspace_skills` reference has been deleted is kept on disk indefinitely. Phase 4 ships NO garbage collection of orphan skill rows — the row's content_hash and embedding remain valid for any future workspace that enables the same `(catalog, plugin, name)` triple.

Phase 5+ may add `tome skills prune` if disk pressure becomes a real concern; current expectation is <5 MB of orphan vector storage per typical user.

## Summariser-failure forward progress (FR-385)

When summary regeneration fails inside the enable/disable/reindex flow:

1. The skill-state mutation transaction MUST have committed before the summariser is invoked.
2. The summariser failure exits with code 20.
3. The workspace's existing cached summary (if any) is left untouched.
4. Doctor reports the summariser subsystem as broken AND the workspace's cached summary as stale.

This is the "fail forward" rule: the developer's intent (enable a plugin) is honoured even if the summariser can't run. They re-run `regen-summary` after fixing the cause.

## Test coverage

- `tests/catalog_workspace_refcount.rs` — multi-workspace catalog clone reuse; refcount cleanup on last reference removed; refcount-under-lock serialisation between two workspaces.
- `tests/plugin_workspace_skills.rs` — enable in workspace A doesn't affect workspace B; disable in A doesn't drop skills row that B still references; rebind via UPSERT.
- `tests/plugin_cheap_reenable.rs` — content_hash match skips embedder (StubEmbedder call count == 0).
- `tests/plugin_summariser_forward_progress.rs` — stub failure on enable leaves workspace_skills committed; exit 20.
- `tests/catalog_update_cross_workspace_reindex.rs` — `catalog update` reindexes plugins enabled in every workspace, not just the resolved one.

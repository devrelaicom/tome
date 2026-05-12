# Catalog Command Extensions (Phase 2)

Two Phase 1 commands gain Phase 2 behaviour: `tome catalog update` and `tome catalog remove`. All Phase 1 behaviour is preserved; the additions are described here.

---

## `tome catalog update` — Phase 2 extension

### New behaviour

After a successful Git operation against a catalog, for every plugin in that catalog whose row in `skills` has `enabled = 1`:

1. Walk `${plugin path}/skills/*/SKILL.md`.
2. Parse each frontmatter leniently. Compute new `content_hash`.
3. Diff against stored rows:
   - Added (new skill not in the index): embed, INSERT.
   - Modified (`content_hash` changed): re-embed, UPDATE.
   - Removed (in index but no longer on disk): DELETE row + embedding.
4. If `plugin.json` is missing or unparsable post-refresh — auto-disable the plugin (FR-033): mark `enabled = 0`, DELETE its rows from `skills` and `skill_embeddings`, print a loud warning to stderr.
5. Aggregate per-catalog into a summary table at end of `update`.

Steps 1–4 run inside one SQLite transaction per plugin inside the advisory lockfile boundary.

### Output (human, summary appended after Phase 1 sync output)

```
Refreshed catalogs:
  midnight-experts        ↑ 3 commits
  another-catalog         (up-to-date)

Reindexed plugins:
  Plugin                              Added  Modified  Removed
  ──────────────────────────────────  ─────  ────────  ───────
  midnight-experts/compact-expert       0        2        0
  midnight-experts/ledger-helper        1        0        0

Warnings:
  midnight-experts/broken-plugin: plugin.json malformed — disabled and de-indexed
```

### Output (`--json`)

The Phase 1 catalog-update JSON is extended with a `plugin_changes` array:

```json
{
  "catalogs_refreshed": [...],
  "plugin_changes": [
    {
      "plugin": "midnight-experts/compact-expert",
      "skills_added": 0,
      "skills_modified": 2,
      "skills_removed": 0
    }
  ],
  "auto_disabled": [
    { "plugin": "midnight-experts/broken-plugin", "reason": "plugin.json malformed" }
  ]
}
```

### Errors

Phase 1 errors continue to propagate. Phase 2 additions:

| Trigger | Exit |
|---|---|
| Embedder missing during reindex | 30 (non-TTY) or download prompt (TTY) |
| Skill header malformed (delimiters) | 23 |
| Embedding generation failed | 36 |
| Index busy | 50 |
| Index integrity failure | 51 |

A per-plugin failure does NOT abort the whole `catalog update`; the affected plugin is reported as failed and the rest continues. This is a deliberate divergence from `plugin enable` (which is atomic per plugin), because `catalog update` is a multi-plugin batch operation where partial progress is desirable.

---

## `tome catalog remove` — Phase 2 extension

### New behaviour

Before any Phase 1 removal logic runs:

1. Query the index for `SELECT DISTINCT plugin FROM skills WHERE catalog = ? AND enabled = 1`.
2. If the result is non-empty AND `--force` is NOT passed:
   - Exit 53 (`CatalogHasEnabledPlugins`) with a message listing the enabled plugins:

   ```
   error: catalog 'midnight-experts' has 2 enabled plugins.
     - midnight-experts/compact-expert
     - midnight-experts/ledger-helper
   Disable them first (tome plugin disable <id>), or pass --force to cascade.
   ```

3. If `--force` is passed: for each enabled plugin in the catalog, run the disable flow (set `enabled = 0` and DELETE the rows). Inside the advisory lockfile, single transaction. Then proceed with Phase 1 catalog removal.

### Output (human, cascade case)

```
Cascading disable of 2 enabled plugins:
  ✓ midnight-experts/compact-expert (12 skill rows dropped)
  ✓ midnight-experts/ledger-helper (8 skill rows dropped)
Removing catalog midnight-experts… done.
```

### Output (`--json`, cascade case)

Phase 1 catalog-remove JSON extended with a `cascade` array:

```json
{
  "catalog": "midnight-experts",
  "removed": true,
  "cascade": [
    { "plugin": "midnight-experts/compact-expert", "skills_dropped": 12 },
    { "plugin": "midnight-experts/ledger-helper", "skills_dropped": 8 }
  ]
}
```

### Errors

| Trigger | Exit |
|---|---|
| Catalog has enabled plugins (no `--force`) | 53 |
| Index busy | 50 |
| Index integrity failure | 51 |
| Phase 1 errors (catalog not found, etc.) | Phase 1 codes |

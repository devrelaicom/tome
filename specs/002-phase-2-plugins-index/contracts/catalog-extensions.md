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

### Output (human, single-line-per-plugin reindex summary appended after the Phase 1 sync output)

```
Refreshed catalogs:
  midnight-experts        ↑ 3 commits
  another-catalog         (up-to-date)

Reindexed plugins:
  midnight-experts/compact-expert: added 0 · modified 2 · removed 0 · unchanged 10
  midnight-experts/ledger-helper:  added 1 · modified 0 · removed 0 · unchanged 7

Warnings:
  midnight-experts/broken-plugin: plugin.json malformed — disabled and de-indexed
```

The reindex summary is rendered as one line per plugin rather than a multi-column comfy-table — the human format mirrors the JSON's `plugin_change` envelopes (one envelope per plugin) and keeps cheap-skip output compact.

### Output (`--json`)

`tome catalog update --json` emits **NDJSON** — one JSON object per line. There are three envelope types, written in this order:

1. One `{"refreshed": ...}` or `{"pinned": ...}` envelope per catalog (Phase 1 behaviour, preserved).
2. One `{"plugin_change": ...}` envelope per reindexed plugin (Phase 2 extension).
3. `auto_disabled` plugins emit a `{"plugin_change": ...}` envelope with an `auto_disabled_reason` field rather than a separate top-level array — the reindex summary fields are set to zero.

Example (three lines, each a self-contained JSON object):

```json
{"refreshed":{"catalog":"midnight-experts","commits":3}}
{"plugin_change":{"plugin":"midnight-experts/compact-expert","skills_added":0,"skills_modified":2,"skills_removed":0,"skills_unchanged":10}}
{"plugin_change":{"plugin":"midnight-experts/broken-plugin","auto_disabled_reason":"plugin.json malformed","skills_added":0,"skills_modified":0,"skills_removed":0,"skills_unchanged":0}}
```

The NDJSON shape is consistent with `tome plugin list` and `tome models download` — Tome's JSON-output convention for batch operations is one envelope per logical record rather than one aggregate object.

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
  ✓ midnight-experts/compact-expert
  ✓ midnight-experts/ledger-helper
Removed catalog `midnight-experts` (cache cleared at /home/alice/.local/share/tome/catalogs/midnight-experts).
```

`✓` is rendered on TTY only; non-TTY contexts omit the glyph.

### Output (`--json`, cascade case)

The Phase 1 catalog-remove JSON envelope is preserved and extended with a `cascade` array. The Phase 1 envelope shape (`removed` carries the full record rather than a boolean) is retained because it carries more useful detail than a bare flag:

```json
{
  "removed": {
    "name": "midnight-experts",
    "url": "https://github.com/midnight-network/midnight-experts.git",
    "cache_path": "/home/alice/.local/share/tome/catalogs/midnight-experts",
    "cascade": [
      { "plugin": "midnight-experts/compact-expert", "skills_dropped": 12 },
      { "plugin": "midnight-experts/ledger-helper", "skills_dropped": 8 }
    ]
  }
}
```

`cascade` is `skip_serializing_if = "Vec::is_empty"` — a no-cascade remove (catalog had no enabled plugins) emits the Phase 1 envelope unchanged. Each `skills_dropped` value is the real per-plugin row count; the array preserves the input order.

### Errors

| Trigger | Exit |
|---|---|
| Catalog has enabled plugins (no `--force`) | 53 |
| Index busy | 50 |
| Index integrity failure | 51 |
| Phase 1 errors (catalog not found, etc.) | Phase 1 codes |

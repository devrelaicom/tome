# `tome reindex` — Command Contract

```
tome reindex [<scope>]
            [--force]
            [--json]
```

`<scope>` is one of:

- omitted — all enabled content.
- `<catalog>` — all enabled plugins under one catalog.
- `<catalog>/<plugin>` — one plugin.

## Behaviour

1. Resolve scope. Unknown catalog → exit 3. Unknown plugin → exit 20.
2. Acquire the index advisory lock. On contention → exit 50.
3. For every skill in scope:
   - Compute new `content_hash` from the on-disk SKILL.md frontmatter (lenient parse, FR-011 / FR-012 fallbacks apply).
   - If unchanged and `--force` is not set: skip.
   - Otherwise: re-embed. Within one SQLite transaction:
     - UPDATE the `skills` row.
     - DELETE the old `skill_embeddings` row and INSERT the new vector.
4. Report a summary: skills checked, skills re-embedded, skills unchanged.

## `--force`

Forces re-embedding of every skill in scope regardless of `content_hash`. Used for embedder upgrades (FR-016 recovery path) and integrity recovery.

## Output (human)

```
Reindexing midnight-experts (12 plugins, 156 skills)…
[#####################################] 156/156 skills · 41.2s
Re-embedded:  4
Unchanged:  152
```

Progress bar suppressed when stderr is not a TTY.

## Output (`--json`)

```json
{
  "scope": "midnight-experts",
  "plugins_visited": 12,
  "skills_checked": 156,
  "skills_re_embedded": 4,
  "skills_unchanged": 152,
  "duration_ms": 41203
}
```

## Errors

| Trigger | Exit |
|---|---|
| Unknown catalog | 3 |
| Unknown plugin | 20 |
| Skill header malformed | 23 (only when the malformed skill is in scope — per FR-013c, malformed-yaml-body skips with warning; malformed delimiters exit) |
| Embedder missing | 30 |
| Embedder corrupt | 31 |
| Inference runtime init failed | 34 |
| Vector ext init failed | 35 |
| Embedding generation failed | 36 |
| Index busy | 50 |
| Index integrity check failure | 51 |
| Schema too new | 52 |

## Notes

- Reindex never changes a plugin's `enabled` flag.
- A reindex of `<catalog>/<plugin>` that re-embeds every skill is the recovery path for FR-016 (embedder version drift). After reindex, `tome query` no longer refuses.

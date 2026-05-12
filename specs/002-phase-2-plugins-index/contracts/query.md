# `tome query` — Command Contract

```
tome query <text>
          [--top-k N]
          [--catalog <name>]
          [--plugin <name>]
          [--no-rerank]
          [--strict] [--min-score <float>]
          [--json]
```

## Behaviour

1. Open the index DB read-only. Run schema-version check (exit 52 on too-new). Check embedder/reranker drift against `meta` (exit 41 / 42 on embedder drift; reranker drift is allowed — see below).
2. Load the embedder; if missing → exit 30 (`ModelMissing`) or download prompt (TTY only).
3. Embed `<text>` (FR-014's composition does not apply here — query text is embedded as-is).
4. Pull top-K candidates from `skill_embeddings` joined with `skills WHERE enabled = 1` (and the optional catalog/plugin filters). K defaults to 10 (FR-027); when reranking is on, the candidate pool is expanded (typically 4× top-K) before reranking and trimmed back after.
5. If `--no-rerank`: skip the reranker stage. Scores are cosine similarity; prepend a one-line banner in human output: `(reranker disabled — showing embedding similarity)`.
6. Otherwise: rerank with `bge-reranker-base`. Scores are raw logits.
7. If `--strict`: drop results below `--min-score`. If no results remain → exit 40 (`QueryNoResultsStrict`). Without `--strict`, return the top-K regardless of score.
8. Render.

## Output (human, default — `comfy-table`)

```
Score   Catalog            Plugin             Skill                 Version  Path
──────  ─────────────────  ─────────────────  ────────────────────  ───────  ──────────────────────────────────────────
 3.142  midnight-experts   compact-expert     compact-language-ref  1.2.0    ~/.local/share/tome/catalogs/.../SKILL.md
 2.891  midnight-experts   compact-expert     compact-debugging     1.2.0    ~/.local/share/tome/catalogs/.../SKILL.md
 …
```

- `Score` is right-aligned, four decimal places.
- `Path` is rendered with `~` shorthand when under the user's home.
- Long skill names truncate with an ellipsis if the terminal is too narrow; the JSON output never truncates.

## Output (`--json`)

```json
{
  "scoring": "reranked",
  "threshold_passed": true,
  "results": [
    {
      "catalog": "midnight-experts",
      "plugin": "compact-expert",
      "skill": "compact-language-ref",
      "plugin_version": "1.2.0",
      "score": 3.142,
      "path": "/Users/alice/.local/share/tome/catalogs/.../skills/compact-language-ref/SKILL.md",
      "scoring": "reranked"
    }
  ]
}
```

When `--no-rerank` is set, `scoring` is `"embedding-similarity"` at both top-level and per-result.

## Reranker drift handling

If `meta.reranker_name` or `meta.reranker_version` disagrees with the configured reranker, the query proceeds (the stored vectors are still valid). A one-line warning is printed to stderr in human mode and a `"reranker_drift"` field appears in JSON output. Exit code remains the result-driven one.

## Embedder drift handling

If `meta.embedder_name` disagrees → exit 41. If `meta.embedder_version` disagrees → exit 42. Message in both: "Stored vectors were produced by a different embedder. Run `tome reindex --force` to rebuild."

## TTY behaviour

- Stdout TTY: colour, table glyphs, score column right-aligned with figures-aligned font.
- Stdout not a TTY: plain ASCII separators, no colour. Table data unchanged.
- Stderr TTY: spinner during model load (visible the first time per session). Suppressed on non-TTY stderr.

## Flags

| Flag | Default | Notes |
|---|---|---|
| `--top-k N` | 10 | Cap on returned results (post-rerank if rerank ran). |
| `--catalog X` | none | Filter to one catalog before retrieval. |
| `--plugin X` | none | Filter to one plugin (across all enabled catalogs unless `--catalog` is also set). |
| `--no-rerank` | off | Skip the reranker stage; scores are cosine similarity. |
| `--strict` | off | Apply the score threshold; non-zero exit on empty result. |
| `--min-score F` | 0.0 (reranker logits) / 0.5 (similarity) | Used only with `--strict`. Reasonable defaults documented; user can override. |
| `--json` | off | Structured output, byte-stable across TTY contexts. |

## Errors

| Trigger | Exit |
|---|---|
| Catalog filter unknown | 3 |
| Plugin filter unknown | 20 |
| Embedder missing | 30 |
| Embedder corrupt | 31 |
| Inference runtime init failed | 34 |
| Vector extension init failed | 35 |
| Embedding generation failed | 36 |
| Reranking failed | 37 |
| `--strict` with no results | 40 |
| Embedder name drift | 41 |
| Embedder version drift | 42 |
| Index busy (rare; query is read-only) | 50 |
| Index integrity check failure | 51 |
| Schema too new | 52 |

---
title: Search
sidebar_position: 2
---

# Search

A catalog of any useful size is hard to browse — one plugin in the demo
catalog contains 28 entries on its own. You should not have to remember what
each entry is called. Tome indexes every enabled skill and command so you can
describe what you need and get the right entry back.

## `tome query`

```bash
tome query "verify a Compact contract"
```

The query text is variadic and space-joined, so quoting is optional:
`tome query reset a counter` works unquoted. Pass `-q`/`--query "…"` instead
when the query itself contains flag-like or shell-significant tokens; the
quoted form is mutually exclusive with the positional words.

```text
top_k=10  rerank=true  min_score=none  (10 results)
|   Score | Catalog         | Plugin          | Name                                      | Type    | Version | Path                                                      |
|---------|-----------------|-----------------|-------------------------------------------|---------|---------|-----------------------------------------------------------|
|  4.7874 | midnight-expert | midnight-verify | midnight-verify:verify-by-execution       | skill   | 0.13.0  | skills/midnight-verify:verify-by-execution/SKILL.md       |
|  3.4658 | midnight-expert | midnight-verify | midnight-verify:verify-by-zkir-checker    | skill   | 0.13.0  | skills/midnight-verify:verify-by-zkir-checker/SKILL.md    |
|  3.1529 | midnight-expert | midnight-verify | midnight-verify:verify-compact            | skill   | 0.13.0  | skills/midnight-verify:verify-compact/SKILL.md            |
|  2.7010 | midnight-expert | midnight-verify | midnight-verify:verify-by-witness         | skill   | 0.13.0  | skills/midnight-verify:verify-by-witness/SKILL.md         |
|  1.4746 | midnight-expert | midnight-verify | midnight-verify:verify-by-cli-execution   | skill   | 0.13.0  | skills/midnight-verify:verify-by-cli-execution/SKILL.md   |
|  0.0356 | midnight-expert | midnight-verify | midnight-verify:verify                    | command | 0.13.0  | commands/midnight-verify:verify.md                        |
| -0.4743 | midnight-expert | midnight-verify | midnight-verify:verify-by-source          | skill   | 0.13.0  | skills/midnight-verify:verify-by-source/SKILL.md          |
| -1.0289 | midnight-expert | midnight-verify | midnight-verify:verify-by-zkir-inspection | skill   | 0.13.0  | skills/midnight-verify:verify-by-zkir-inspection/SKILL.md |
| -1.2946 | midnight-expert | midnight-verify | midnight-verify:verify-tooling            | skill   | 0.13.0  | skills/midnight-verify:verify-tooling/SKILL.md            |
| -3.3564 | midnight-expert | midnight-verify | midnight-verify:verify-ledger             | skill   | 0.13.0  | skills/midnight-verify:verify-ledger/SKILL.md             |
```

The right skill is at the top with a clear margin, and the scores drop
steeply — below zero for entries that only share vocabulary with the query.

The dim header line above the table shows the effective knobs that produced
the results — the resolved `top_k`, whether reranking ran, the applied
`min_score` floor (or `none` when no floor is enforced), and the result count.
It is shown only in an interactive terminal; piped or redirected output omits
it so the table stays clean to `grep`. The `Type` column reports whether each
result is a `skill`, `command`, or `agent`.

Search runs in two stages:

1. **KNN retrieval** — your query is embedded with a local model and matched
   against the vector index to retrieve the nearest candidates.
2. **Reranking** — a local cross-encoder reranker re-scores those candidates so
   the most relevant results are ranked first.

Both models run on your machine; nothing is sent anywhere.

## Scoping and flags

| Flag | Effect |
| --- | --- |
| `--top-k <n>` | Return at most *n* results. |
| `--min-score <s>` | Drop results scoring below *s*. |
| `--no-rerank` | Skip the reranking stage; results come back in raw KNN order. |
| `--catalog <name>` | Restrict the search to a catalog. Repeatable: pass `--catalog` several times to include entries from any of the named catalogs. |
| `--plugin <name>` | Restrict the search to a plugin (across all catalogs unless `--catalog` is also set). Repeatable: include entries from any of the named plugins. |
| `--kind <kind>` | Restrict the search to an entry kind (`skill`, `command`, or `agent`). Repeatable. `query` only searches indexed, searchable entries, so `--kind agent` typically returns nothing. |
| `-q`, `--query <text>` | The query as a single quoted string, instead of the positional words. Mutually exclusive with the positional form. |
| `--strict` | Fail (non-zero exit) instead of returning weak results when no result scores high enough. |
| `--json` | Emit machine-readable output. |

The repeatable scoping flags compose, so you can narrow by kind and several
plugins at once:

```bash
tome query reset a counter --kind skill --plugin a --plugin b
```

### Limit results with `--top-k`

```bash
tome query "verify a Compact contract" --top-k 3
```

```text
top_k=3  rerank=true  min_score=none  (3 results)
|  Score | Catalog         | Plugin          | Name                                   | Type  | Version | Path                                                   |
|--------|-----------------|-----------------|----------------------------------------|-------|---------|--------------------------------------------------------|
| 4.3648 | midnight-expert | midnight-verify | midnight-verify:verify-by-execution    | skill | 0.13.0  | skills/midnight-verify:verify-by-execution/SKILL.md    |
| 3.8602 | midnight-expert | midnight-verify | midnight-verify:verify-by-zkir-checker | skill | 0.13.0  | skills/midnight-verify:verify-by-zkir-checker/SKILL.md |
| 3.6187 | midnight-expert | midnight-verify | midnight-verify:verify-compact         | skill | 0.13.0  | skills/midnight-verify:verify-compact/SKILL.md         |
```

Same query, same top three entries — different scores than the ten-result run
above. Reranker scores are relative to the candidate set, not absolute, so
compare scores within a single run, never across runs.

## Why search matters: load on demand

The point of search is **load on demand**. Instead of loading every skill into
your agent's context in advance, the agent searches at runtime and loads only
what the current task needs. That:

- **protects the context window** — skills that aren't relevant never take up
  space;
- **cuts token spend** — you pay for the skills you use, not for every enabled
  entry;
- **scales** — a large catalog stays usable because retrieval, not context size,
  does the filtering.

For example: the top result above, `verify-by-execution`, is a single
SKILL.md of 11,652 characters (1,539 words). Loading it costs one skill's
worth of context — the plugin's other 27 entries are not loaded.

Inside a configured harness, this same search runs over the
[MCP server](./mcp-server.md), so your agent gets search and skill loading
without you running `tome query` by hand.

## Pitfalls

| Exit code | What happened | What to do |
| --- | --- | --- |
| `40` | `--strict` was set and no result scored high enough. | Expected in scripts — treat it as "no match", or broaden the query. See [exit codes](../reference/exit-codes.md). |

## Where next

- [MCP server](./mcp-server.md) — the same search, driven by your agent
  mid-task.
- [Plugins & catalogs](./plugins-and-catalogs.md) — what gets indexed, and
  when.

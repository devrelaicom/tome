# NUMBERS.md — the only numbers and names docs/landing may use

Source: `devrelaicom/midnight-expert` (local Claude Code marketplace clone at
`~/.claude/plugins/marketplaces/midnight-expert`), converted with
`tome catalog convert` (tome 0.6.0) on 2026-06-10. Every figure below is read
from a capture file in this directory — cite nothing that is not here.

## Slug rule

Docs and the landing demo show the intended public slug
`devrelaicom/midnight-expert-tome` in `catalog add` command lines (the publish
dependency is recorded in spec §11 as a launch hard-stop). They may quote
count/score lines from these captures — but **never a line embedding the local
`/tmp` (or `/private/tmp`) path**, and never the `file://` source line from
`catalog-add.txt` / `catalog-show.txt`.

## Canonical demo plugin

- **`midnight-expert/midnight-verify`** v0.13.0 (chosen in Step 4; it owns
  every hit for the canonical query below).

## Catalog counts

- **Plugins in the catalog: 13** (`catalog-add.txt`: "plugins: 13";
  `catalog-show.txt` lists all 13).
- Converted catalog version: v0.46.0, name `midnight-expert-tome`
  (`catalog-show.txt`).
- Conversion: **616 files written, 128 warnings, 100 infos**, exit 0
  (`convert.txt`).
- Lint of the converted catalog: **0 errors, 18 warnings, 0 infos**, exit 0
  (`lint.txt`).

## Entry counts (canonical plugin, midnight-verify)

- **19 skills, 2 commands, 7 agents** = **28 indexed entries**
  (`plugin-list.txt`; `status.txt` confirms "28 skills indexed").
- Hooks: the converted plugin carries a `hooks/hooks.json` (pass-through), but
  hooks are not indexed entries and the CLI reports no hook count — do not
  state a hook number.

## Canonical query

`tome query "verify a Compact contract"` (`query.txt`) — top hits with scores:

| Score  | Entry                                   |
|--------|-----------------------------------------|
| 4.7874 | `midnight-verify:verify-by-execution`   |
| 3.4658 | `midnight-verify:verify-by-zkir-checker`|
| 3.1529 | `midnight-verify:verify-compact`        |
| 2.7010 | `midnight-verify:verify-by-witness`     |
| 1.4746 | `midnight-verify:verify-by-cli-execution`|

(10 rows total in the capture; all from midnight-verify.)

**`--top-k 3` caveat:** `query-topk.txt` shows *different* scores for the same
top hits (4.3648 / 3.8602 / 3.6187) — reranker scores depend on the candidate
set. Quote each capture's scores only alongside its own command line; never mix.

## Top-hit size (for the "loaded one skill" framing)

- `midnight-verify:verify-by-execution/SKILL.md` measures **11,652 characters /
  1,539 words** (`wc` on the converted file). **No token count was captured** —
  do not state a token figure (e.g. "4.2k tokens") unless a real tokenizer
  measurement is added here first.

## Other captured facts

- Models: embedder bge-small-en-v1.5 32 MB + reranker bge-reranker-base 266 MB
  (+ optional summariser qwen2.5-0.5b-instruct 469 MB) — `models-list.txt`.
- Meta skill: `convert-marketplace` rev `1a57b1e8df28caf7` (`meta-list.txt`).
- `status.txt` / `doctor.txt`: schema version 4, drift none, overall healthy.

## Honest-failure capture

- `plugin-enable.txt` ends in a llama.cpp abort (exit 134,
  `GGML_ASSERT(n_tokens_all <= cparams.n_batch)`) in the post-commit
  summariser; the enable itself **committed** (see `plugin-list.txt`). Do not
  quote it as success output. A clean success line for this enable is not
  capturable on tome 0.6.0 with this catalog (CLI bug — reported in the Task 3
  deviations).

---
title: Troubleshooting
sidebar_position: 7
---

# Troubleshooting

When something looks wrong, Tome gives you two read-only commands to diagnose
it and one flag to repair it: `status` for a quick check, `doctor` for a full
report, and `doctor --fix` for repairs.

## `tome status`

A fast, read-only pre-flight check. It never takes the index lock, so it's safe
to run any time — even while another Tome command is working.

```bash
tome status
```

```text
Tome:               0.6.0
Embedder:           bge-small-en-v1.5 (1.5)  [ok]
Reranker:           bge-reranker-base (base)  [ok]
Index database:     [ok] (1 plugins enabled, 28 skills indexed, 1.6 MiB)
Schema version:     4
Drift:              none
Overall:            [ok] healthy
```

`--verify` runs deeper checks; `--json` emits machine-readable output.

## `tome doctor`

`doctor` reports on every subsystem — index, models, harness config,
workspaces, installed meta skills — and can repair them. It is **read-only by
default**; it only writes when you pass `--fix`.

```bash
tome doctor            # report only (no writes)
tome doctor --fix      # repair what it safely can
tome doctor --force    # apply fixes it would otherwise hold back
tome doctor --verify   # deeper checks
tome doctor --json     # machine-readable
```

The report starts with the same information as `status`, in more detail:

```text
Tome:            0.6.0

Workspace:       (global)
  resolved via:  global fallback
  catalogs:      1
  plugins:       1 total, 1 enabled

Models:
  embedder       bge-small-en-v1.5 (1.5)  [ok] ok
  reranker       bge-reranker-base (base)  [ok] ok

Index database:  [ok] (1 plugins enabled, 28 skills indexed, 1.6 MiB)
Schema version:  4
Drift:           none
```

…and continues through catalog caches and detected harnesses before the
overall verdict (`Overall: [ok] healthy` when everything is healthy).

`--fix` re-runs the same idempotent reconcilers the normal commands use, so a
repair inherits all their safety (marker-bounded edits, structural-match-only
removal, symlink refusal). It won't take a destructive shortcut.

## Common failures

Every failure maps to a specific exit code — no generic "something went wrong".
Common cases:

| Symptom | Exit code | Fix |
| --- | --- | --- |
| Plugin has a legacy `.claude-plugin/plugin.json` but no `tome-plugin.toml` | `80` | Run `tome plugin convert` — see [Converting](../authoring/convert.md). |
| `convert` can't tell what format the source is | `83` | Pass `--from <harness>` explicitly. |
| `meta add` finds no harness to install into | `89` | Pass `--harness <name>`, or set up a supported harness first. |
| `query --strict` found nothing | `40` | Broaden the query, or drop `--strict`. |
| The index is busy (another process holds the lock) | `50` | Wait for it to finish; `tome status` is always safe meanwhile. |
| `catalog remove` refuses — plugins still enabled | `53` | Disable them first, or `--force` to cascade. |
| A required model is missing | `30` | `tome models download`. |
| Embedder name or version drift between index and models | `41` / `42` | `tome reindex --force`. |
| `create`/`convert` refuses to overwrite existing output | `81` | Choose a fresh `--output`, or pass `--force`. |
| `lint` found errors, or warnings under `--strict` | `85` / `86` | These are verdicts, not crashes — fix the findings. See [Linting](../authoring/lint.md). |

The complete table, codes 0–89, is in
[Exit codes](../reference/exit-codes.md).

## Common issues

- **A harness's config drifted** — run `tome harness sync`, or
  `tome doctor --fix` to reconcile rules, MCP wiring, agents, and hooks from
  current state.
- **Search returns nothing or stale results** — reindex the affected scope:
  `tome reindex` (everything), `tome reindex <catalog>`, or
  `tome reindex <catalog>/<plugin>`. Add `--force` to rebuild from scratch.
- **An installed meta skill is missing, stale, or modified** — `tome doctor`
  reports it; `tome doctor --fix` re-installs from the bundled copy. See
  [Meta skills](./meta-skills.md).

If `doctor` reports a problem it can't fix safely, it tells you what it found so
you can resolve it by hand.

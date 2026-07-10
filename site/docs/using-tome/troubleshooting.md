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
Tome v0.7.16

Global
Models:       embedder [ok] · reranker [ok] · summariser [ok]
              ~325 MB on disk
Workspaces:   2
Index:        schema v6 · 1.6 MiB · [ok] integrity
Drift:        none

Workspace
Current:      contracts (project)
Entries:      19 skills · 2 commands · 7 agents
Catalogs:     1 enrolled
Reindexed:    just now
```

In an interactive terminal a coloured "bookshelf" is drawn above this panel;
piped or `--json` output drops the art. The figures are representative — yours
reflect your own models, catalogs, and workspaces. `--verify` runs deeper
checks; `--json` emits machine-readable output.

On a brand-new install with no catalog enrolled, `status` renders a "Getting
started" block naming the real first-run flow instead of reading as broken:

```text
Getting started
Not set up yet — start with:
  1. tome catalog add <source>
  2. tome plugin enable <catalog>/<plugin>
  3. tome harness use <name>
  4. tome query "<what you need>"
```

This is guidance, not a failure — the health verdict and exit code are
unchanged, and `--json` carries the same state through the existing
`catalogs_enrolled` and `entries` fields rather than the onboarding prose.

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
✓ healthy — 0 failing, 0 warnings, 24 ok

Tome:            0.7.16

Workspace:       global
  resolved via:  global fallback
  catalogs:      1
  plugins:       1 total, 1 enabled

MCP server log:  ~/.tome/logs/mcp.log

Models:
  embedder       bge-small-en-v1.5 (1.5)  [ok] ok
  reranker       bge-reranker-base (base)  [ok] ok

Index database:  [ok] (1 plugins enabled, 28 skills indexed, 1.6 MiB)
Schema version:  6
Drift:           none
```

…and continues through catalog caches and detected harnesses before the
overall verdict (`Overall: [ok] healthy` when everything is healthy).

When a model capability is served by an external
[provider](../reference/config.md#model-providers-byokbyom), its bundled local
model row reads `not_applicable` rather than `missing`, and `--fix` does not
download it — the provider supplies that capability instead.

`--fix` re-runs the same idempotent reconcilers the normal commands use, so a
repair inherits all their safety (marker-bounded edits, structural-match-only
removal, symlink refusal). It won't take a destructive shortcut.

### Informational advisories

Some of what `doctor` reports is guidance, not a fault. These entries never
affect the health verdict or the exit code, so a pristine install stays healthy
and doesn't flip to a spurious [exit `75`](../reference/exit-codes.md).

On a fresh install, `doctor` emits onboarding suggestions under the
`onboarding` subsystem for the not-set-up conditions — no catalog enrolled, no
plugins once a catalog exists, no harness configured. They are `auto_fixable:
false` and are excluded from the `--fix` remaining-manual-fixes gate, so
`doctor --fix` on a pristine install won't report them as unfixed manual work.

`doctor` also reports **unrepresented hooks** — enabled-plugin hook events that
a harness can only receive as `GUARDRAILS.md` prose because it has no native
hook mechanism for them, so the hook is described rather than enforced. `status`
shows an `unrepresented_hooks` count and `tome harness info` shows a
`hooks_notice`. It is an advisory, not a failure, and mirrors the parallel
report for unrepresented agents (agents with no native form on a rules-only
harness).

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
| Embedder name or version drift between index and models | `41` / `42` | Run a **bare** `tome reindex` (add `--force` to rebuild every skill). Recovering from drift, or switching embedders, needs the whole-index form — a scoped reindex is refused under drift (exit `47`). |
| `create`/`convert` refuses to overwrite existing output | `81` | Choose a fresh `--output`, or pass `--force`. |
| `lint` found errors, or warnings under `--strict` | `85` / `86` | These are verdicts, not crashes — fix the findings. See [Linting](../authoring/lint.md). |
| A configured external provider has no resolvable credential | `93` | Set `TOME_<NAME>_API_KEY` — `tome models test <capability>` and `tome doctor` name the exact variable. A remote request that fails outright is `94`; a remote embedding that fails content validation is `95`. See [Model providers](../reference/config.md#model-providers-byokbyom). |
| `config.toml` is malformed | `5` | Run `tome config validate` — it names the offending key, section, and line. `tome doctor` and `tome status` keep running and report the same problem as a finding rather than bricking. See [Configuration](../reference/config.md#global-configtoml). |

The complete table, codes 0–95, is in
[Exit codes](../reference/exit-codes.md).

## Guidance in output

Tome's human output points you at the next step so a result is never a dead end.

Successful commands print a `next:` hint: `catalog add` points at browsing and
enabling plugins, `plugin enable` at querying and syncing, `workspace use` at
enrolling a catalog. Common first-run errors carry a recovery `hint:` line
pointing at the right command — a `CatalogNotFound` names `tome catalog list`
and `tome catalog add`, a `PluginNotFound` names `tome plugin list`, and an
invalid `<catalog>/<plugin>` id does the same.

Human error output is styled: a red `error:` prefix with the `hint:` and any
other continuation lines dimmed and indented under it. The styling is gated on
colour, so a piped, `NO_COLOR`, or `--no-color` invocation prints the plain
`error: …` / `hint: …` text. `--json` output is unaffected — the `hint:` rides
the error message into both stderr and the `--json` error envelope, while
`next:` is human-stdout only.

Empty results carry the same nudge. `tome query` distinguishes an empty corpus
("No skills indexed for this scope yet — enable a plugin …") from a genuine
no-match ("No match — try rephrasing …"), so you know whether to index or to
rephrase. `tome plugin list` is catalog-aware: it tells you to add a catalog
when none are enrolled, and to enable a plugin when catalogs exist but none are
enabled. A bare `tome` or `tome --help` ends with a four-step "Getting started"
quickstart. All of these are human-only; `--json` is untouched.

## Common issues

- **A harness's config drifted** — run `tome sync`, or
  `tome doctor --fix` to reconcile rules, MCP wiring, agents, and hooks from
  current state.
- **A translated plugin hook doesn't seem to fire** — translated hooks fail
  open by design, so a misconfigured hook is silent rather than blocking. Run
  `tome harness run-hook --explain --event <event> --harness <name>` for a
  dry-run of what would fire, or set `TOME_HOOK_DEBUG=1` to trace a live
  dispatch. `tome harness info <name>` and `tome doctor` print the same
  pointer under their hook-translation sections.
- **Search returns nothing or stale results** — reindex the affected scope.
  `tome reindex` alone rebuilds everything; positional scopes are variadic and
  accept `*` globs (`tome reindex "<catalog>/*"`, `tome reindex "compact-*"`),
  and `--catalog` / `--plugin` (both repeatable) select by named flag. Add
  `--force` to re-embed every in-scope skill regardless of its content hash. See
  the [reindex reference](../reference/commands.md#tome-reindex) for the full
  form list. One caveat: only a bare `tome reindex` restamps the embedder
  identity, so a scoped reindex is refused under embedder drift (exit `47`) —
  recover with the bare form.
- **An installed meta skill is missing, stale, or modified** — `tome doctor`
  reports it; `tome doctor --fix` re-installs from the bundled copy. See
  [Meta skills](./meta-skills.md).
- **The MCP server never starts on a PATH-less or sandboxed host** (a CI runner,
  an agent SDK) — an older Tome wrote the launch command as the bare name
  `tome`, resolved against the host `PATH`. Where `tome` isn't on `PATH`, the
  server never started and the agent got zero skills. Re-run
  `tome sync` (or `tome doctor --fix`) to rewrite the config with an
  absolute launcher; set `$TOME_BIN` to an absolute path first to pin the
  launcher explicitly. Re-syncing stays idempotent, so it's safe to run on hosts
  that were already fine.

If `doctor` reports a problem it can't fix safely, it tells you what it found so
you can resolve it by hand.

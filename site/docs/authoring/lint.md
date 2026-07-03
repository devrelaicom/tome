---
title: Linting
sidebar_position: 4
---

# Linting

Before you publish a catalog — or enable one you just converted — you want a
clear answer: what is malformed, what is lossy, and what will misbehave at
read time. `tome {catalog,plugin,skill} lint <PATH>...` reads one or more Tome
artifacts and reports findings. It never modifies files unless you pass
`--autofix`, and it never stops partway through a run.

## Worked example

Lint the converted `midnight-expert-tome` catalog from the
[converting](./convert.md) walkthrough:

```console
$ tome catalog lint ~/catalogs/midnight-expert-tome
[warning] lint/residual-harness-ism: shell-execution injection (`` !`cmd` ``) is not supported — Tome does not execute commands in bodies (…/midnight-expert-tome/compact-core/skills/compact-core:compact-language-ref/SKILL.md)
[warning] lint/description-too-long: entry `compact-dev` description is 2557 characters (max 1024) (…/midnight-expert-tome/compact-core/agents/compact-dev.md)
[warning] lint/residual-harness-ism: file-reference injection (`@path`) is not supported — Tome does not inject file contents (…/midnight-expert-tome/midnight-verify/skills/midnight-verify:verify-by-execution/SKILL.md)
  …
Summary: 0 error(s), 18 warning(s), 0 info(s)
```

*(Excerpt — 18 warnings in the full run; paths shortened here.)*

Each finding is one line: a severity (`error`, `warning`, `info`), a rule id,
a message, and the offending file. The `Summary` line at the end always
appears — see below.

## Lint never halts

A malformed artifact is a *finding*, not a crash. A broken manifest or
unparseable frontmatter becomes a finding against that file, and lint
continues with the rest of the tree. You always get the complete report and
the `Summary` line — never a stack trace partway through, and never a report
that silently stopped at the first bad file.

## Linting multiple sources

`lint` accepts more than one source. Pass several paths, or let the shell expand
a glob — Tome does no globbing of its own, so `plugins/*` is expanded by your
shell into one argument per match:

```bash
tome plugin lint plugins/*
tome skill lint skills/foo skills/bar skills/baz
```

Each source is linted independently, with the same never-halt guarantee applied
per source: a source that fails to parse (or mismatches the command level, or
hits an autofix I/O error) is reported as that source's failure and does **not**
abort the remaining sources. The command's exit code is the **worst verdict
across every source**: if any source has errors → `85`; else, under `--strict`,
if any source has warnings → `86`; else `0`. This makes `tome plugin lint
plugins/* --strict` a single CI gate over a whole tree.

Single-source output is unchanged. With more than one source, human output
prints a `== <source> ==` section per source, and `--json` switches from the
single object to JSONL — one `{ "source", "findings", "summary" }` record per
line (see [below](#scripting-with---json)).

Note the exit-code nuance for a source that fails *before* linting (an
unreadable path, a level mismatch, or an autofix I/O error): with a **single**
source that failure keeps its own exit code (`2` for a usage/parse error, `7`
for an I/O error), preserving the pre-existing single-source behaviour; with
**multiple** sources it is folded into the aggregate verdict and surfaces as
`85`. So `tome plugin lint bad-path` exits `2`, while `tome plugin lint
bad-path other-path` exits `85`.

## `--autofix`

`--autofix` applies the mechanically-safe fixes — rewritable harness-isms,
making an entry's `name` match its directory — then re-lints and repeats until
nothing changes (a fixpoint), reporting what it fixed and what still needs
manual edits. It does not apply fixes that require judgment: an over-long
description is flagged, not truncated.

Add `--dry-run` to see what `--autofix` *would* change without writing
anything. `--dry-run` requires `--autofix` — plain `lint` already writes
nothing, so a bare `lint --dry-run` is a usage error (exit `2`); pair the two.

```bash
tome catalog lint ~/catalogs/midnight-expert-tome --autofix --dry-run
```

## Verdicts and CI

Lint's exit code is a verdict:

| Exit | Verdict |
| --- | --- |
| `0` | Clean — or warnings only, without `--strict` |
| `85` | At least one error found |
| `86` | Warnings found under `--strict` (and no errors) |

Without `--strict`, warnings are advisory and the exit code stays `0`. With
`--strict`, warnings fail too — which is exactly what you want in CI:

```yaml
- run: tome catalog lint . --strict
  # exit 85 = errors found, 86 = warnings under --strict, 0 = clean
```

Run this before tagging a release, and unconverted content never reaches your
users. The full code table lives in the
[exit code reference](../reference/exit-codes.md).

## Scripting with `--json`

For a single source, `--json` emits a single object —
`{ "findings": [...], "summary": {...} }` — suitable for `jq`. Every finding
carries the same fields:

```json
{
  "rule": "lint/description-too-long",
  "severity": "warning",
  "message": "entry `compact-dev` description is 2557 characters (max 1024)",
  "file": "compact-core/agents/compact-dev.md",
  "line": null,
  "autofixable": false
}
```

`file` and `line` come from the finding's location: `file` is the offending
path (or `null` when the finding has no location), and `line` is a 1-based line
number (or `null` when the location has no line). `autofixable` is `true` when
`--autofix` can apply the fix mechanically.

The per-finding shape is identical to a `convert --json` diagnostic line — see
[converting](./convert.md#scripting-with---json) for the shared contract. A
script that reads lint findings can read convert diagnostics with the same
parser; convert only adds a `"type": "diagnostic"` discriminator and wraps the
stream as JSONL.

With **multiple** sources, `lint --json` also becomes JSONL: one record per
source, each the single-source object with a leading `"source"` field naming the
path it came from — `{ "source": "...", "findings": [...], "summary": {...} }`.
A source that failed to lint still emits a record (its failure is surfaced as a
`lint/source-failed` error finding plus an `"error"` field), so the stream
accounts for every source you passed. The single-source object is left exactly
as above — the `"source"` wrapper is added only when there is more than one.

## Pitfalls

- **Warnings don't fail by default.** A pipeline that gates on lint's exit
  code passes a catalog full of warnings unless you pass `--strict`.
- **`85` and `86` are verdicts, not crashes.** Don't retry them; read the
  findings. A genuinely broken invocation fails with a different code — see
  the [exit code reference](../reference/exit-codes.md).
- **`--autofix` applies only mechanical fixes.** Residual injections (`@path`,
  shell execution) and over-long descriptions stay flagged until you edit the
  files yourself.

## Next steps

- [Converting](./convert.md) — where most lint warnings come from.
- [Creating](./create.md) — scaffolds are lint-clean by construction.
- [Distributing](./distributing.md) — `lint --strict` belongs in your CI
  before you tag.

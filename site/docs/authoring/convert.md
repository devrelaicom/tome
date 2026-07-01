---
title: Converting
sidebar_position: 3
---

# Converting

You probably already have plugins — a Claude Code marketplace, a Codex
project, or a set of `SKILL.md` files written for Cursor or OpenCode.
`tome catalog convert` (and its `plugin` and `skill` siblings) turns
what you have into native Tome artifacts, reports exactly what it could and
could not convert, and never modifies the original. Convert instead of
rewriting.

## What converts

The command names what you want to produce: a whole catalog (`catalog`), one
plugin (`plugin`), or a single skill (`skill`). The source format is
auto-detected; `--from` overrides detection when it can't decide.

| Source (`--from`) | What it covers |
| --- | --- |
| `claude-code` | Claude Code marketplaces, plugins, and skills |
| `codex` | Codex projects |
| `cursor` | Native `SKILL.md` trees written for Cursor |
| `opencode` | Native `SKILL.md` trees written for OpenCode |
| `cline` | Native `SKILL.md` trees written for Cline |
| `agent-skills` | The generic Agent Skills `SKILL.md` layout |

When detection fails, convert exits `83` and asks you to pass `--from` rather
than guessing.

## A marketplace becomes a catalog

The catalog used throughout these docs is a real conversion: the
`devrelaicom/midnight-expert` Claude Code marketplace — 13 plugins — converted
in one command. Always start with `--dry-run`, which prints the full plan and
writes nothing:

```console
$ tome catalog convert ~/.claude/plugins/marketplaces/midnight-expert --output ~/catalogs --dry-run
Would convert claude-code catalog `midnight-expert` → `midnight-expert-tome`
  [info] convert/catalog-synthesized-field: marketplace has no `description`; synthesizing one
  [info] convert/dropped-manifest-field: plugin.json `homepage` is not modelled by Tome; dropping it
  [warning] harness-ism/claude-skill-dir: rewrote 6 occurrence(s) of `${CLAUDE_SKILL_DIR}` → `${TOME_SKILL_DIR}`
  [warning] harness-ism/claude-plugin-data: rewrote 2 occurrence(s) of `${CLAUDE_PLUGIN_DATA}` → `${TOME_PLUGIN_DATA}`
  …
  [warning] convert/tool-restriction-dropped: frontmatter `allowed-tools` (a tool restriction) is dropped — Tome does not constrain tools, so dropping it silently broadens capability
  [warning] convert/agent-lossy: agent frontmatter `memory` is not modelled by Tome; dropping it (agent conversion is lossy)
  …
Dry run: 616 file(s) to …/midnight-expert-tome  (128 warning(s), 100 info(s))
```

*(Excerpt — the full plan lists every diagnostic; output paths shortened here.)*

Read the warnings before you continue. Each one records a decision convert
made for you: a rewritten variable, a dropped manifest field, a tool
restriction it cannot enforce. When you are satisfied, run the same command
without `--dry-run`:

```console
$ tome catalog convert ~/.claude/plugins/marketplaces/midnight-expert --output ~/catalogs
Converted claude-code catalog `midnight-expert` → `midnight-expert-tome`
  …
Done: 616 file(s) to …/midnight-expert-tome  (128 warning(s), 100 info(s))
```

Then lint the result to see what still needs manual review:

```console
$ tome catalog lint ~/catalogs/midnight-expert-tome
[warning] lint/description-too-long: entry `compact-dev` description is 2557 characters (max 1024) (…/midnight-expert-tome/compact-core/agents/compact-dev.md)
[warning] lint/residual-harness-ism: file-reference injection (`@path`) is not supported — Tome does not inject file contents (…/midnight-expert-tome/midnight-verify/skills/midnight-verify:verify-by-execution/SKILL.md)
  …
Summary: 0 error(s), 18 warning(s), 0 info(s)
```

Zero errors: the catalog works as-is. The 18 warnings are what remains for
manual review — over-long descriptions and harness-isms Tome does not
emulate. [Linting](./lint.md) covers how to work through them (and what
`--autofix` can do for you).

## Output location

Convert never modifies the source — it writes a converted copy:

- The copy is named `<source-name>-tome` by default. Override it with the
  positional `NAME` argument or `--name` (supplying both with different values
  is a usage error).
- Convert writes the copy under the current directory by default;
  `--output <dir>` selects a different parent.
- `--into <path>` places the converted artifact inside an existing Tome
  artifact instead: a plugin converted `--into` a catalog is registered in its
  `tome-catalog.toml`; a skill converted `--into` a plugin is written to its
  `skills/` directory. `--into` and `--output` are mutually exclusive.
- If the destination already contains files convert wants to write, it refuses
  with exit `81`. `--force` overwrites the colliding files — only those files,
  never the whole directory.

## Remote sources

The source can be a local path, an `owner/repo` shorthand, or a git URL.
Remote sources are fetched as a shallow clone into a temporary directory and
cleaned up on every exit path — success, failure, or `--strict` abort. If the
source string could be read both ways (a local directory named `owner/repo`
exists), the local path wins.

For `tome catalog convert` only, `--no-fetch` skips the marketplace's
remote-source plugins: they're warned and skipped instead of fetched. The
source argument itself may still be remote.

## What gets rewritten

Claude Code bodies use variables that Tome does not support directly. Convert
rewrites them to their Tome equivalents and reports every occurrence:

| Claude Code | Tome |
| --- | --- |
| `${CLAUDE_SKILL_DIR}` | `${TOME_SKILL_DIR}` |
| `${CLAUDE_PLUGIN_DATA}` | `${TOME_PLUGIN_DATA}` |
| `${CLAUDE_PLUGIN_ROOT}` | `${TOME_PLUGIN_DIR}` |

Legacy positional arguments (`$1`..`$9`) are rewritten to Tome's 0-based
argument substitution.

## What cannot be converted

Some features Tome cannot represent, and convert reports them instead of
silently dropping them. Hook monitors, LSP servers, themes, output styles, and
the other unsupported components are each reported as a warning that names
exactly what was lost. Two injections can't be rewritten at all —
file-reference injection (`@path`) and shell-execution injection — because
Tome does not inject file contents or execute commands in bodies; convert
warns and leaves them in place for you to review, and `lint` keeps flagging
them afterwards as `lint/residual-harness-ism`.

If you prefer to fail rather than lose anything, pass `--strict`: any
unsupported feature aborts the conversion with exit `84`, writing nothing. The
error names how many blocking findings there were and the distinct rule-ids
behind them, so you can see the whole set in one run rather than fixing them one
at a time.

When a drop is intentional — a plugin that deliberately ships a `themes/`
directory Tome cannot carry, say — demote that rule with `--allow <rule-id>` so
it no longer blocks `--strict`. The flag is repeatable
(`--allow convert/unsupported-component --allow convert/agent-lossy`); an allowed
rule is still reported as a warning, it just stops aborting the run. Naming a
rule-id that is not blocking (or does not exist) is a harmless no-op. This lets
`convert --strict` serve as a CI gate that fails on *new* lossiness while
tolerating the drops you have already reviewed.

## Why exit 80 exists

Tome reads exactly one plugin manifest: `tome-plugin.toml`, parsed strictly.
There is no fallback — a plugin that ships only a legacy
`.claude-plugin/plugin.json` is not loaded; Tome exits `80` with a message
telling you to run `tome plugin convert`. One format means you always know
whether a plugin has been converted, and there is no lenient partial parse to
debug.

For authors, the practical consequence: run convert against your own checkout,
review the converted copy, and commit it — replacing the legacy layout — so
your users never see exit `80`. Publish the converted tree; see
[Distributing](./distributing.md). For the manifest itself, see
the [authoring overview](./overview.md).

## Run the workflow with your agent

Tome includes a bundled meta skill, `convert-marketplace`, that teaches your
agent this whole workflow: run `convert` and `lint`, review the remaining
warnings, and report back for your confirmation before registering anything:

```bash
tome meta add convert-marketplace
```

See [Meta skills](../using-tome/meta-skills.md).

## Pitfalls

| Exit | When it happens | What to do |
| --- | --- | --- |
| `80` | A plugin ships only a legacy `plugin.json` — Tome won't load it | Run `tome plugin convert` (or ask the author to) |
| `81` | The destination already contains files convert would write | `--force` to overwrite the colliding files, or pick another `--output` |
| `83` | Source format detection failed | Pass `--from <source>` |
| `84` | `--strict` found a feature Tome cannot represent | Drop `--strict` to convert with warnings, or remove the unsupported component |

Full table in the [exit code reference](../reference/exit-codes.md).

## Next steps

- [Linting](./lint.md) — work through the warnings, use the verdict in CI.
- [Distributing](./distributing.md) — publish the converted catalog.
- [Authoring overview](./overview.md) — the structure of the converted layout.

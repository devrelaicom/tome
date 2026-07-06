---
title: Commands
sidebar_position: 1
---

# Commands

Every command, subcommand, and flag. Tome exits `0` on success and a specific
non-zero code for every failure class; see [Exit codes](./exit-codes.md).

## `tome init`

Guided first-run setup wizard. Walks the same flow you would run by hand —
bind the current directory to a workspace (the `tome workspace use --create`
path), multi-select detected harnesses to configure (`tome harness use`), add
a catalog (`tome catalog add`), and enable plugins (`tome plugin enable`,
preceded by an up-front warning naming the active model profile and the real
total download size when models are not yet on disk) — then closes with the
standard `tome status` panel and any remaining steps.

The wizard takes no flags. It is idempotent — a re-run detects existing state
and offers only the outstanding steps; when everything is already configured
it says so and shows status. Every step is skippable (Esc or an explicit skip
moves on, never errors).

Interactive only: without a terminal it exits `54` (`not_a_terminal`) and
prints the equivalent manual commands. `--json` is not supported — script the
individual commands instead.

## `tome catalog`

Manage catalogs — the git repositories of plugins you have registered — plus
the authoring commands for creating new ones.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `add <source>` | `--name` (`-n`), `--ref` (`--branch`, `--tag`) | Register a catalog from an `owner/repo` shorthand, a git URL, or a local path (interpreted as `file://`). The shorthand also accepts a forge prefix: `gh:owner/repo`, `gl:owner/repo`, `bb:owner/repo` for GitHub, GitLab, and Bitbucket. `--name` (short `-n`) overrides the display name; `--ref` tracks a branch, tag, or SHA (default `main`) and has visible aliases `--branch` and `--tag`. The output reports the resolved `commit` — the short SHA in human output, the full 40-char SHA in the `--json` `added.commit` field. |
| `remove <name>` | `--force` | Remove a registered catalog. Refuses while it still has enabled plugins (exit `53`); `--force` cascades the disable. |
| `list` | | List registered catalogs. |
| `update [<name>]` | | Refresh one catalog, or every registered catalog when the name is omitted. |
| `show <name>` | | Show a catalog's manifest and registration metadata. |
| `create <name>` | `--template`, `--output`, `--description`, `--author`, `--dry-run`, `--force` | Scaffold a new catalog from a template. `--description` sets the manifest description; `--author` sets the catalog owner (otherwise a `Your Name` placeholder); `--dry-run` previews the files without writing them. See [Creating](../authoring/create.md). |
| `convert <PATH\|REPO\|URL> [<name>]` | `--from`, `--output`, `--into`, `--force`, `--dry-run`, `--strict`, `--allow`, `--no-fetch` (`--local-only`) | Convert a Claude Code marketplace into a native Tome catalog — a copy; the source is never modified. The source is a single positional (a local path, an `owner/repo` shorthand, or a git URL). The new name is the optional positional `<name>` (default `<source>-tome`). `--no-fetch` (alias `--local-only`) skips fetching the marketplace's remote-source plugins (they are warned-and-skipped) — this flag is unique to `catalog convert`. See [Converting](../authoring/convert.md). |
| `lint <PATH>...` | `--autofix`, `--dry-run`, `--strict` | Validate one or more Tome catalogs and every plugin/skill each nests. Accepts multiple sources (the shell expands a glob such as `catalogs/*`); each is linted independently with never-halt forward-progress and the exit code is the worst verdict across them all. CI-ready exit codes. See [Linting](../authoring/lint.md). |

## `tome plugin`

Manage plugin lifecycle. Bare `tome plugin` opens an interactive
catalog → plugin → action picker (refused on a non-TTY, exit `54`).

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `enable <catalog>/<plugin>...` | `--catalog`, `--yes`, `--sync` | Enable one or more plugins: index their entries and surface them in queries. Accepts multiple ids in one call; the plugin segment (or a bare name with `--catalog`) may contain `*` wildcards, e.g. `enable midnight/compact-*` or `enable midnight/*`. `--catalog <name>` scopes bare or wildcard names to one catalog (a slash-qualified `<catalog>/<plugin>` ignores it). A wildcard that matches nothing is an error, not a silent no-op. In a batch, a bad id is reported and the rest still process (the first failure sets the exit code). `--yes` skips the model-download confirmation (required in CI when models are missing). `--sync` applies the change to your harnesses inline (runs the same propagation as `tome sync` over every bound project); without it, enable prints a reminder to run `tome sync`. |
| `disable <catalog>/<plugin>...` | `--catalog`, `--yes`, `--sync` | Disable one or more plugins; embeddings stay on disk so re-enable is cheap. Accepts multiple ids and `*` wildcards (e.g. `disable midnight/*`) plus `--catalog <name>` for bare or wildcard names, mirroring `enable`. A zero-match wildcard is an error; a batch keeps going past a bad id and surfaces the first failure's exit code. `--yes` skips the confirmation prompt (a batch prompts once, naming every plugin); `--force` is accepted as a hidden back-compat alias. `--sync` applies the change to your harnesses inline (runs the same propagation as `tome sync` over every bound project); without it, disable prints a reminder to run `tome sync`. |
| `list` | `--catalog`, `--enabled-only`, `--filter`, `--tier` | List plugins across every catalog, grouping Skills and Commands with per-entry annotations. `--filter <substr>` keeps only plugins whose name or description contains the (case-insensitive) substring. `--tier <1\|2\|3>` keeps only plugins with at least one enabled entry routed at that tier. All filters compose (logical AND). |
| `show <catalog>/<plugin>` | `--details` | Show one plugin's metadata, component counts, and index status. `--details` annotates each per-entry line with its routing tier; without it the output is unchanged. |
| `create <name>` | `--template`, `--output`, `--into`, `--description`, `--author`, `--dry-run`, `--force` | Scaffold a new plugin from a template. `--into` registers it in an existing catalog's `tome-catalog.toml`. `--description` sets the manifest description; `--author` records the plugin's `[author]`; `--dry-run` previews the files without writing them. See [Creating](../authoring/create.md). |
| `convert <PATH\|REPO\|URL> [<name>]` | `--from`, `--output`, `--into`, `--force`, `--dry-run`, `--strict`, `--allow` | Convert a Claude Code plugin (or a Codex project) into a native Tome plugin. The source is a single positional (a local path, an `owner/repo` shorthand, or a git URL). The new name is the optional positional `<name>` (default `<source>-tome`). `--no-fetch` is not accepted here — it is `catalog convert` only (a single plugin has no remote-plugin fan-out). See [Converting](../authoring/convert.md). |
| `lint <PATH>...` | `--autofix`, `--dry-run`, `--strict` | Validate one or more Tome plugins and every skill each nests. Accepts multiple sources (the shell expands a glob such as `plugins/*`); each is linted independently (never-halt) and the exit code is the worst verdict across them all. See [Linting](../authoring/lint.md). |

## `tome skill`

Author, convert, and validate standalone skills.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `create <name>` | `--template`, `--bare`, `--plugin-name`, `--output`, `--into`, `--description`, `--author`, `--dry-run`, `--force` | Scaffold a new skill. Wraps it in a minimal plugin by default; `--bare` emits only a `<name>/SKILL.md`; `--plugin-name` names the wrapping plugin; `--into` drops the skill into an existing plugin's `skills/`. `--description` sets the skill description; `--author` records the wrapping plugin's `[author]`; `--dry-run` previews the files without writing them. See [Creating](../authoring/create.md). |
| `convert <PATH\|REPO\|URL> [<name>]` | `--from`, `--output`, `--into`, `--force`, `--dry-run`, `--strict`, `--allow` | Convert a foreign skill — a native `SKILL.md` from Claude Code, Cursor, OpenCode, Cline, or a generic Agent Skill. The source is a single positional (a local path, an `owner/repo` shorthand, or a git URL). The new name is the optional positional `<name>` (default `<source>-tome`). `--no-fetch` is not accepted here — it is `catalog convert` only. See [Converting](../authoring/convert.md). |
| `lint <PATH>...` | `--autofix`, `--dry-run`, `--strict` | Validate one or more Tome skills: structure correctness plus residual harness-isms. Accepts multiple sources (never-halt; worst-of exit code). See [Linting](../authoring/lint.md). |

Sources: `convert` takes a single positional source — a local path, an
`owner/repo` shorthand, or a git URL (remote sources are shallow-cloned into a
temp dir that is cleaned up on every exit path). `lint` takes **one or more**
sources: pass several paths, or let the shell expand a glob (`plugins/*`) into
multiple arguments — Tome does no globbing of its own. Each `lint` source is
validated independently with never-halt forward-progress (a parse error, level
mismatch, or autofix I/O failure on one source is reported for that source and
does not abort the rest), and the command exit code is the worst verdict across
all of them: any source with errors → `85`, else `--strict` with any warnings →
`86`, else `0`. Single-source `lint` output is unchanged — human output and the
single `{ findings, summary }` `--json` object are byte-identical to before;
multiple sources emit JSONL, one `{ source, findings, summary }` record per
source.

Flags shared by the three `create`/`convert`/`lint` families: `--output` names
the parent directory the artifact lands under and `--into` injects it into an
existing Tome artifact (the two are mutually exclusive); `--force` overwrites
colliding files — only those files, never a directory wipe (without it, a
collision exits `81`); `--from` overrides source-format detection with a closed
set (`claude-code | codex | cursor | opencode | cline | agent-skills`, plus the
`claude` and `agent` aliases) validated at parse time; the new name is the
optional positional `<name>` on `convert` (default `<source>-tome`), not a flag;
`--dry-run` prints the plan and writes nothing (on `lint` it requires
`--autofix`, since it only qualifies that pass — a bare `lint --dry-run` is a
usage error, exit `2`); `--strict` aborts on anything Tome cannot represent
(exit `84` for `convert`) or promotes lint warnings to failure (exit `86`);
`--allow <rule-id>` (repeatable, `convert` only) demotes a named rule out of the
`--strict` blocking set so an intentional drop (e.g.
`--allow convert/unsupported-component` for a plugin that ships a `themes/`
directory) no longer aborts — the finding is still reported as a warning, and a
strict abort names the count and the distinct blocking rule-ids so you know
exactly what to allow; `--no-fetch` (alias `--local-only`) is `catalog convert`
only and skips fetching the marketplace's remote-source plugins; `--autofix`
applies mechanically-safe lint fixes.

## `tome query`

Semantic search across enabled skills and commands (KNN + reranker).

The query text is given as one or more positional words, joined with a single
space, so `tome query reset a counter` works without quoting. Alternatively pass
a single quoted string with `-q`/`--query` (`tome query -q "reset a counter"`)
when the query contains flag-like or shell-significant tokens. The two forms are
mutually exclusive; giving neither is a usage error.

| Flag | Meaning |
| --- | --- |
| `-q`, `--query <text>` | The query as a single quoted string, instead of positional words. |
| `--top-k <n>` | Cap on returned results (post-rerank when reranking). Default `10`. |
| `--catalog <name>` | Restrict the search to a catalog. Repeatable: pass `--catalog` several times to include entries from any of the named catalogs. |
| `--plugin <name>` | Restrict the search to a plugin (across all catalogs unless `--catalog` is also set). Repeatable: include entries from any of the named plugins. |
| `--kind <kind>` | Restrict the search to an entry kind (`skill`, `command`, or `agent`). Repeatable. Note that `query` only searches indexed, searchable entries, so `--kind agent` typically returns nothing. |
| `--no-rerank` | Skip the reranker stage; scores are raw cosine similarity. |
| `--strict` | Apply the score threshold and exit `40` on an empty result. |
| `--min-score <s>` | Minimum score to retain a result (only enforced with `--strict`). Default `0.0` with the reranker on, `0.5` with `--no-rerank`. |

Example: `tome query reset a counter --kind skill --plugin a --plugin b`.

See [Search](../using-tome/search.md).

## `tome models`

Manage the local embedding and rerank models, against a pinned registry.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `download` | `--force`, `--all`, `--profile <tier>` | Download the active profile's missing models. `--force` re-downloads even when the on-disk manifest records a complete install. `--all` fetches every registered model (every tier). `--profile <small\|medium\|large>` fetches a SPECIFIC tier's models WITHOUT changing the stored active profile — pre-fetch another tier's weights before switching to it (mutually exclusive with `--all`). |
| `list` | `--verify` | List every model with its on-disk state. `--verify` rehashes installed files against their pinned SHA-256 (slower, catches silent corruption). |
| `remove [<name>...]` | `--all`, `--yes` | Remove installed model directories and their manifests. Name one or more models, or pass `--all` to evict every installed model. `--yes` skips the confirmation prompt — asked once for the whole set (required on a non-TTY); `--force` is accepted as a hidden back-compat alias. A failure on one model still processes the rest, then surfaces the first error's exit code. |
| `profile [<tier>]` | — | Show or set the active model profile (`small\|medium\|large`). Omit `<tier>` to show the current profile; pass one to switch. The profile selects which embedder + reranker Tome uses; changing the embedder prints a `tome reindex` notice (never auto-reindexes). |
| `test <capability>` | `--verify` | Run ONE real round-trip against the active model for `summariser`, `embedding`, or `reranker` (the configured remote provider, else the bundled local model) and report success. `--verify` additionally rehashes the active bundled model's on-disk primary artefact against its pinned SHA-256 (the same check `status`/`doctor`/`list` perform); a no-op for a remote provider (no on-disk artefact). Read-only — writes no stored state. |
| `update` | `--include-registry` | Bring local model assets up to date: ensure the active profile's models are present, re-downloading any missing. `--include-registry` also refreshes the harness model-id registry override (`~/.tome/cache/model-registry.json`) from models.dev. |

## `tome reindex`

Force re-embedding outside the `tome catalog update` schedule — for embedder
upgrades or integrity recovery.

```bash
tome reindex                              # everything (whole-index)
tome reindex <catalog>                    # one catalog
tome reindex <catalog>/<plugin>           # one plugin
tome reindex <catalog>/compact-*          # every matching plugin in a catalog
tome reindex mid-* other                  # multiple scopes, unioned + deduped
tome reindex --catalog midnight           # a whole catalog (named flag form)
tome reindex --catalog 'mid-*'            # every enrolled catalog matching the glob
tome reindex --plugin midnight/compact-*  # a plugin glob (named flag form)
tome reindex --catalog a --plugin b/c     # combine --catalog + --plugin (union)
```

Scopes are variadic. Each positional token is a `<catalog>` (whole catalog), a
`<catalog>/<plugin>` (one plugin), or a `*` glob (`<catalog>/*`,
`<catalog>/compact-*`, or a bare `mid-*` matching enrolled catalog **names**).
Multiple tokens are unioned and deduplicated.

- `--catalog <name>` (repeatable) reindexes every enabled plugin in the named
  catalog(s); a `*` glob matches enrolled catalog names.
- `--plugin <catalog>/<plugin>` (repeatable) reindexes the named plugin(s); a
  `*` glob is allowed in the plugin segment.
- `--catalog` and `--plugin` may be combined (their targets union), but neither
  can be mixed with positional `<scope>` tokens.
- A glob that matches nothing is a usage error (never a silent no-op).

Only the **whole-index** form (no scopes and no `--catalog`/`--plugin`) restamps
the global embedder identity; any explicit selection is refused under embedder
drift (run a bare `tome reindex` to switch embedders).

`--force` re-embeds every in-scope skill regardless of its content hash.

## `tome status`

Read-only pre-flight check across models, index, and drift. **Never takes the
index lock.** Three distinct health verdicts drive the exit code: `0` when
**healthy**, `10` when **degraded** (a non-fatal issue such as a missing
reranker or summariser — queries still serve), and `1` when **unhealthy** (a broken index,
embedder drift, or a malformed config). Both non-zero codes fail a plain "fail
on any non-zero" CI gate; the distinct `10` lets a "fail on unhealthy only" gate
branch (or read `--json`'s `.overall` field — `"ok"` / `"degraded"` /
`"unhealthy"` — the documented gating source). `--verify` rehashes each
installed model against its pinned SHA-256.

`tome status [<workspace>]` accepts an optional positional workspace name: it
reports on that named workspace instead of the resolved scope (defaulting to the
resolved scope when omitted), mirroring `tome workspace info [<name>]`. The name
must already exist in the central registry — a missing name exits `13`
(`workspace_not_found`). Targeting a name resolves it as a flag-style scope
(`project`, `project_root = None`), so the project-relative harness/MCP rows are
inactive, exactly as with `--workspace <name>`. See
[Exit codes](./exit-codes.md#health-verdicts-status--doctor) and
[Troubleshooting](../using-tome/troubleshooting.md).

## `tome doctor`

Comprehensive diagnostic across every subsystem (workspace, models, index,
drift, catalog caches, harnesses, meta skills). **Read-only by default.**

Human output leads with a one-line verdict (`unhealthy — 1 failing, 2 warnings,
22 ok`), renders failing sections first, then warnings, and collapses the
all-ok subsystems into a single line; the global `--verbose` flag restores the
full section listing. `--json` output is unaffected by this structure.

Exit codes match `tome status`: `0` healthy, `10` degraded, `1` unhealthy — read
`--json`'s `.overall` field to gate in scripts. When `--fix` runs but un-fixable
issues remain, `doctor` exits `75` (`doctor_fix_unsafe`) instead of the health
code, signalling "the repair did something, but manual work remains".

| Flag | Meaning |
| --- | --- |
| `--fix` | Apply the safe automatic repairs: re-download missing/corrupt models, re-clone broken catalog caches, forward-migrate the index schema, re-copy project rules, re-run the sync for drifted harnesses. Destructive repairs are never automatic. |
| `--force` | Override the safe-by-default repair gates (currently: rewrite developer-authored harness MCP `tome` entries on `--fix`). |
| `--verify` | Rehash installed models against their pinned SHA-256, and probe each effective harness's registered `tome mcp` server end-to-end (a real `initialize` + `tools/list` round-trip over stdio, bounded by a timeout; failures report the reason and a stderr tail). Network-free but slower. Also enabled by `[doctor] verify_by_default` in `~/.tome/config.toml`. |
| `--dry-run` | With `--fix`: list the automatic repairs `--fix` would apply, apply nothing, and exit with the read-only health code. An error (exit `2`) without `--fix`. |

## `tome workspace`

Per-project scopes and composition.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `use [<name>]` | `--create`, `--force` | Bind the current project directory to a workspace (writes `<cwd>/.tome/config.toml`). `--create` creates the workspace first (create-if-absent), so `init` + `use` happen in one step; it requires an explicit `<name>`. Omit `<name>` on a terminal to pick from a list of existing workspaces; on a non-terminal an omitted name is a usage error (exit `54`). `--force` bypasses the refusal when CWD is your home directory or the filesystem root. |
| `init <name>` | `--bind`, `--inherit-global` | Create a workspace. `--bind` also binds the current directory to the new workspace (the mirror of `use --create`), running a sync in the same step. `--inherit-global` seeds its catalogs from the global workspace's enrolments at creation time. |
| `list` | `--absolute` | List workspaces with catalog, plugin, skill, and bound-project counts. The workspace resolved for the current directory is marked with `*` in the `Cur` column. `Last used` renders as a relative time (e.g. `2 days ago`) by default; `--absolute` forces the RFC 3339 timestamp. `--json` adds a per-row `current` boolean and always emits the absolute `last_used_at` (the relative rendering is human-only). |
| `current` | | Print the workspace bound to the current directory on one line, with no decoration — for shell prompts / scripting (`$(tome workspace current 2>/dev/null)`). Read-only. Exits `12` (`workspace_not_bound`) with a clear stderr message and no stdout when nothing is bound. |
| `info [<name>]` | | Report a workspace's details. Read-only; never acquires the advisory lock. Defaults to the resolved workspace. |
| `rename <old> <new>` | | Rename a workspace, updating every bound project's marker atomically. Refuses either side of `global`. |
| `regen-summary [<name>]` | | Force regeneration of a workspace's cached summaries and rules file. `<name>` defaults to the resolved workspace, but only after an interactive confirmation; on a non-terminal the name is required (exit `54` when there is no TTY, exit `2` under `--non-interactive` / `TOME_NONINTERACTIVE`), so the resolved (often `global`) scope is never regenerated silently. |
| `remove <name>` | `--force` | Remove a workspace and its DB rows. Refuses the reserved `global` (exit `15`) and refuses without `--force` while projects are bound (exit `16`). |

Workspace state is propagated to bound projects by the top-level [`tome sync`](#tome-sync) (the former `tome workspace sync`).

See [Workspaces](../using-tome/workspaces.md).

## `tome harness`

Configure target coding agents (Claude Code, Codex, Cursor, Gemini CLI,
OpenCode). Bare `tome harness` enumerates every supported harness.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `list [<workspace>]` | | List the effective harness list for the resolved project, or a named workspace's directly-declared list. |
| `use [<name>...]` | `--all`, `--include-opt-in`, `--scope`, `--force` | Configure one or more harnesses in the chosen scope and run the sync. With **names**, exactly those (aliases and the opt-in targets `generic`/`generic-op` resolve by name). With **no names and no `--all`**, every auto-detected harness. With **`--all`**, every auto-detectable harness — but NOT the opt-in `generic`/`generic-op` targets; when it skips them it prints a one-line `note:` on stderr naming them (human output only, suppressed under `--json`). Add **`--include-opt-in`** (requires `--all`) to ALSO configure those opt-in targets. `--scope` is `project` (default), `workspace`, or `global`. `--force` overrides a harness-clash on the MCP config write (otherwise exit `19`). |
| `remove [<name>...]` | `--all`, `--scope` | Remove one or more harnesses from the chosen scope and run the cleanup pass. Name harnesses, or pass `--all` to clear every harness configured in the resolved scope. (Unlike `use`, an empty selection with no `--all` is a usage error — there is no "all detected" default for a destructive op.) A per-harness failure still processes the rest, then surfaces the first error. |
| `info [<name>]` | | Per-harness details for the current project: detection, targets, integration state, source-of-scope. With a **name**, reports that one harness. With **no name**, reports one section per harness in the effective list (the same set `harness list` reports), like `workspace info [<name>]`; `--json` returns an array. When nothing is configured for the scope it prints a short hint (exit `0`, not an error). An unknown explicit name exits `18`. |
| `preview <name>` | `--plugin` | Preview what a sync would deliver vs drop for one harness, per enabled entry: agents native/persona/unrepresented (with dropped model/tools), skills/commands MCP-routing, and hooks native vs `GUARDRAILS.md` fallback. `--plugin` scopes the preview to one enabled plugin. Read-only. |
| `session-start` | `--harness` | Reconcile the project, then print the workspace's skill-routing directive to stdout, generated fresh from live state. Intended as a `SessionStart` hook target; not usually run by hand. `--harness <name>` selects the host harness whose stdout envelope wraps the directive (absent → the raw directive). |
| `run-hook` | `--event`, `--explain`, `--harness` | Translate a plugin hook event from the target harness's native format, run the enabled plugins' matching hooks, and emit the harness's wire decision. A hook-dispatch target; not run by hand — fails open. `--event <name>` is the CC event (`PreToolUse`, `PostToolUse`, …); `--harness <name>` is the host harness; `--explain` is a dry-run that reports what would fire without running anything. |

`use` and `remove` take their harness selection as a bare **positional** (`<name>...`) — the deliberate exception. Every other multi-select of harnesses (`tome sync`, `tome meta add`/`remove`) uses the repeatable **`--harness`** flag, because there the harness is a filter on some other subject. `use`/`remove` act *on* the harnesses, so the positional reads naturally.

See [Harnesses](../using-tome/harnesses.md).

## `tome sync`

Propagate workspace state to bound projects: write each project's
`.tome/RULES.md` and reconcile its harness files (rules sink, MCP config, hooks,
agents). Byte-for-byte idempotent — re-running changes nothing. This is the same
propagation `tome plugin enable/disable --sync` runs inline.

| Flags | Purpose |
| --- | --- |
| `--all` | Sync **every** project bound to the resolved workspace, not just the current one. |
| `--rules-only` | Only write `.tome/RULES.md`; skip the harness reconcile. Mutually exclusive with `--harness-only`. |
| `--harness-only` | Only reconcile harness files; skip the `.tome/RULES.md` write. |
| `--harness <name>` | Restrict the harness reconcile to one or more harnesses (repeatable: `--harness a --harness b`). Aliases resolve to their canonical module; unknown names exit `18`. Ignored with `--rules-only`. Empty (the default) reconciles the full effective set. |
| `--dry-run` | Preview: compute and print exactly what this sync would change — the same per-harness classification a real run performs — write nothing, and exit `0`. Combines with every other flag. |

**Where it acts.** `tome sync` targets projects, not the whole workspace at
once. Its behaviour depends on where you run it:

- **Inside a bound project** (a `.tome/config.toml` marker resolves): syncs
  **that project**.
- **`--all`** (from anywhere): fans out to **every project bound to the
  resolved workspace**. `--all` is scoped to the *active* workspace's bindings —
  it does not reach projects bound to other workspaces.
- **Outside any project, without `--all`**: rather than erroring, `tome sync`
  falls back to the `--all` fan-out over the resolved workspace's bound
  projects, printing a short note to stderr so it's clear it acted outside the
  current directory (`--json` output is identical to `--all`). If the workspace
  has **no** bound projects, it exits `2` (`usage`) with a message naming the
  next step — run `tome workspace use` inside a project to bind it, or
  `tome sync --all` once you have bindings.

The fan-out reuses the exact `--all` writer path, so every project it touches
inherits the same safety: managed edits stay inside Tome's markers, symlinked
sinks are refused, and writes are atomic. A per-project failure does not abort
the run — every reachable project is attempted, each failure is reported (one
`FAILED` line per project in human mode; a trailing `failures` array in
`--json`, present only when something failed), and the first error still sets
the exit code while partial progress lands.

**Output.** When a project's files changed, human mode enumerates the
file-level actions per harness (`+` added, `~` updated, `-` removed; paths
relative to the project where possible) instead of a bare change count; an
unchanged project keeps the one-line summary. `--dry-run` prints the same
enumeration under a leading banner without touching a single file — every sink
computes its real classification (reads, merges, byte-compares) and skips only
the final write, so the preview is exactly what the next real run will do.

## `tome meta`

Install Tome's bundled meta skills — native `SKILL.md` guides that teach an
agent how to use Tome itself — into your detected harnesses.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `list` | | List the bundled meta skills and their per-harness install status. |
| `add [<skill_id>...]` | `--all`, `--harness`, `--global`, `--force` | Install bundled meta skills. Name one or more skill ids, or pass `--all` to install every bundled skill. Default: project scope, every detected harness that consumes native skills. `--harness <name>` (repeatable) targets specific harnesses; `--global` installs into the user-level skills dir; `--force` re-writes even when the on-disk copy is current. A per-location failure still processes the rest, then surfaces the first error. |
| `remove [<skill_id>...]` | `--all`, `--harness`, `--global` | Remove installed meta skills from the selected harnesses. Name one or more skill ids, or pass `--all` to remove every bundled skill (a not-present location is a no-op). |

See [Meta skills](../using-tome/meta-skills.md).

## `tome tier`

Manage the per-workspace **routing tier** of enabled skills and commands. Tiers
drive what instructions Tome injects so an agent knows when to fetch a skill
(Tier 1/2 via `get_skill`) or search for it (Tier 3, the default). Every command
operates on the resolved workspace (use `--workspace` / `-w` to target another).

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `set <plugin>/<name> <1\|2\|3>` | `--plugin`, `--catalog`, `--kind` | Set the routing tier of one entry, a name-glob, or whole plugins. |
| `list` | | List every enabled skill/command grouped by routing tier. |
| `clear <plugin>/<name>` | `--plugin`, `--all`, `--catalog`, `--kind` | Reset the tier of one entry, a name-glob, whole plugins, or the entire workspace back to the default (3). |

```bash
tome tier set plugin-alpha/skill-a 1        # one entry
tome tier set "plugin-alpha/*" 2            # every entry of a plugin (name glob)
tome tier set "plugin-alpha/foo-*" 2        # a name-glob subset
tome tier set --plugin midnight/compact 1   # whole plugin (catalog/plugin selector)
tome tier set --plugin compact --catalog midnight 1  # bare selector + --catalog
tome tier clear plugin-alpha/skill-a        # reset one entry to tier 3
tome tier clear --plugin midnight/compact   # reset a whole plugin
tome tier clear --all                       # reset every enabled entry in the workspace
```

- The positional `<plugin>/<name>` id is **entry-level**; its name segment may be
  a `*` glob (`<plugin>/*`, `<plugin>/foo-*`). A glob that matches nothing is a
  usage error (`entry_not_found`), never a silent no-op.
- `--plugin <catalog>/<plugin>` (repeatable) is **plugin-level** — it retiers
  every enabled tierable entry of the named plugin(s). A bare `<plugin>` is
  disambiguated by `--catalog`; a `*` glob in either segment fans out. A
  `--plugin` naming a plugin with no tierable entries is `entry_not_found`.
- `--all` (clear only) resets **every** enabled tierable entry in the workspace.
- Exactly one selection source is required: the positional id **xor** `--plugin`
  (`set`), plus `--all` (`clear`). Passing none or more than one is a usage error.
- `--catalog` / `--kind` disambiguate when the same plugin name spans catalogs or
  the same entry name exists as both a skill and a command. Agents carry no tier.

Tiers persist on `workspace_skills.tier`; `set`/`clear` run one UPDATE batch under
the advisory index lock and regenerate the workspace `RULES.md` once afterwards.

## `tome telemetry`

Inspect and control anonymous usage telemetry. Telemetry is opt-out, auto-disabled
under CI, and never blocks the foreground: commands only append to a local queue,
which a detached background flusher drains best-effort.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `status` | | Report telemetry state: enabled + why, install UUID (if any), the delivery endpoint, queued-event count, and last-flush stamp. Read-only — never mints an install id. |
| `inspect` | | Pretty-print the pending event queue WITHOUT sending it. Read-only — the queue file is byte-identical after. Reports any corrupt/unparsable lines; exits `92` if any exist. |
| `on` | | Enable telemetry (sets the opt-out switch on) and ensure an install identity exists. |
| `off` | | Disable telemetry. The install UUID is left intact; a later `on` resumes it. Use `purge` to also delete the identity. |
| `reset` | `--yes` | Sever telemetry continuity: mint a fresh install UUID and clear the queue. Prompts for confirmation unless `--yes`. |
| `purge` | | Delete all telemetry state (install UUID + queue) and switch telemetry off until explicitly re-enabled. |
| `flush` | `--quiet` | Drain the pending event queue to the collector in the FOREGROUND and report the outcome. The drain is best-effort: an unreachable endpoint does not fail the command, so `flush` always exits `0` and deliverability is never surfaced as an exit code. The detached background flusher invokes this with `--quiet` (no output). |

## `tome config`

Inspect and validate the unified global config, `~/.tome/config.toml`. Both
subcommands are **read-only**: they never write the file, create directories, or
take the index lock.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `show` | | Print each curated scalar config knob with its **effective** value and a provenance annotation: `(default)`, `(config)`, or `(env)`. `--json` emits a stable object of `key → { "value", "source" }`. |
| `validate` | | Run the strict config parse. Prints `config is valid` and exits `0` on a good (or absent) config; on a malformed config, prints the legible key-naming error to stderr and exits `5` (`manifest_invalid`). `--json` emits `{ "valid", "error" }` on stdout. |

The knobs shown are the curated scalar toggles: `query.top_k`, `query.rerank`,
`query.strict_min_score`, `summariser.enabled`, `summariser.long_max_chars`,
`logging.level`, `output.color`, `output.progress`, `workspace.default`,
`mcp.description_max_chars`, `models.profile`, `doctor.verify_by_default`,
`harness.default_scope`, `hooks.translate_plugin_hooks`, `telemetry.enabled`,
and `telemetry.endpoint`. Each shown default is read from the same source
constant the consumer uses, so it can't drift from the effective value.

Provenance is resolved per knob, highest precedence first:

- `(env)` — the knob genuinely has an environment override and that variable is
  set. Only these knobs can be `(env)`: `logging.level` (`TOME_LOG` /
  `RUST_LOG`), `output.color` (`NO_COLOR`), `workspace.default`
  (`TOME_WORKSPACE`), `telemetry.enabled` (`TOME_TELEMETRY`, plus the CI
  auto-disable), and `telemetry.endpoint` (`TOME_GAUGE_ENDPOINT`). A knob with
  no env override is never annotated `(env)`.
- `(config)` — the key is present in `~/.tome/config.toml` (detected from the raw
  document, so a key set to its default value still reads as `(config)`).
- `(default)` — none of the above; the built-in default applies.

`show` surfaces the curated scalar knobs only. Non-scalar or credential-bearing
config is intentionally omitted: the BYOK/BYOM provider registry (`[providers]`)
and the capability `provider`/`model` reference fields (a provider entry can
carry an inline `api_key`, which must never be echoed through a user-facing
surface), and the list-valued `[harness]` composition settings. Setting values
from the CLI (`config set`) is a planned fast-follow.

## `tome exit-codes`

Print the exit-code reference table: every code with its `--json` error
`category` slug and a one-line meaning.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `exit-codes [<code>]` | | Print the full code → category → meaning table, or one code's row (`tome exit-codes 50`). An unknown code is a usage error (exit `2`) pointing at the full table. |

The data is the same static table that backs the [Exit codes](./exit-codes.md)
page — a test pins the two against each other, so the command and the docs
cannot drift. It is a pure static lookup over the closed error set: it needs no
configured HOME, index, config, or lock, and honours the global `--json` flag
(`category` is `null` for the success row, matching the CLI contract's
`exitCodes` shape).

## `tome completions`

Generate a shell completion script and print it to stdout.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `completions <shell>` | | Print a completion script for `<shell>` to stdout, where `<shell>` is one of `bash`, `zsh`, `fish`, `powershell`, or `elvish`. An unknown shell is a usage error (exit `2`) that lists the valid values. |

Generating completions is a pure static operation over the command tree, so it
needs no configured HOME, index, or config — you can run it during shell setup
before Tome is otherwise configured. Redirect the output to the file your shell
loads completions from:

- **zsh** — `tome completions zsh > ~/.zfunc/_tome` (ensure `~/.zfunc` is on your
  `fpath` and `autoload -U compinit && compinit` runs in `~/.zshrc`).
- **bash** — `tome completions bash > /usr/local/etc/bash_completion.d/tome`
  (or source it from `~/.bashrc`: `source <(tome completions bash)`).
- **fish** — `tome completions fish > ~/.config/fish/completions/tome.fish`.
- **powershell** — `tome completions powershell | Out-String | Invoke-Expression`
  (or append it to your `$PROFILE`).
- **elvish** — `tome completions elvish > ~/.config/elvish/lib/tome.elv` and
  `use tome` it from your `rc.elv`.

## `tome mcp`

Run Tome as a stdio MCP server backed by the resolved workspace's index.
Exposes the `search_skills`, `get_skill`, and `get_skill_info` tools, the
built-in `meta` tool, plus user-invocable entries as MCP prompts.

`--harness <name>` tells the server which harness is hosting it
(`claude-code`, `cursor`, `codex`, `opencode`) so the built-in `meta` tool can
install skills into the right place. You rarely write it yourself — `tome
sync` stamps it into the spawned server's arguments. See the
[MCP server](../using-tome/mcp-server.md).

The server writes a rotating JSON-lines log to `~/.tome/logs/mcp.log`
(10 MiB cap, one `mcp.log.1` backup). The `TOME_MCP_LOG` environment
variable overrides the file sink — useful in ephemeral CI containers where
the log is wasted IO and a surprise artifact. It is distinct from `TOME_LOG`
/ `RUST_LOG`, which tune verbosity; `TOME_MCP_LOG` controls the file
destination only. stdout stays protocol-only and stderr stays error-only in
every case.

- unset → default path (`~/.tome/logs/mcp.log`, 10 MiB rotation).
- `TOME_MCP_LOG=off` (case-insensitive) or an empty value → no file log is
  opened; nothing is created on disk.
- `TOME_MCP_LOG=<path>` → write the rotating log to `<path>` instead, creating
  parent directories and rotating `<path>.1` with the same 10 MiB cap.

If an override path cannot be opened (bad path, no permission, or a
directory), the server prints one warning to stderr and continues with no
file log rather than failing to start.

## Global behaviour

- `--json` is available on the read-only inspection commands and emits
  machine-readable JSON on stdout, orthogonal to logging (which goes to
  stderr). `tome mcp` intentionally ignores it — the protocol *is* the
  structured output. The environment variable `TOME_JSON` (any truthy value —
  set, non-empty, and not `0`/`false`/`no`/`off`) forces JSON when the flag is
  absent; the `--json` flag always wins.
- `--no-color` disables ANSI colour in human output. A truthy `TOME_NO_COLOR`
  (same truthy rule as above) does the same — a Tome-specific override layered
  on top of the standard `NO_COLOR` signal and the `[output] color` config knob,
  so you can force Tome's colour off without disabling colour in every other
  `NO_COLOR`-respecting tool. Precedence (highest first): `--no-color` /
  `TOME_NO_COLOR` → `NO_COLOR` → `[output] color` → auto (TTY).
- `--workspace <name>` (short `-w`) runs the command against a named workspace.
  When omitted, the resolver consults the `TOME_WORKSPACE` environment variable
  (an empty value is ignored) and the project-marker walk before falling back to
  the privileged `global` workspace. `TOME_WORKSPACE` is resolved by the scope
  resolver (not as a clap `env=`), so it keeps its distinct `env` provenance in
  `doctor` / `workspace info` / `list` / `current` diagnostics and the
  empty-value-ignored behaviour.
- `--non-interactive` auto-confirms every prompt-bearing command
  (`catalog remove`, `plugin enable`/`disable`, `models remove`,
  `telemetry reset`), equivalent to passing that command's `--force` / `--yes`.
  The environment variable `TOME_NONINTERACTIVE=1` (any truthy value — set,
  non-empty, and not `0`/`false`/`no`/`off`) does the same. Either lets a
  scripted caller drive Tome without knowing each command's skip flag. A
  persistently-exported `TOME_NONINTERACTIVE=1` also auto-confirms the prompts
  inside the otherwise-interactive `tome plugin` TUI — intended, since the env
  var auto-confirms *every* prompt. It does **not** bypass non-prompt safety
  refusals such as `catalog remove`'s enabled-plugin cascade guard (exit `53`)
  or `workspace remove`'s bound-project guard (exit `16`), which still require
  the per-command `--force`. For consistency every prompt-bearing command also
  accepts both `--force` and `--yes` (the non-canonical spelling is a hidden
  alias).
- `-v` / `--verbose` raises log verbosity to info; `-vv` to debug
  (env: `TOME_LOG`).
- On `SIGINT` (Ctrl-C), Tome exits with code `8`.
- Every failure class has its own [exit code](./exit-codes.md).

---
title: Commands
sidebar_position: 1
---

# Commands

Every command, subcommand, and flag. Tome exits `0` on success and a specific
non-zero code for every failure class; see [Exit codes](./exit-codes.md).

## `tome catalog`

Manage catalogs — the git repositories of plugins you have registered — plus
the authoring commands for creating new ones.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `add <source>` | `--name`, `--ref` | Register a catalog from an `owner/repo` shorthand, a git URL, or a local path (interpreted as `file://`). `--name` overrides the display name; `--ref` tracks a branch, tag, or SHA (default `main`). |
| `remove <name>` | `--force` | Remove a registered catalog. Refuses while it still has enabled plugins (exit `53`); `--force` cascades the disable. |
| `list` | | List registered catalogs. |
| `update [<name>]` | `--force` | Refresh one catalog, or every registered catalog when the name is omitted. (`--force` is accepted but currently a no-op.) |
| `show <name>` | | Show a catalog's manifest and registration metadata. |
| `create <name>` | `--template`, `--output`, `--force` | Scaffold a new catalog from a template. See [Creating](../authoring/create.md). |
| `convert <source> [<name>]` | `--name`, `--from`, `--output`, `--into`, `--force`, `--dry-run`, `--strict`, `--no-fetch` | Convert a Claude Code marketplace into a native Tome catalog — a copy; the source is never modified. `--no-fetch` skips fetching the marketplace's remote-source plugins (they are warned-and-skipped). See [Converting](../authoring/convert.md). |
| `lint <path>` | `--autofix`, `--dry-run`, `--strict` | Validate a Tome catalog and every plugin/skill it nests. CI-ready exit codes. See [Linting](../authoring/lint.md). |

## `tome plugin`

Manage plugin lifecycle. Bare `tome plugin` opens an interactive
catalog → plugin → action picker (refused on a non-TTY, exit `54`).

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `enable <catalog>/<plugin>` | `--yes`, `--sync` | Enable a plugin: index its entries and surface them in queries. `--yes` skips the model-download confirmation (required in CI when models are missing). `--sync` applies the change to your harnesses inline (runs the same propagation as `tome sync` over every bound project); without it, enable prints a reminder to run `tome sync`. |
| `disable <catalog>/<plugin>` | `--force`, `--sync` | Disable a plugin; embeddings stay on disk so re-enable is cheap. `--force` skips the confirmation prompt. `--sync` applies the change to your harnesses inline (runs the same propagation as `tome sync` over every bound project); without it, disable prints a reminder to run `tome sync`. |
| `list` | `--catalog`, `--enabled-only` | List plugins across every catalog, grouping Skills and Commands with per-entry annotations. |
| `show <catalog>/<plugin>` | | Show one plugin's metadata, component counts, and index status. |
| `create <name>` | `--template`, `--output`, `--into`, `--force` | Scaffold a new plugin from a template. `--into` registers it in an existing catalog's `tome-catalog.toml`. See [Creating](../authoring/create.md). |
| `convert <source> [<name>]` | `--name`, `--from`, `--output`, `--into`, `--force`, `--dry-run`, `--strict`, `--no-fetch` | Convert a Claude Code plugin (or a Codex project) into a native Tome plugin. (`--no-fetch` is accepted but only meaningful for `catalog convert`.) See [Converting](../authoring/convert.md). |
| `lint <path>` | `--autofix`, `--dry-run`, `--strict` | Validate a Tome plugin and every skill it nests. See [Linting](../authoring/lint.md). |

## `tome skill`

Author, convert, and validate standalone skills.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `create <name>` | `--template`, `--bare`, `--plugin-name`, `--output`, `--into`, `--force` | Scaffold a new skill. Wraps it in a minimal plugin by default; `--bare` emits only a `<name>/SKILL.md`; `--plugin-name` names the wrapping plugin; `--into` drops the skill into an existing plugin's `skills/`. See [Creating](../authoring/create.md). |
| `convert <source> [<name>]` | `--name`, `--from`, `--output`, `--into`, `--force`, `--dry-run`, `--strict`, `--no-fetch` | Convert a foreign skill — a native `SKILL.md` from Claude Code, Cursor, OpenCode, Cline, or a generic Agent Skill. (`--no-fetch` is accepted but only meaningful for `catalog convert`.) See [Converting](../authoring/convert.md). |
| `lint <path>` | `--autofix`, `--dry-run`, `--strict` | Validate a Tome skill: structure correctness plus residual harness-isms. See [Linting](../authoring/lint.md). |

Flags shared by the three `create`/`convert`/`lint` families: `--output` names
the parent directory the artifact lands under and `--into` injects it into an
existing Tome artifact (the two are mutually exclusive); `--force` overwrites
colliding files — only those files, never a directory wipe (without it, a
collision exits `81`); `--from` overrides source-format detection
(`claude-code | codex | cursor | opencode | cline | agent-skills`); `--dry-run`
prints the plan and writes nothing; `--strict` aborts on anything Tome cannot
represent (exit `84` for `convert`) or promotes lint warnings to failure (exit
`86`); `--autofix` applies mechanically-safe lint fixes.

## `tome query`

Semantic search across enabled skills and commands (KNN + reranker).

| Flag | Meaning |
| --- | --- |
| `--top-k <n>` | Cap on returned results (post-rerank when reranking). Default `10`. |
| `--catalog <name>` | Restrict the search to a single catalog. |
| `--plugin <name>` | Restrict the search to a single plugin (across all catalogs unless `--catalog` is also set). |
| `--no-rerank` | Skip the reranker stage; scores are raw cosine similarity. |
| `--strict` | Apply the score threshold and exit `40` on an empty result. |
| `--min-score <s>` | Minimum score to retain a result (only enforced with `--strict`). Default `0.0` with the reranker on, `0.5` with `--no-rerank`. |

See [Search](../using-tome/search.md).

## `tome models`

Manage the local embedding and rerank models, against a pinned registry.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `download` | `--force` | Download every registered model that is missing. `--force` re-downloads even when the on-disk manifest records a complete install. |
| `list` | `--verify` | List every model with its on-disk state. `--verify` rehashes installed files against their pinned SHA-256 (slower, catches silent corruption). |
| `remove <name>` | `--force` | Remove an installed model directory and its manifest. `--force` skips the confirmation prompt (required on a non-TTY). |

## `tome reindex`

Force re-embedding outside the `tome catalog update` schedule — for embedder
upgrades or integrity recovery.

```bash
tome reindex                      # everything
tome reindex <catalog>            # one catalog
tome reindex <catalog>/<plugin>   # one plugin
```

`--force` re-embeds every in-scope skill regardless of its content hash.

## `tome status`

Read-only pre-flight check across models, index, and drift. **Never takes the
index lock.** Exits `0` when healthy, `1` on degraded. `--verify` rehashes each
installed model against its pinned SHA-256. See
[Troubleshooting](../using-tome/troubleshooting.md).

## `tome doctor`

Comprehensive diagnostic across every subsystem (workspace, models, index,
drift, catalog caches, harnesses, meta skills). **Read-only by default.**

| Flag | Meaning |
| --- | --- |
| `--fix` | Apply the safe automatic repairs: re-download missing/corrupt models, re-clone broken catalog caches, forward-migrate the index schema, re-copy project rules, re-run harness sync for drifted harnesses. Destructive repairs are never automatic. |
| `--force` | Override the safe-by-default repair gates (currently: rewrite developer-authored harness MCP `tome` entries on `--fix`). |
| `--verify` | Rehash installed models against their pinned SHA-256. |

## `tome workspace`

Per-project scopes and composition.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `use <name>` | `--force` | Bind the current project directory to a workspace (writes `<cwd>/.tome/config.toml`). `--force` bypasses the refusal when CWD is your home directory or the filesystem root. |
| `init <name>` | `--inherit-global` | Create a workspace. `--inherit-global` seeds its catalogs from the global workspace's enrolments at creation time. |
| `list` | | List workspaces with catalog, plugin, skill, and bound-project counts. |
| `current` | | Print the workspace bound to the current directory on one line, with no decoration — for shell prompts / scripting (`$(tome workspace current 2>/dev/null)`). Read-only. Exits `12` (`workspace_not_bound`) with a clear stderr message and no stdout when nothing is bound. |
| `info [<name>]` | | Report a workspace's details. Read-only; never acquires the advisory lock. Defaults to the resolved workspace. |
| `rename <old> <new>` | | Rename a workspace, updating every bound project's marker atomically. Refuses either side of `global`. |
| `regen-summary <name>` | | Force regeneration of a workspace's cached summaries and rules file. |
| `remove <name>` | `--force` | Remove a workspace and its DB rows. Refuses the reserved `global` (exit `15`) and refuses without `--force` while projects are bound (exit `16`). |
| `sync [<name>]` | | Copy the workspace's central rules file to every bound project. Idempotent; never regenerates summaries. |

See [Workspaces](../using-tome/workspaces.md).

## `tome harness`

Configure target coding agents (Claude Code, Codex, Cursor, Gemini CLI,
OpenCode). Bare `tome harness` enumerates every supported harness.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `list [<workspace>]` | | List the effective harness list for the resolved project, or a named workspace's directly-declared list. |
| `use <name>` | `--scope`, `--force` | Append a harness to the chosen scope's settings and run the sync. `--scope` is `project` (default), `workspace`, or `global`. `--force` overrides a harness-clash on the MCP config write (otherwise exit `19`). |
| `remove <name>` | `--scope` | Remove a harness from the chosen scope and run the cleanup pass. |
| `info <name>` | | Per-harness details for the current project: detection, targets, integration state, source-of-scope. |
| `sync` | | Reconcile the project's filesystem against the effective harness list. Byte-for-byte idempotent. |

See [Harnesses](../using-tome/harnesses.md).

## `tome meta`

Install Tome's bundled meta skills — native `SKILL.md` guides that teach an
agent how to use Tome itself — into your detected harnesses.

| Subcommand | Flags | Purpose |
| --- | --- | --- |
| `list` | | List the bundled meta skills and their per-harness install status. |
| `add <skill_id>` | `--harness`, `--global`, `--force` | Install a bundled meta skill. Default: project scope, every detected harness that consumes native skills. `--harness <name>` (repeatable) targets specific harnesses; `--global` installs into the user-level skills dir; `--force` re-writes even when the on-disk copy is current. |
| `remove <skill_id>` | `--harness`, `--global` | Remove an installed meta skill from the selected harnesses. |

See [Meta skills](../using-tome/meta-skills.md).

## `tome mcp`

Run Tome as a stdio MCP server backed by the resolved workspace's index.
Exposes the `search_skills`, `get_skill`, and `get_skill_info` tools, the
built-in `meta` tool, plus user-invocable entries as MCP prompts.

`--harness <name>` tells the server which harness is hosting it
(`claude-code`, `cursor`, `codex`, `opencode`) so the built-in `meta` tool can
install skills into the right place. You rarely write it yourself — `tome
harness sync` stamps it into the spawned server's arguments. See the
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
  structured output.
- `--workspace <name>` runs the command against a named workspace. When
  omitted, the resolver consults `TOME_WORKSPACE` and the project-marker walk
  before falling back to the privileged `global` workspace.
- `-v` / `--verbose` raises log verbosity to info; `-vv` to debug
  (env: `TOME_LOG`).
- On `SIGINT` (Ctrl-C), Tome exits with code `8`.
- Every failure class has its own [exit code](./exit-codes.md).

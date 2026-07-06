---
title: Workspaces quickstart
sidebar_position: 3
---

# Workspaces quickstart

A **workspace** is a per-project scope: each one enables its own catalogs and
plugins, so the set active for a contracts project never leaks into a web app's
context. This page builds two workspaces, binds each to a project directory, and
shows how the MCP server serves the right one per project with no manual switch.
It reuses the [Quickstart](./quickstart.md)'s catalog,
`devrelaicom/midnight-expert-tome`, so if you've run that page you already have
the catalog added.

If you haven't installed Tome yet, start with [Install](./install.md).

## 1. Create a workspace

`tome workspace use --create <name>` creates the workspace and binds the current
directory to it in one step. Run it from inside your contracts project:

```bash
cd ~/code/contracts
tome workspace use --create contracts
```

The bind writes a project marker at `<cwd>/.tome/config.toml`, so every Tome
command run from this tree resolves to the `contracts` workspace. If you'd rather
create a workspace without binding a directory, use
`tome workspace init <name>`; `tome workspace init --bind <name>` is the mirror
of `use --create`.

Add the catalog if you don't already have it:

```bash
tome catalog add devrelaicom/midnight-expert-tome
```

Catalog enrolment is recorded per workspace, so a catalog you add here belongs to
`contracts`, not to every workspace.

## 2. Add some plugins

Enable a plugin against the active workspace. Plugins are addressed as
`<catalog>/<plugin>`:

```bash
tome plugin enable midnight-expert/midnight-verify
```

Each `plugin enable` is recorded against the *active* workspace only. The first
enable offers to download the search models if they're missing (see
[Install](./install.md)). Check what the workspace now holds:

```bash
tome workspace info
```

The report names the active workspace, its enrolled catalogs, and its enabled
plugins.

## 3. Link it to a project

You already bound this directory in step 1: `tome workspace use --create` writes
the `.tome/config.toml` marker that ties the current tree to `contracts`. A
workspace can be bound to more than one project directory, and the binding is
what makes the right composition activate automatically when you work in that
project.

Confirm the binding:

```bash
tome workspace current   # prints: contracts
```

`tome workspace current` prints just the bound name on one line, with no
decoration, so it drops into a shell prompt or a script:
`$(tome workspace current 2>/dev/null)`. When nothing is bound to the current
directory it writes nothing to stdout and exits `12` (`workspace_not_bound`), so
the substitution collapses to empty rather than to an error.

## 4. Add more plugins and re-sync

Enable another plugin, and pass `--sync` to apply the change to your harnesses in
the same step:

```bash
tome plugin enable midnight-expert/midnight-dapp-dev --sync
```

Without `--sync`, enable updates the index and prints a reminder to run
`tome sync`. With it, enable runs the same propagation `tome sync` performs: it
writes each bound project's `.tome/RULES.md` and reconciles that project's
harness files. So the new plugin's entries are searchable immediately, and your
configured harnesses see the updated composition without a second command.

## 5. Keep the project in sync

`tome sync` propagates the active workspace's composition to its bound projects.
Run bare inside a bound project, it reconciles that project; `--all` fans out to
every project bound to the active workspace. Run bare outside any project, it
falls back to the `--all` fan-out over the resolved workspace's bound projects
and prints a note to stderr, so a stray `tome sync` from your home directory
still reaches every bound project rather than erroring.

```bash
tome sync         # sync the current project (or fan out when there isn't one)
tome sync --all   # sync every project bound to the active workspace
```

For each project, `tome sync` reconciles four things:

| Sink | What Tome writes |
| --- | --- |
| Rules file | the workspace's routing directive and each plugin's `GUARDRAILS.md` prose, as marker-bounded regions in the harness's rules file (`CLAUDE.md`, `AGENTS.md`, or a harness-specific file) |
| MCP config | wiring so the harness can reach `tome mcp`, with the active workspace stamped into the spawned server's arguments |
| Native agents | each plugin agent translated to the harness's native agent format, where the harness has one |
| Hooks | event-driven actions, where the harness supports them |

Tome only ever edits inside its own markers, so hand-written content in a shared
rules file is left alone. `tome sync` is byte-for-byte idempotent: re-running it
changes nothing. If a project looks out of sync, `tome doctor` reports it and
`tome doctor --fix` re-runs the reconciliation. See
[Harnesses](../using-tome/harnesses.md#syncing-bound-projects) for the full
`tome sync` behaviour and the per-harness sinks.

## 6. Create a second workspace

Move to a different project and create a second workspace with a different
composition:

```bash
cd ~/code/dapp
tome workspace use --create dapp
tome catalog add devrelaicom/midnight-expert-tome
tome plugin enable midnight-expert/midnight-dapp-dev --sync
```

The `dapp` workspace now enables `midnight-dapp-dev`; the `contracts` workspace
still enables `midnight-verify` and `midnight-dapp-dev`. The two compositions are
independent because plugin enablement is recorded per workspace.

## 7. Use it with a different project

The `~/code/dapp` directory is now bound to `dapp`—the
`tome workspace use --create dapp` in step 6 wrote its `.tome/config.toml`
marker. To point the two harnesses at their respective workspaces, configure
each one and sync. For `claude-code` and `cursor`, from inside the `dapp`
project:

```bash
tome harness use claude-code cursor
```

Replace the names with `codex`, `gemini`, or `opencode` as needed.
`tome harness use` resolves the active workspace's composition and writes native
config for exactly that set, so the config it lands under `~/code/dapp` reflects
`dapp`, and the config under `~/code/contracts` reflects `contracts`. See
[Harnesses](../using-tome/harnesses.md) for what Tome writes per harness.

## 8. Switch between workspaces

Which workspace is active is decided per directory. Tome resolves the active
workspace for a command in this order, taking the first that matches:

| Priority | Source |
| --- | --- |
| 1 | `--workspace <name>` flag (`-w` for short) |
| 2 | `TOME_WORKSPACE` environment variable |
| 3 | `[workspace] default` in `~/.tome/config.toml` |
| 4 | the nearest `.tome/config.toml` project marker, walking up from the current directory |
| 5 | the `global` workspace (the fallback) |

So `cd ~/code/contracts` resolves to `contracts` and `cd ~/code/dapp` resolves to
`dapp` through the project-marker walk (priority 4), with no explicit flag. To
re-bind the current directory to a different existing workspace, run
`tome workspace use <name>`:

```bash
tome workspace use contracts   # re-bind this directory to `contracts`
```

`tome workspace list` shows every workspace and marks the one resolved for the
current directory with a `*` in the `Cur` column:

```bash
tome workspace list
```

A global `[workspace] default` (priority 3) wins over a project marker (priority
4). When one is set, the per-project binding goes inactive: `tome sync`,
`${TOME_PROJECT_DIR}`, and the status `harness_mcp` report stop tracking the
project. Tome prints a one-line `note:` on stderr when this happens, so the
shadowing is never silent. Unset the default or run `tome workspace use` in the
project to restore the binding. See [Workspaces](../using-tome/workspaces.md) for
the full lifecycle.

## 9. MCP auto-switching

The search you run at a shell with `tome query` is the same search a coding agent
runs through the [MCP server](../using-tome/mcp-server.md). The payoff of binding
a workspace to a project is that the running server serves the right workspace's
skills per project, with no manual step. The mechanism is a flag `tome sync`
writes and the harness passes back.

When `tome sync` writes a harness's MCP config for a project, it stamps the bound
workspace into the spawned server's arguments:

```text
tome mcp --workspace <ws> --harness <name>
```

`<ws>` is the workspace bound to *that* project. The harness launches this exact
command as a stdio subprocess when you open the project. `--workspace <ws>` is
Tome's global scope flag (priority 1 in the table above), so it pins the server's
resolved scope to `<ws>` before anything else is consulted. The server builds its
searchable corpus and its prompt list from that one scope, and `search_skills`
runs its KNN over exactly that workspace's enabled entries. The `corpus_size`
field on every result reports the size of that scope, so the count reflects the
bound workspace, not the whole index.

Open `~/code/contracts` in Claude Code and its server starts with
`--workspace contracts`, so `search_skills` returns `contracts` entries. Open
`~/code/dapp` and its server starts with `--workspace dapp`, so the same tool
returns `dapp` entries. The switch is the harness launching a different
subprocess for a
different project, each pinned by the flag `tome sync` wrote into that project's
config. There is one central index under `~/.tome/`; the `--workspace` flag is
what scopes each server's view of it.

When you change a workspace's composition, re-sync so the stamp and the routing
directive stay current: `tome plugin enable ... --sync`, or `tome sync` after the
fact. A running server also picks up composition changes without a restart—it
polls the index about once a minute and rebuilds its prompt list and tool
description in place when the workspace's entries drift. The refresh is a poll,
not an event, so a change can take up to that interval to appear; if a rebuild
cycle fails, the server keeps serving the last good state and retries on the next
tick rather than going dark. A harness that reconnects to a fresh subprocess gets
the current corpus immediately, either way.

## Pitfalls

- Naming a workspace that doesn't exist (without `--create`) exits `13`
  (`workspace_not_found`); create it first with `tome workspace init <name>` or
  `tome workspace use --create <name>`.
- `tome workspace current` exits `12` (`workspace_not_bound`) when the current
  directory has no binding—that's the signal for scripts, not a failure to fix.
- `tome sync` outside any project with no bound projects exits `2` (`usage`) and
  names the next step; bind a project with `tome workspace use` or run
  `tome sync --all` once you have bindings.
- A plugin with only a legacy Claude Code `plugin.json` isn't loaded—that's
  exit `80` (`plugin_not_converted`);
  [`tome plugin convert`](../authoring/convert.md) migrates it.

## Next steps

- [Concepts](./concepts.md)—the model behind catalogs, plugins, and workspaces.
- [Workspaces](../using-tome/workspaces.md)—the full workspace lifecycle and
  composition model.
- [Harnesses](../using-tome/harnesses.md)—how a workspace's composition is
  written to each agent's native config.
- [MCP server](../using-tome/mcp-server.md)—the search-then-load tools an agent
  uses at runtime.
- [Commands reference](../reference/commands.md#tome-workspace)—every command
  and flag.

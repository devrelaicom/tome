---
title: Quickstart
sidebar_position: 2
---

# Quickstart

`tome init` is the fastest path from a fresh install to an agent that loads
exactly the skill it needs during a task. It's a single guided command that
walks you through the same setup you'd otherwise do by hand — binding a
workspace, configuring your coding agents, adding a catalog, and enabling
plugins — and stops to confirm before it writes anything. If you haven't
installed Tome yet, start with [Install](./install.md).

Prefer to run each step yourself and see exactly what it does? The
[Manual Quickstart](./manual-quickstart.md) covers the same flow one command at
a time.

## Run the wizard

```bash
tome init
```

`tome init` is interactive: it needs a terminal, and every prompt can be skipped
with `Esc`. It's also idempotent — run it again any time and it offers only the
steps you haven't finished yet, so it's safe to re-run as you go. Nothing is
written until you answer a prompt.

It opens by naming the workspace it resolved for the current directory, then
works through the outstanding steps in order, each with a `[step/total]` banner:

```text
Workspace: `global` (the default global workspace)

[1/4] bind this directory to a workspace
```

A step whose precondition is already met is skipped silently, so a partly
configured machine sees a shorter list. If everything is already set up, `init`
prints `Everything is already set up — nothing to do.` and jumps straight to the
status panel.

## 1. Bind a workspace

A **workspace** is a per-project scope — its own catalogs and plugins, so the
set active for a contracts project never leaks into a web app's context. When
the current directory isn't bound to one, `init` offers to bind it:

```text
This directory has no project binding. Bind it to a workspace?
> Bind to `contracts`
  Create a new workspace and bind to it
  Skip
```

Pick an existing workspace, or choose **Create a new workspace and bind to it**
and give it a name (1–64 characters of letters, digits, `-`, or `_`). Binding
writes a `.tome/config.toml` marker in the current directory, so every Tome
command run from this tree resolves to that workspace. Skip it, and the
remaining steps configure the `global` workspace instead. This step composes the
same `tome workspace use --create <name>` you'd run by hand.

## 2. Configure your harnesses

`init` detects the coding agents installed on your machine and offers to point
each one at Tome:

```text
[2/4] configure detected harnesses
Select harnesses to configure (space toggles, enter confirms; none = skip)
> [x] claude-code
  [x] cursor
  [ ] codex
```

The ones you select get Tome's native configuration — a rules file, MCP server
wiring, and native agents and hooks where the harness supports them. This is
`tome harness use` under the hood; see [Harnesses](../using-tome/harnesses.md)
for the seventeen supported agents and what Tome writes for each. If you skipped
the bind step, `init` notes that it's configuring at `--scope global` rather
than for a single project.

## 3. Add a catalog

A **catalog** is a git repository of plugins. When your workspace has none
enrolled, `init` asks for one:

```text
[3/4] add a plugin catalog
Catalog source (empty to skip)
> devrelaicom/midnight-expert-tome
  (hint: an owner/repo GitHub shorthand, a git URL, or a file:// path)
```

This example catalog — thirteen plugins of Midnight development expertise — is
the same one the [Manual Quickstart](./manual-quickstart.md) and
[Workspaces quickstart](./workspaces-quickstart.md) use, so you can compare your
output with a working setup. Tome clones the source, parses every plugin inside,
and registers them all; the plugins start disabled. If a source can't be
reached, `init` reports the error and asks again rather than giving up. This step
is `tome catalog add <source>`.

## 4. Enable plugins

Enabling a plugin parses, embeds, and indexes its entries for search. `init`
lists every disabled plugin across your enrolled catalogs so you can pick which
to turn on:

```text
[4/4] enable plugins
Note: enabling a first plugin downloads the search models
      (you will be asked to confirm before anything downloads).
Select plugins to enable (space toggles, enter confirms; none = skip)
> [x] midnight-expert/midnight-verify
  [ ] midnight-expert/midnight-dapp-dev
```

The first enable needs Tome's local search models. If they aren't on disk yet,
`init` shows what it will fetch and its size before the selection, and the
enable step asks you to confirm the download — decline and nothing is enabled.
See [Install](./install.md) for the model profiles. This step is
`tome plugin enable <catalog>/<plugin>`.

## Finish

When the steps are done, `init` prints a `tome status` panel — your models,
workspace, enabled entries, and index — followed by a next move:

```text
Setup complete. Try: tome query "verify a Compact contract"
```

If you skipped a step or one didn't complete, `init` lists the remaining manual
commands instead and reminds you that a re-run picks up where you left off:

```text
Remaining steps:
  - tome catalog add <source>  (add a plugin catalog)
  - tome plugin enable <catalog>/<plugin>  (enable plugins)
Re-run `tome init` any time to pick up where you left off.
```

Each step runs independently: if one fails, `init` warns and continues so the
later steps still run, then surfaces the first failure's exit code at the end.

## Search

With plugins enabled, run a semantic search across every enabled skill and
command:

```bash
tome query "verify a Compact contract"
```

```text
top_k=10  rerank=false  min_score=none  (10 results)
|   Score | Catalog         | Plugin          | Name                                | Type  |
|---------|-----------------|-----------------|-------------------------------------|-------|
|  0.7412 | midnight-expert | midnight-verify | midnight-verify:verify-by-execution | skill |
```

That query ran a KNN vector search over the embeddings, entirely on your
machine. Reranking is off by default; add `--rerank` to run the reranker over
the KNN hits (it needs the ~280 MB reranker model). Inside a configured harness
the same search runs over the
[MCP server](../using-tome/mcp-server.md) — the agent searches, then loads only
the top result instead of holding every indexed entry in context.

## Pitfalls

- `tome init` is interactive. With no terminal (piped, or in CI) it exits `54`
  (`not_a_terminal`) and prints the manual steps to run instead — see the
  [Manual Quickstart](./manual-quickstart.md).
- `tome init --json` exits `2` (`usage`): the wizard is interactive-only. Script
  the individual commands (`tome catalog add`, `tome plugin enable`,
  `tome harness use`) with `--json` instead.
- A plugin with only a legacy Claude Code `plugin.json` isn't loaded — that's
  exit `80` (`plugin_not_converted`);
  [`tome plugin convert`](../authoring/convert.md) migrates it.

## Next steps

- [Manual Quickstart](./manual-quickstart.md) — the same flow, one command at a
  time, when you want to see each step.
- [Workspaces quickstart](./workspaces-quickstart.md) — one scope per project,
  switched automatically.
- [Concepts](./concepts.md) — the model behind catalogs, plugins, and
  workspaces.
- [Commands reference](../reference/commands.md) — every command and flag.
- [Troubleshooting](../using-tome/troubleshooting.md) — `tome doctor` and common
  issues.

---
title: Manual Quickstart
sidebar_position: 3
---

# Manual Quickstart

Four commands take you from a fresh install to an agent that loads exactly the
skill it needs during a task. This page runs them against a real catalog —
`devrelaicom/midnight-expert-tome`, thirteen plugins of Midnight development
expertise — so you can compare your output with a working setup. If you haven't
installed Tome yet, start with [Install](./install.md).

Want to run all of this in one guided step instead? [`tome init`](./quickstart.md)
walks you through the same flow and stops to confirm before each action.

## 1. Add a catalog

A catalog is a git repository of plugins. Add it once, and Tome clones it,
parses every plugin inside, and registers them all:

```bash
tome catalog add devrelaicom/midnight-expert-tome
```

Tome confirms the add with the ref it pinned and the plugin count — for this
catalog, `plugins: 13`. Nothing is indexed yet: plugins start disabled, and you
choose which ones to enable.

## 2. Enable a plugin

Enabling a plugin parses, embeds, and indexes its entries for search. Plugins
are addressed as `<catalog>/<plugin>`:

```bash
tome plugin enable midnight-expert/midnight-verify
```

The first enable offers to download the search models if they're missing (see
[Install](./install.md)). Then list your plugins:

```bash
tome plugin list
```

```text
| Catalog         | Plugin                | Version | Status     | Entries                           | Last indexed | Last upstream change |
|-----------------|-----------------------|---------|------------|-----------------------------------|--------------|----------------------|
| midnight-expert | midnight-verify       | 0.13.0  | ✓ enabled  | (19 skills, 2 commands, 7 agents) | just now     | just now             |
```

That's one row of thirteen — the other twelve plugins stay `✗ disabled` until
you enable them.

## 3. Point a harness at Tome

Tome writes native configuration for each supported harness — rules files, MCP
server wiring, and (where the harness supports them) native agents and hooks.

```bash
tome harness use cursor
```

Replace `cursor` with any of the seventeen supported harness names —
`claude-code`, `codex`, `gemini`, `zed`, `copilot`, and more. Run
`tome harness use` with no names to configure every harness Tome auto-detects.
See [Harnesses](../using-tome/harnesses.md) for the full table and what Tome
writes for each.

## 4. Search

Run a semantic search across every enabled skill and command:

```bash
tome query "verify a Compact contract"
```

```text
top_k=10  rerank=false  min_score=none  (10 results)
|   Score | Catalog         | Plugin          | Name                                      | Type  | Version | Path                                                      |
|---------|-----------------|-----------------|-------------------------------------------|-------|---------|-----------------------------------------------------------|
|  0.7412 | midnight-expert | midnight-verify | midnight-verify:verify-by-execution       | skill | 0.13.0  | skills/midnight-verify:verify-by-execution/SKILL.md       |
|  0.6685 | midnight-expert | midnight-verify | midnight-verify:verify-by-zkir-checker    | skill | 0.13.0  | skills/midnight-verify:verify-by-zkir-checker/SKILL.md    |
|  0.6493 | midnight-expert | midnight-verify | midnight-verify:verify-compact            | skill | 0.13.0  | skills/midnight-verify:verify-compact/SKILL.md            |
```

(Top three of ten rows shown. The dim header line above the table — shown
only in an interactive terminal — reports the effective `top_k`, `rerank`,
and `min_score` knobs plus the result count.)

:::note[What just happened]

That query ran a KNN vector search over the embeddings, entirely on your
machine. Reranking is off by default; add `--rerank` to run the reranker over
the KNN hits (it needs the ~563 MB reranker model). Inside a configured harness
the same search runs over the
[MCP server](../using-tome/mcp-server.md) — the agent searches, then loads only
the top result instead of holding all 28 indexed entries in context.

:::

## Pitfalls

- Adding a catalog that's already registered exits `4`
  (`catalog_already_exists`).
- Enabling a plugin that's already enabled exits `21`
  (`plugin_already_in_state`).
- A plugin with only a legacy Claude Code `plugin.json` isn't loaded — that's
  exit `80` (`plugin_not_converted`);
  [`tome plugin convert`](../authoring/convert.md) migrates it.

## Next steps

- [Quickstart](./quickstart.md) — the same flow guided by `tome init`.
- [Concepts](./concepts.md) — the model behind catalogs, plugins, and workspaces.
- [Plugins & catalogs](../using-tome/plugins-and-catalogs.md) — the day-to-day
  lifecycle.
- [Commands reference](../reference/commands.md) — every command and flag.
- [Troubleshooting](../using-tome/troubleshooting.md) — `tome doctor` and common
  issues.

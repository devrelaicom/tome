---
title: Concepts
sidebar_position: 3
---

# Concepts

Every coding agent has its own place to put knowledge — rules files, skill
directories, MCP config — and none of them read each other's. Tome organises
all of it behind a small set of concepts. Everything else in the docs builds on
these terms.

```text
catalog (git repo)
  └─ plugins
       └─ entries (skills · commands · agents · hooks)
                │
                │  tome plugin enable
                ▼
        central index — ~/.tome/ (SQLite + vector search, fully local)
                │
        ┌───────┴──────────────┐
        ▼                      ▼
    tome query             tome mcp
    (you, at a shell)      (your agent, mid-task)

tome harness use <name> writes each harness's native config:
Claude Code · Cursor · Codex · Gemini CLI · OpenCode
```

## Catalog

A **catalog** is a git repository of plugins. You add a catalog by pointing Tome
at its repo; Tome clones it and registers every plugin it contains. Catalogs are
the unit of distribution — to share your work, publish a catalog and tell
people to add it.

```bash
tome catalog add <repo>
```

## Plugin

A **plugin** is a bundle of capabilities inside a catalog, described by a
`tome-plugin.toml` manifest. Plugins are enabled or disabled individually;
enabling one parses, embeds, and indexes its entries, making them available to
your harnesses and to search. Plugins that only have a legacy Claude Code
`plugin.json` aren't loaded — [`tome plugin convert`](../authoring/convert.md)
migrates them; unconverted plugins exit `80`.

```bash
tome plugin enable <catalog>/<plugin>
```

## Entry kinds

A plugin contributes four kinds of **entry**:

- **Skill** — a unit of knowledge or procedure (`SKILL.md` with frontmatter). The
  primary thing semantic search retrieves.
- **Command** — a user-invocable action, exposed as a slash command in supporting
  harnesses and as an MCP prompt.
- **Agent** — a persona with its own system prompt, translated to each harness's
  native agent format (where supported).
- **Hook** — an event-driven action wired into the harness (for example, real
  Claude Code hooks).

## Routing tier

Every enabled skill and command has a **routing tier** in each workspace: a
number from `1` to `3` that decides how the agent is told to reach it. **Tier 3**
is the default — the agent finds the entry through semantic search (`search_skills`)
when a task calls for it, and nothing is loaded up front. **Tier 1** and **Tier 2**
raise an entry's priority: Tome injects an instruction into the harness's rules
file so the agent knows to fetch it directly with `get_skill`. Tier 1 entries are
loaded at session start; Tier 2 entries are loaded when the task matches the
entry's description.

Tiers are per-workspace, so the same skill can be search-only in one project and
loaded up front in another. Agents carry no tier; only skills and commands do.

```bash
tome tier set <plugin>/<name> 1
```

See [`tome tier`](../reference/commands.md#tome-tier) for the full command surface,
and `tome plugin list --tier` / `tome plugin show --details` to see the tiers your
enabled entries currently hold.

## Workspace

A **workspace** is a per-project scope. Different workspaces can enable different
catalogs and plugins, and a workspace can be bound to one or more project
directories so the right composition is active for the right project.

```bash
tome workspace use <name>
```

## Harness

A **harness** is a coding agent that Tome targets: Claude Code, Cursor, Codex,
Gemini CLI, or OpenCode. Running `tome harness use <name>` writes that harness's
native configuration — rules file, MCP server wiring, and native agents/hooks
where the harness supports them.

## The central index

Tome maintains a single local SQLite index (with a vector extension for semantic
search) under `~/.tome/`. Adding a catalog or enabling a plugin updates this
index. It powers both `tome query` and the MCP server's search. The index, the
embedding model, and the reranker all stay on your machine — there is no
telemetry.

## MCP server

`tome mcp` runs Tome as an [MCP](https://modelcontextprotocol.io) server. It
exposes search and skill-loading tools (`search_skills`, `get_skill`,
`get_skill_info`), a `meta` tool that installs Tome's bundled meta skills into
the host harness, plus user-invocable entries as MCP prompts — so an agent can
find and load only the skills it needs at runtime rather than holding everything
in context. See the [MCP server](../using-tome/mcp-server.md) page for wiring.

## Meta skill

A **meta skill** is a skill about Tome itself — curated, bundled inside the
binary, and installed as a native `SKILL.md` into your harness so your agent
knows how to drive Tome. The first one, `convert-marketplace`, guides an agent
through converting a Claude Code marketplace into a Tome catalog. See
[Meta skills](../using-tome/meta-skills.md).

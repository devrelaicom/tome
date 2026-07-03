---
title: MCP server
sidebar_position: 6
---

# MCP server

`tome mcp` runs Tome as a [Model Context Protocol](https://modelcontextprotocol.io)
server. This is how a coding agent searches your catalogs and loads skills at
runtime, instead of holding everything in context.

```bash
tome mcp
```

The server uses MCP over stdio, so harnesses launch it as a subprocess.

## Tools

Tome exposes four tools. The first three form a search-then-load flow:

- **`search_skills`** — semantic search over enabled skills and commands. Returns
  candidate matches (KNN + reranker), so the agent can decide what's relevant.
- **`get_skill_info`** — a middle tier that returns metadata about a skill
  (including its `when_to_use` guidance) without pulling the full body. Useful for
  confirming relevance before loading. The `name` accepts a `*` wildcard that
  resolves to a unique match — a fuzzy name no longer forces a re-search. If the
  wildcard matches several entries the error lists the candidate `(name, kind)`
  pairs so the agent can pick one; if a name (exact or wildcard) resolves to
  nothing, the error's `data` carries the available `(name, kind)` entries for
  that `(catalog, plugin)`, so the agent needn't round-trip back to
  `search_skills`.
- **`get_skill`** — loads a skill's full content, with variable substitution
  applied, ready for the agent to use. Pass `raw: true` to receive the body with
  literal `${TOME_*}` substitution tokens preserved instead — useful for
  authoring or conversion workflows that need the source tokens, not the
  resolved values. Pass `include_resource_bodies: true` to also inline the
  contents of small text resources as `{ path, content }` alongside the resource
  paths, avoiding a separate file read per resource (and working even when the
  host's file tool can't reach a path). Inlining is byte-capped per file and in
  total, so binary, oversized, or budget-exceeding resources are skipped — their
  paths still appear in `resources` for the agent to fetch itself.

The typical loop is: `search_skills` to find candidates → `get_skill_info` to
confirm → `get_skill` to load only the best match.

The fourth tool lets the agent extend its own harness:

- **`meta`** — installs a bundled [meta skill](./meta-skills.md) into the
  **host harness**, the agent the server is running inside. Install-only:
  there is no removal over MCP. The host's identity comes from the `--harness`
  flag that `tome harness sync` stamps into the server arguments
  (`tome mcp --workspace <ws> --harness <name>`). If the server was started
  with no host, an unknown one, or one without native skill support, the tool
  **fails closed** with the `no_harness_detected` category — the MCP
  counterpart of [exit code `89`](../reference/exit-codes.md) — rather than
  guessing where to write.

## Prompts

User-invocable entries — **commands** and agent **personas** (when enabled) — are
exposed as **MCP prompts**. In a harness that surfaces prompts, these appear as
slash commands the user can invoke directly, with argument substitution handled
by Tome.

Tome also registers one built-in prompt of its own:
**`add-tome-conversion-skill`**, which installs the `convert-marketplace`
[meta skill](./meta-skills.md) into the host harness. It is always on, and
plugin prompts never replace it — a plugin entry with the same name gets a
suffix instead.

## Configure your editor

You normally don't configure this by hand. Running

```bash
tome harness use <name>
```

writes the MCP server configuration for that harness automatically, so the
editor knows to launch `tome mcp` — with the active workspace and the host
harness stamped into the arguments — and which tools are available. See
[Harnesses](./harnesses.md) for what's written per harness.

If you configure an MCP client manually, set it to run the `tome mcp` command
over stdio (without `--harness`, the `meta` tool will refuse to install —
everything else works). If the server fails to start,
[Troubleshooting](./troubleshooting.md) and `tome doctor` will report why.

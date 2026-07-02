---
title: Harnesses
sidebar_position: 3
---

# Harnesses

You probably don't use one coding agent. You might use Claude Code at work,
Cursor for a side project, and a new tool next month. Each one has its own
place for rules, skills, and MCP config — and none of them reads the others'.
Tome handles this for you: run `tome harness use <name>` and your enabled
plugins are written to that agent's native configuration.

A **harness** is a coding agent Tome targets. Tome supports five:
**Claude Code, Cursor, Codex, Gemini CLI, and OpenCode**. Running

```bash
tome harness use <name>
```

translates your enabled plugins into that harness's native configuration. Tome
keeps each harness in sync as you enable or disable plugins.

## What `tome harness use` writes

For every harness, Tome reconciles three sinks, in this order: **hooks →
guardrails (rules) → agents**. What is written depends on what the harness
supports:

- **Rules file** — a prose fallback (rendered from each plugin's `GUARDRAILS.md`)
  written as per-plugin marker regions in the harness's rules file. Tome only
  ever edits inside its own markers, so your hand-written content is left alone.
- **MCP server config** — wiring so the harness can reach `tome mcp` for search
  and skill loading. When `tome harness sync` writes this wiring, it stamps the
  host's identity into the spawned server's arguments
  (`tome mcp --workspace <ws> --harness <name>`) — the
  [MCP server](./mcp-server.md)'s `meta` tool relies on that stamp to know which
  harness it is installing into.
- **Native agents** — each plugin agent translated to the harness's native agent
  format, where the harness has one.
- **Hooks** — event-driven actions, where the harness supports them.

## Per-harness summary

| Harness | Rules sink | Native agents | Native hooks |
| --- | --- | --- | --- |
| Claude Code | `CLAUDE.md` | yes | yes (real JSON hooks) |
| Cursor | rules file (+ a Tome-owned sibling) | yes | — |
| Codex | `AGENTS.md` | yes | — |
| Gemini CLI | rules file | **no** (no native agents) | — |
| OpenCode | rules file | yes | — |

## Per-harness caveats

- **Claude Code** — the rules sink is `CLAUDE.md` (not `AGENTS.md`). Hooks are
  written as real JSON hooks, merged structurally into
  `.claude/settings.local.json` — never the committed `settings.json`. Agent
  personas can optionally be exposed as MCP prompts (off by default).
- **Cursor** — Tome maintains the rules file and a Tome-owned sibling rules file;
  the sibling is removed when it would otherwise be empty.
- **Codex** — the rules sink is `AGENTS.md`. Native agents are supported.
- **Gemini CLI** — has **no native agent format**, so agents are not translated
  natively; rules and MCP wiring still apply. Where agent personas are needed,
  use the MCP-prompt path.
- **OpenCode** — native agents are supported. OpenCode's rules file uses an
  inline body style for the managed regions.

## Inspecting and removing

```bash
tome harness list          # show configured harnesses
tome harness info          # show what Tome manages for every effective harness
tome harness info <name>   # ...or just one harness
tome harness sync          # re-write native config from current state
tome harness remove <name> # remove Tome-managed config for a harness
```

Before you declare any harness, `tome harness list` reports that:

```text
No harnesses declared in any settings layer.
```

Harness declarations live in Tome's layered settings; `tome harness use <name>`
adds one.

Bare `tome harness` opens an interactive picker. For repairing a harness's
configuration, see [Troubleshooting](./troubleshooting.md) and `tome doctor`.

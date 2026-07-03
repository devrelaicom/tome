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

A **harness** is a coding agent Tome targets. The five primary harnesses,
documented in the tables below, are **Claude Code, Cursor, Codex, Gemini CLI,
and OpenCode**; Tome configures a wider set of named harnesses beyond these,
plus two opt-in write targets covered below. Running

```bash
tome harness use <name>
```

translates your enabled plugins into that harness's native configuration. `use`
takes one or more names, so `tome harness use claude-code cursor` configures
both in one call. Tome keeps each harness in sync as you enable or disable
plugins.

To configure every harness Tome can auto-detect, pass `--all`:

```bash
tome harness use --all
```

`--all` skips the two **opt-in targets**, `generic` and `generic-op`. An opt-in
target is a portable write target that never auto-detects, so `--all` leaves it
out and prints a one-line `note:` on stderr naming the ones it skipped (human
output only, suppressed under `--json`). To fold them in, add
`--include-opt-in`, which requires `--all`:

```bash
tome harness use --all --include-opt-in
```

To configure a single opt-in target, name it directly: `tome harness use generic-op`.

## What `tome harness use` writes

For every harness, Tome reconciles three sinks, in this order: **hooks →
guardrails (rules) → agents**. What is written depends on what the harness
supports:

- **Rules file** — a prose fallback (rendered from each plugin's `GUARDRAILS.md`)
  written as per-plugin marker regions in the harness's rules file. Tome only
  ever edits inside its own markers, so your hand-written content is left alone.
- **MCP server config** — wiring so the harness can reach `tome mcp` for search
  and skill loading. When `tome sync` writes this wiring, it stamps the
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

The two opt-in targets are portable write targets rather than named agents:

| Target | What it writes |
| --- | --- |
| `generic` | `AGENTS.md` plus an MCP config alongside it, for any agent that reads those files |
| `generic-op` | an Open Plugins `tome-op` bundle, for hosts that consume Open Plugins |

Neither auto-detects, so `--all` skips both unless you pass `--include-opt-in`.

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
tome harness preview <name> # what sync would deliver vs drop for this harness (read-only)
tome sync                  # re-write native config from current state
tome harness remove <name> # remove Tome-managed config for a harness
```

`tome harness preview <name>` reports, per enabled entry, what `tome sync`
would deliver to that harness and what it would drop. For a plugin author
shipping across harnesses, this is how you check a target before you ship:
agents show as native, MCP-persona, or unrepresented (with any dropped
`model`/`tools` fields), skills and commands show their MCP routing, hooks show
whether each event reaches the harness natively or falls back to the
`GUARDRAILS.md` prose, and the report names the rules directive and MCP
registration target. Scope it to one plugin with `--plugin <id>`, or add
`--json`. The preview shares sync's own translation logic, so its verdict
matches what sync writes. It opens the index read-only, takes no lock, and
writes nothing; an unknown harness name exits `18`.

Before you declare any harness, `tome harness list` reports that:

```text
No harnesses declared in any settings layer.
```

Harness declarations live in Tome's layered settings; `tome harness use <name>`
adds one.

Bare `tome harness` opens an interactive picker. For repairing a harness's
configuration, see [Troubleshooting](./troubleshooting.md) and `tome doctor`.

## Syncing bound projects

`tome sync` propagates workspace state to bound projects, writing each one's
rules file and reconciling its harness files. Run inside a bound project it
reconciles that project alone; `--all` fans out to every project bound to your
active workspace.

```bash
tome sync         # sync the current project, or fan out when there isn't one
tome sync --all   # sync every project bound to the active workspace
```

Where `tome sync` acts depends on where you run it:

- **Inside a bound project**, it syncs that project.
- **With `--all`**, it fans out to every project bound to the active workspace.
  The fan-out stays inside that workspace's bindings and never reaches projects
  bound to another workspace.
- **Outside any project, without `--all`**, it falls back to the `--all`
  fan-out over the resolved workspace's bound projects instead of erroring. It
  prints a note to stderr so it's clear it acted outside the current directory;
  `--json` output is identical to `--all`. If the workspace has no bound
  projects, it exits `2` and names the next step, `tome workspace use` to bind a
  project or `tome sync --all` once you have bindings.

See [`tome sync`](../reference/commands.md#tome-sync) for the full flag set.

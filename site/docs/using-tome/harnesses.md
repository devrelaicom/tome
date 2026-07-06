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

A **harness** is a coding agent Tome targets. Tome configures seventeen named
harnesses — Claude Code, Codex, Cursor, Gemini CLI, OpenCode, GitHub Copilot
(VS Code and CLI), Devin, Cline, Junie, JetBrains AI Assistant, Antigravity,
Pi, Crush, Zed, Kiro, and Goose — plus two opt-in write targets covered below.
Running

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
- **Hooks** — event-driven actions. Claude Code receives plugin hooks as real
  JSON; five more harnesses receive them through a Tome dispatcher (see the
  table); everywhere else the plugin's `GUARDRAILS.md` prose is the fallback.

## Per-harness summary

One row per supported harness, derived from Tome's harness registry. Paths
starting `~/` are per-user files; every other path is project-relative. The
**MCP config** column notes whether Tome writes the entry itself (automatic),
leaves it to you (manual), or writes it but cannot confirm the harness consumes
it (unverified). In the **Native hooks** column, "yes" means plugin hooks merge
as real JSON, "dispatcher" means Tome translates them into the harness's native
hook file and dispatches them at fire time, and "no" means hooks fall back to
the `GUARDRAILS.md` prose in the rules sink.

| Harness | Rules sink | MCP config | Native agents | Native hooks |
| --- | --- | --- | --- | --- |
| Antigravity IDE (`antigravity`) | `.agent/rules/tome.md` (Tome-owned file) | `~/.gemini/config/mcp_config.json` (automatic) | no | no |
| Claude Code (`claude-code`) | `CLAUDE.md` (managed block) | `.claude/settings.json` (automatic) | yes | yes (real JSON hooks) |
| Cline (`cline`) | `.clinerules/tome.md` (Tome-owned file) | `~/.cline/mcp.json` (automatic) | no | no |
| Codex (`codex`) | `AGENTS.md` (managed block) | `~/.codex/config.toml` (automatic) | yes | dispatcher |
| GitHub Copilot, VS Code (`copilot`) | `.github/copilot-instructions.md` (managed block) | `.vscode/mcp.json` (automatic) | yes | no |
| GitHub Copilot CLI (`copilot-cli`) | `.github/copilot-instructions.md` (managed block) | `~/.copilot/mcp-config.json` (automatic) | yes | dispatcher |
| Crush (`crush`) | `CRUSH.md` (managed block) | `crush.json` (automatic) | no | no |
| Cursor (`cursor`) | `.cursor/rules/TOME_SKILLS.md` (Tome-owned file) | `.cursor/mcp.json` (automatic) | yes | dispatcher |
| Devin (`devin`) | `AGENTS.md` (managed block) | `.devin/config.json` (automatic) | yes | dispatcher |
| Gemini CLI (`gemini`) | `AGENTS.md` or `GEMINI.md` (managed block) | `~/.gemini/settings.json` (automatic) | yes | dispatcher |
| Goose (`goose`) | `AGENTS.md` inside the `tome-op` bundle | `.mcp.json` inside the `tome-op` bundle (automatic) | yes | no |
| JetBrains AI Assistant (`jetbrains-ai`) | `.aiassistant/rules/tome.md` (Tome-owned file) | manual — Settings UI, no file written | no | no |
| Junie (`junie`) | `.junie/AGENTS.md` (managed block) | `.junie/mcp/mcp.json` (automatic) | no | no |
| Kiro (`kiro`) | `.kiro/steering/tome.md` (Tome-owned file) | `.kiro/settings/mcp.json` (automatic) | yes | no |
| OpenCode (`opencode`) | `AGENTS.md` (managed block) | `opencode.json` (automatic) | yes | no |
| Pi (`pi`) | `AGENTS.md` (managed block) | `~/.pi/agent/mcp.json` (automatic; unverified until the adapter is installed) | yes | no |
| Zed (`zed`) | `.rules` (Tome-owned file) | `.zed/settings.json` (automatic) | no | no |

Six harnesses — `antigravity`, `cline`, `crush`, `jetbrains-ai`, `junie`, and
`zed` — are rules-only for agents and hooks: plugin agents have no native form
there (`tome status` and `tome doctor` report them as unrepresented), and
plugin hooks fall back to the `GUARDRAILS.md` prose. Rules and MCP wiring still
apply.

The two opt-in targets are portable write targets rather than named agents:

| Target | What it writes |
| --- | --- |
| `generic` | a managed block in `AGENTS.md` plus an `mcp.json` alongside it, for any agent that reads those files |
| `generic-op` | a self-contained Open Plugins `tome-op` bundle at `<project>/tome-op`, for hosts that consume Open Plugins |

Neither auto-detects, so `--all` skips both unless you pass `--include-opt-in`.

One alias: `antigravity-cli` resolves to `gemini`. The Antigravity CLI consumes
Gemini's configuration surface, so both names reach the same module, and naming
a harness twice — once by alias — collapses to one. The `antigravity` name is
the IDE, which has its own rules sink.

`tome status` and `tome doctor` report each harness's MCP state: `ok`,
`manual` (no file Tome can write), `unverified` (written, but dependent on an
external adapter), or `drift`. A harness never silently loses its MCP wiring —
a missing or foreign entry surfaces in both reports.

## Per-harness caveats

- **Claude Code** — the rules sink is `CLAUDE.md` (not `AGENTS.md`). Hooks are
  written as real JSON hooks, merged structurally into
  `.claude/settings.local.json` — never the committed `settings.json`. Agent
  personas can optionally be exposed as MCP prompts (off by default).
- **Codex** — the rules sink is `AGENTS.md`. The MCP entry is global TOML,
  `~/.codex/config.toml` under `mcp_servers`. Plugin hooks are dispatched
  through `.codex/hooks.json`.
- **Cursor** — Tome maintains a standalone rules file and a Tome-owned
  guardrails sibling (`TOME_GUARDRAILS.md`); the sibling is removed when it
  would otherwise be empty. A session-start hook and dispatched plugin hooks
  land in `.cursor/hooks.json`.
- **Gemini CLI** — native agents are supported (`.gemini/agents/`). The MCP
  entry is global (`~/.gemini/settings.json`) while hooks land in the project
  `.gemini/settings.json` — two different files, and the writers own disjoint
  keys, so neither clobbers the other. `antigravity-cli` is an alias for this
  harness.
- **OpenCode** — the rules block uses an inline body. The MCP entry uses
  OpenCode's own shape: an `mcp` key, a single `command` array,
  `"type": "local"`, and `"enabled": true`. The session-start directive ships
  as a TypeScript plugin shim at `.opencode/plugin/tome.ts`, executed by
  OpenCode's own runtime, never by Tome. Agents land in `.opencode/agent/`
  (singular).
- **GitHub Copilot (VS Code) and Copilot CLI** — two harnesses, one shared
  rules sink (`.github/copilot-instructions.md`, exactly one Tome block) and
  one co-owned agents directory (`.github/agents/`, byte-identical output from
  either). The VS Code side registers MCP under the `servers` key in
  `.vscode/mcp.json`; the CLI side is global (`~/.copilot/mcp-config.json`) and
  additionally gets a session-start hook at `.github/hooks/tome.json` plus
  dispatched plugin hooks.
- **Devin** — agents use a directory-per-agent layout,
  `.devin/agents/<plugin>__<name>/AGENT.md`. Plugin hooks are dispatched
  through `.devin/hooks.v1.json`.
- **Antigravity IDE** — rules-only for now: the directive rides
  `.agent/rules/tome.md`, and no hook file is written until Antigravity's hook
  shape is confirmed against a live install. Detection shares `~/.gemini/` with
  the `gemini` harness, so auto-detecting from that directory configures both;
  their sinks are distinct, so nothing collides.
- **Cline** — the session-start directive ships as a TypeScript plugin shim at
  `.cline/plugins/tome.ts`, executed by Cline's runtime. Rules-only for agents
  and hooks.
- **Pi** — Tome writes the MCP entry to `~/.pi/agent/mcp.json`, but it has no
  effect until you run `pi install pi-mcp-adapter`; `tome harness use` prints
  that notice, and `tome status`/`tome doctor` report the entry `unverified`
  until then. Run `tome harness info pi` for the paste-able snippet. The
  session-start directive ships as a TypeScript shim at `.pi/extensions/tome.ts`.
  Native agents are emitted but stay inert until Pi's `agentScope` is enabled.
- **JetBrains AI Assistant** — manual MCP: the assistant configures MCP servers
  through its Settings UI, so there is no file Tome can own and the sync skips
  the MCP sink. Run `tome harness info jetbrains-ai` for the exact snippet to
  paste; `tome doctor` reports the state as `manual`, not broken. The rules
  file carries an `apply: always` front-matter header.
- **Kiro** — the steering file carries an `inclusion: always` front-matter
  header so Kiro applies it on every session.
- **Zed** — rules land in `.rules` at the project root, Zed's
  highest-precedence project rules file. The MCP entry nests under the
  `context_servers` key in `.zed/settings.json`.
- **Goose** — integrates through a self-contained Open Plugins bundle at
  `.config/goose/plugins/tome-op` rather than the per-sink loop; the bundle
  carries its own `AGENTS.md` and `.mcp.json`. Native agents land in
  `.agents/agents/`.
- **Junie** — rules and MCP only: a managed block inside `.junie/AGENTS.md`
  (Junie's namespaced copy, not the project-root one) and a per-project MCP
  file.

For any harness whose MCP step is manual or unverified, `tome harness info
<name>` prints the exact paste-able MCP-config snippet in that harness's own
dialect.

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

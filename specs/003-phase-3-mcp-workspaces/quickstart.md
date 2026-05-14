# Phase 3 Quickstart

A walkthrough of the new Phase 3 surfaces from the perspective of a developer who already has a working Phase 1+2 install. Each section is independent — read what you need.

## Prerequisites

You have:

- Tome installed (`cargo install --path .` from this repo, or a release build of v0.3.0 / Phase 3).
- A working Phase 2 install: at least one catalog registered, at least one plugin enabled, the embedder and reranker models downloaded. `tome status` shows ✓ healthy.

## 1. Run an MCP server inside Claude Code

Register Tome's MCP server in `~/.claude/.claude.json` (or wherever your Claude Code config lives):

```json
{
  "mcpServers": {
    "tome": {
      "command": "tome",
      "args": ["mcp"]
    }
  }
}
```

Restart Claude Code. Open a project. Inside an agent session, ask the agent something where you'd expect a skill to help:

> "Generate a release-notes draft for a new feature."

Watch the tool calls in Claude Code's UI: the agent should call `search_skills` with a query similar to that prompt, then `get_skill` on the top result. The skill's body lands in the agent's context.

To run the server scoped to a specific project (so it sees only that project's enabled plugins), pass `--workspace`:

```json
{
  "mcpServers": {
    "tome-project": {
      "command": "tome",
      "args": ["mcp", "--workspace", "/abs/path/to/project"]
    }
  }
}
```

The MCP server's logs live at `~/.local/state/tome/mcp.log`. Tail them while debugging:

```sh
tail -F ~/.local/state/tome/mcp.log | jq -c
```

For verbose output: `TOME_LOG=debug` in the harness's per-server env block.

## 2. Initialize a workspace and add a project-specific catalog

```sh
cd ~/projects/acme-app
tome workspace init --inherit-global
```

You now have `~/projects/acme-app/.tome/config.toml` containing the catalogs you had globally, and an empty workspace index (no plugins enabled yet). Verify:

```sh
tome workspace info
# Workspace:       /home/you/projects/acme-app
#   resolved via:  CWD walk
#   catalogs:      3 (inherited from global)
#   plugins:       0 enabled
#   skills:        not yet bootstrapped
```

Enable a plugin specifically for this project:

```sh
tome plugin enable acme-catalog/release-notes
# (Phase 2 enable flow: embeds skills, populates the workspace's index)
```

Now `tome query` from inside this directory hits the workspace's index:

```sh
tome query "draft a release blog post"
# (Phase 2 query flow, but only the workspace's enabled plugins are in scope)
```

Step out of the workspace and the global state takes over again:

```sh
cd ~
tome query "draft a release blog post"
# Returns results from the global enabled set, not the workspace's.
```

## 3. Add a project-specific catalog that isn't in your global config

Inside the workspace:

```sh
cd ~/projects/acme-app
tome catalog add github:acme/internal-skills
```

This writes to `~/projects/acme-app/.tome/config.toml`, not the global one. If you'd already added the same catalog globally, Tome reuses the existing on-disk clone — no second `git clone` runs.

## 4. Operate on global state from inside a workspace

Sometimes you want to do something to your global state without leaving the project directory. Pass `--global`:

```sh
cd ~/projects/acme-app
tome --global plugin list             # list globally-enabled plugins
tome --global catalog add github:other/catalog   # add to the global config
```

`--global` and `--workspace` are mutually exclusive (exit 72). All other Phase 1/2 commands honour both flags.

## 5. Run a full diagnostic

```sh
tome doctor
```

Healthy output reports every subsystem as ok. If something's wrong:

```sh
tome doctor
# Models:
#   reranker  bge-reranker-base (base)  ✗ missing
# Suggested fixes:
#   Re-download the reranker (automatically with --fix):
#     tome models download
# Overall: ⚠ degraded
```

Auto-fix the safe classes:

```sh
tome doctor --fix
# [running] tome models download
# [done]    reranker installed
# Overall: ✓ healthy
```

The `--fix` flag handles three classes:

- Missing or corrupt model files (re-download).
- Broken or missing catalog clone directories (re-clone from the recorded URL).
- Older-than-expected DB schema (apply pending forward migrations).

Destructive repairs (drift recovery, schema-too-new) are NEVER auto-applied. They surface as suggested commands you run by hand.

## 6. Inspect a workspace's state

```sh
tome workspace info --json | jq
```

JSON output is byte-stable; pipe-friendly:

```sh
tome workspace info --json | jq -r '.path // "global"'
```

## 7. Two MCP servers, two workspaces, one machine

You can run multiple MCP servers concurrently against different workspaces. Each is its own process; they share the model artefacts but have independent index databases.

In Claude Code:

```json
{
  "mcpServers": {
    "tome-project-a": {
      "command": "tome",
      "args": ["mcp", "--workspace", "/abs/projects/a"]
    },
    "tome-project-b": {
      "command": "tome",
      "args": ["mcp", "--workspace", "/abs/projects/b"]
    }
  }
}
```

Each agent session sees the union of tools from both servers; queries to `tome-project-a` only see project A's enabled plugins.

## Troubleshooting

| Symptom | First check |
|---|---|
| MCP server dies immediately on launch | `tail ~/.local/state/tome/mcp.log` for the startup error; check `tome doctor` |
| `tome workspace info` reports the wrong workspace | Run with `-vv`; debug log includes the resolution source |
| A `tome plugin enable` wrote to the wrong scope | Same — the debug log line `scope resolved scope=… source=…` is the source of truth |
| Catalog operations succeed but no on-disk change | You're likely operating against the global registry from inside a workspace; pass `--global` deliberately or check `tome workspace info` |
| `tome doctor` reports an orphan catalog clone | Either re-add the catalog to a scope, or `rm -rf <path>` the cache dir. Orphans are safe to remove |
| `tome --version` doesn't include model identities | Should: rebuild Tome at HEAD of Phase 3 |

## Common commands

```sh
# Workspaces
tome workspace init [<path>] [--inherit-global] [--force]
tome workspace info [--json]

# MCP
tome mcp [--workspace <path> | --global]

# Doctor
tome doctor [--fix] [--json]

# Global flags (work on every command)
tome --workspace <path> <subcommand> …
tome --global <subcommand> …
```

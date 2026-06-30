---
title: Meta skills
sidebar_position: 5
---

# Meta skills

Most skills teach your agent about your domain. **Meta skills** teach it about
Tome itself. They are bundled inside the `tome` binary — versioned with it,
with no download — and install as native `SKILL.md` files into your harness,
so the agent reads them the same way it reads any other skill.

## List available meta skills

```bash
tome meta list
```

```text
convert-marketplace — Guided conversion of a Claude Code marketplace into Tome's native plugin format. Drives `tome convert` and `tome lint` for the mechanical work, applies judgment to the parts Tome cannot represent, verifies the result, then reports to you and waits for explicit confirmation before registering anything in a workspace.  [1a57b1e8df28caf7]
    claude-code/global: not-installed
    claude-code/project: not-installed
    codex/global: not-installed
    codex/project: not-installed
    cursor/global: not-installed
    cursor/project: not-installed
    opencode/global: not-installed
    opencode/project: not-installed
```

Each bundled skill appears with its revision (the hash in brackets) and an
install state for every harness × scope pair Tome can target.

:::note
Gemini CLI has no native skill support, so it is not a meta-skill target —
which is why it doesn't appear in the list above.
:::

## Install one

```bash
tome meta add convert-marketplace
```

By default this writes the skill into the **project** skill directory of
**every detected harness** — run it once from your project root and every
agent you use there can read it. Removal works the same way:

```bash
tome meta remove convert-marketplace
```

### Choose the target

| Flag | Effect |
| --- | --- |
| `--harness <name>` | Target one harness instead of all detected ones. Repeatable. |
| `--global` | Install into the harness's *global* skill directory instead of the project one. |
| `--force` | Overwrite an existing install (including one that's been edited on disk). `add` only. |

`--harness` and `--global` work the same way on `remove`.

## The `convert-marketplace` skill

This is the bundled skill to install first. It guides your agent through
converting a Claude Code marketplace into Tome's native format: it drives
`tome catalog convert` and `tome catalog lint` for the mechanical work,
applies judgment to the parts Tome cannot represent, verifies the result —
then **reports to you and waits for explicit confirmation** before registering
anything in a workspace. Your agent does the conversion; you give the final
approval.

The manual version of that workflow lives in
[Converting](../authoring/convert.md).

## Keep installs up to date

Installed copies can drift from the binary: a Tome upgrade ships a newer
revision (**stale**), a file gets deleted (**missing**), or someone edits the
installed copy (**modified**). `tome doctor` reports all three states for
every installed meta skill, and

```bash
tome doctor --fix
```

re-installs from the bundled copy.

## Pitfalls

- **Exit `87` — unknown skill id.** The id you passed isn't a bundled meta
  skill. `tome meta list` shows which skills the binary includes.
- **Exit `88` — install failed.** Tome couldn't write the skill files — for
  example, it refuses to write through symlinked directories by design. The
  error says which target failed; installs into other harnesses still proceed.
- **Exit `89` — no harness detected.** Tome found no supported harness to
  install into. Pass `--harness <name>` explicitly, or run from a directory
  where a supported harness is set up.

The full table is in [Exit codes](../reference/exit-codes.md).

## Where next

- [MCP server](./mcp-server.md) — agents can install meta skills themselves,
  via the `meta` tool or the built-in `add-tome-conversion-skill` prompt.
- [Converting](../authoring/convert.md) — the commands the guided conversion
  runs.

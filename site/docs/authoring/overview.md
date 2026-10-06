---
title: Authoring overview
sidebar_position: 1
---

# Authoring

This page covers writing your own skills, plugins, and catalogs.

The previous sections covered using other people's work: add a catalog,
enable a plugin, and let your agents use the entries. Sooner or later you have
knowledge of your own — a debugging workflow, a deploy runbook, a style guide —
and pasting it into five harness configs is exactly the problem Tome solves.
Write it once, publish it as a catalog, and every agent you use can load it.

This page describes the files you will write. The three commands that produce
them — [`create`](./create.md), [`convert`](./convert.md), and
[`lint`](./lint.md) — each have their own page, and
[Distributing](./distributing.md) covers how to share the result.

## Catalog structure

A catalog is just a **git repository of plugins** with one manifest at its root:

```
your-catalog/
├── tome-catalog.toml          # the catalog manifest (required, at the root)
└── <plugin>/
    ├── tome-plugin.toml       # the plugin manifest — what Tome reads (strict)
    ├── .claude-plugin/
    │   └── plugin.json        # legacy (converted by `tome plugin convert`)
    ├── skills/
    │   └── <name>/SKILL.md    # one directory per skill
    ├── commands/
    │   └── <name>.md          # one file per command
    ├── agents/
    │   └── <name>.md          # one file per agent
    └── hooks/
        └── hooks.json         # event-driven actions (one JSON file)
```

The layout is based on Claude Code's plugin format, which is why converting
an existing Claude Code plugin is mostly mechanical — but Tome reads its own
manifest. A plugin directory that contains only the legacy
`.claude-plugin/plugin.json` is not yet a Tome plugin: commands that read it
exit `80` (`plugin_not_converted`) until [`tome plugin convert`](./convert.md)
writes the native manifest alongside it.

## `tome-catalog.toml`

The catalog manifest lives at the repo root and lists the plugins the catalog
ships. It is parsed strictly — unknown fields are rejected.

```toml
name = "my-catalog"
description = "A short description of what this catalog provides."
version = "0.1.0"

[owner]
name = "Your Name"
email = "you@example.com"
```

Each `[[plugins]]` entry names a plugin and points `source` at its directory
within the repo:

```toml
[[plugins]]
name = "my-plugin"
source = "my-plugin"
```

A handful of optional fields add discoverability and provenance metadata,
mirrored on `tome-plugin.toml` below:

| Field | Type | Purpose |
| --- | --- | --- |
| `keywords` | array of strings | Feeds keyword search (`tome query --keyword`, MCP `search_skills` `keywords`). Max 20 entries, 40 chars each. |
| `homepage` | string | Project homepage URL. |
| `repository` | string | Source repository URL. |
| `bugs` | string | Issue tracker URL. |
| `documentation` | string | Documentation URL. |
| `display_name` | string | Human-friendly name shown in place of `name` where space allows. |
| `icon` | string | Path to an icon file, relative to the manifest. |

`homepage`, `repository`, `bugs`, and `documentation` must be `http://` or
`https://` URLs. `icon` must be a relative path (no URL, no `..` traversal, no
absolute path).

`[owner]` also accepts an optional `url` and an open `[owner.handles]` table
mapping platform name to handle:

```toml
[owner]
name = "Your Name"
email = "you@example.com"
url = "https://example.com"

[owner.handles]
github = "your-handle"
bluesky = "you.bsky.social"
```

## `tome-plugin.toml`

Each plugin has a manifest at its root: `<plugin>/tome-plugin.toml`. Like the
catalog manifest, it is parsed strictly — a misspelled or unknown field is a
parse error, not a silently ignored value.

```toml
name = "my-plugin"
version = "0.1.0"
description = "What this plugin does, and when an agent should reach for it."
license = "MIT"

[author]
name = "Your Name"
email = "you@example.com"
```

You rarely write this file by hand: [`create`](./create.md) scaffolds it and
[`convert`](./convert.md) derives it from a legacy `plugin.json`.

The same optional metadata fields as `tome-catalog.toml` are accepted —
`keywords`, `homepage`, `repository`, `bugs`, `documentation`, `display_name`,
`icon` — with the same types and validation notes. `[author]` gains an
optional `url` and an open `[author.handles]` table (platform → handle), and
the plugin may also list `[[contributors]]`, each entry shaped exactly like
`[author]`:

```toml
[author]
name = "Your Name"
email = "you@example.com"
url = "https://example.com"

[author.handles]
github = "your-handle"

[[contributors]]
name = "A. Nother"
handles = { github = "a-nother" }
```

## Entries

A plugin contributes four kinds of
[entry](../getting-started/concepts.md#entry-kinds):

- **Skills** — `skills/<name>/SKILL.md`. A `SKILL.md` is a Markdown file with a
  YAML frontmatter header. A `when_to_use` field in the frontmatter is indexed
  to improve [semantic search](../using-tome/search.md), so write it well.
- **Commands** — `commands/<name>.md`. Exposed as slash commands and MCP
  prompts.
- **Agents** — `agents/<name>.md`. Translated to each harness's native agent
  format where supported.
- **Hooks** — `hooks/hooks.json`. A single JSON file describing event-driven
  actions, wired in where the harness supports them.

## The three commands

Three commands produce all of the above. [`create`](./create.md) scaffolds a
new skill, plugin, or catalog from a built-in template — lint-clean by
construction, so you start from a passing state and keep it passing as you
edit. [`convert`](./convert.md) migrates what you already have: Claude Code
marketplaces, plugins, and skills, Codex projects, and native `SKILL.md` trees
from other harnesses. [`lint`](./lint.md) validates the result — and never
halts on a malformed artifact, because malformed input is a finding, not a
crash. When the lint run is clean, [Distributing](./distributing.md) shows how
to publish the result with one `git push`.

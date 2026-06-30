---
title: Configuration
sidebar_position: 3
---

# Configuration

Tome keeps all of its state in one place. Every Tome-owned path lives under a
single root — `~/.tome/` — so you can inspect, back up, or remove Tome's state
by working with one directory. Project directories carry a small marker so Tome
knows which workspace they belong to.

## `~/.tome/` layout

```
~/.tome/
├── config.toml          # global configuration
├── settings.toml        # global settings layer (harness composition)
├── index.db             # the central SQLite search index
├── index.lock           # the index advisory lock
├── catalogs/            # cloned catalog content
├── models/              # downloaded embedding + rerank models
├── logs/                # logs
└── workspaces/
    └── <name>/
        ├── settings.toml   # workspace-layer settings
        └── RULES.md        # workspace-layer rules surface
```

There is exactly one central `index.db`, one `index.lock`, and one global
`config.toml`. Per-workspace state lives under `workspaces/<name>/`.

A few details worth knowing:

- **`index.lock`** is an advisory lock guarding index writes. Read-only
  commands — `tome status` in particular — never take it, so they are always
  safe to run while something else is indexing.
- **`catalogs/`** holds shared clones, reference-counted across everything
  that uses them. Adding the same catalog twice doesn't clone it twice, and a
  clone is deleted only when the last reference to it is removed.

## Global `config.toml`

`~/.tome/config.toml` holds global configuration. Tome bootstraps the `~/.tome/`
root on first write. Tome-owned config is parsed strictly — unknown fields are
rejected — so a typo surfaces as an error rather than being silently ignored.

## Settings layers

Harness composition is resolved from layered settings:

- **Global** — `~/.tome/settings.toml`
- **Workspace** — `~/.tome/workspaces/<name>/settings.toml`

The workspace layer composes over the global layer to produce the configuration
written to each [harness](../using-tome/harnesses.md). Edits are made
surgically, preserving comments and key order — Tome never rewrites a settings
file wholesale.

## Project markers

A project bound to a workspace carries a marker directory at the project root:

```
<project>/.tome/
├── config.toml          # which workspace this project maps to
└── RULES.md             # a copy of the workspace-layer rules
```

This is how working inside a project activates the right
[workspace](../using-tome/workspaces.md) composition automatically.

## Models

Downloaded models live under `~/.tome/models/`, each with its own
`manifest.toml`. (Older installs that still have a `manifest.json` are migrated
in place by `tome doctor --fix` — no re-download.) Manage models with
`tome models {download,list,remove}` against a pinned registry. See the
[Commands reference](./commands.md#tome-models).

---
title: Workspaces
sidebar_position: 4
---

# Workspaces

Your contracts project needs the verification skills; your web app does not.
A **workspace** is a per-project scope: each one enables its own catalogs and
plugins, so the composition that is active for one project never appears in
another project's context.

## Why workspaces

Without workspaces, every plugin you enable is enabled everywhere. Workspaces
let you keep a focused set per project — a Compact contract project might
enable the Midnight Expert verification plugin, while another project enables
a different set entirely.

## Two projects, two compositions

```bash
# the contracts project uses the verification plugin
# `use --create` creates the workspace and binds this directory in one step
tome workspace use --create contracts
tome plugin enable midnight-expert/midnight-verify

# the dapp project uses a different plugin
tome workspace use --create dapp
tome plugin enable midnight-expert/midnight-dapp-dev
```

Each `plugin enable` is recorded against the *active* workspace. Switch back
to `contracts` and `midnight-dapp-dev` is no longer part of what your agent
sees; `midnight-verify` is. To check the current state:

```bash
tome workspace info   # the active workspace and its composition
```

## Lifecycle

```bash
tome workspace init <name>          # create a workspace
tome workspace init --bind <name>   # create a workspace and bind this directory
tome workspace use <name>           # bind this directory to an existing workspace
tome workspace use --create <name>  # create-if-absent, then bind — one step
tome workspace use                  # pick a workspace to bind from a list
tome workspace list                 # list workspaces
tome workspace info                 # show the active workspace and its composition
tome workspace rename <a> <b>       # rename a workspace
tome workspace remove <name>        # remove a workspace
```

`tome workspace use --create <name>` and `tome workspace init --bind <name>` are
mirrors of each other: both create the workspace (create-if-absent for `use
--create`) and bind the current directory in a single step. Run `tome workspace
use` with no name on a terminal to choose from a picker.

## Project binding

A workspace can be **bound** to one or more project directories, so the right
composition activates automatically when you work in that project. Catalog and
plugin enablement is recorded per workspace as the source of truth, rather than
globally.

## Composition

The set of catalogs and plugins enabled in a workspace is its **composition**.
When you run `tome harness use <name>`, Tome resolves the active workspace's
composition and writes native config for exactly that set. Switching workspaces
changes what your harnesses see.

## Summaries and sync

```bash
tome workspace regen-summary <name>  # regenerate a named workspace's summary
tome workspace regen-summary         # regenerate the active workspace (confirms first)
tome workspace sync                  # reconcile the workspace with current state
```

`tome workspace regen-summary` with no name defaults to the active workspace but
asks for confirmation first, so you never accidentally regenerate the resolved
(often `global`) scope. On a non-terminal the name is required.

If a workspace looks out of sync, `tome doctor` reports it and `tome doctor --fix`
re-runs the relevant reconciliation. See
[Troubleshooting](./troubleshooting.md).

## Where next

- [Harnesses](./harnesses.md) — how a workspace's composition is written to
  each agent's native config.
- [Plugins & catalogs](./plugins-and-catalogs.md) — the enable/disable
  lifecycle that workspaces scope.

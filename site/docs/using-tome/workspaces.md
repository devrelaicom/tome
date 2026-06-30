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
tome workspace init contracts
tome workspace use contracts
tome plugin enable midnight-expert/midnight-verify

# the dapp project uses a different plugin
tome workspace init dapp
tome workspace use dapp
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
tome workspace init <name>     # create a workspace
tome workspace use <name>      # switch the active workspace
tome workspace list            # list workspaces
tome workspace info            # show the active workspace and its composition
tome workspace rename <a> <b>  # rename a workspace
tome workspace remove <name>   # remove a workspace
```

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
tome workspace regen-summary   # regenerate the workspace summary
tome workspace sync            # reconcile the workspace with current state
```

If a workspace looks out of sync, `tome doctor` reports it and `tome doctor --fix`
re-runs the relevant reconciliation. See
[Troubleshooting](./troubleshooting.md).

## Where next

- [Harnesses](./harnesses.md) — how a workspace's composition is written to
  each agent's native config.
- [Plugins & catalogs](./plugins-and-catalogs.md) — the enable/disable
  lifecycle that workspaces scope.

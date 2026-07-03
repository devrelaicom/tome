---
title: Plugins & catalogs
sidebar_position: 1
---

# Plugins & catalogs

A **catalog** is a git repository of plugins that you add once. Tome clones
and indexes it. A **plugin** is a bundle of skills, commands, agents, and
hooks inside a catalog. You enable the plugins you want. Day-to-day, Tome is
mostly these two actions — add catalogs, enable plugins — and everything else
(search, harness config, the MCP server) follows from what you have enabled.

## Add a catalog

```bash
tome catalog add devrelaicom/midnight-expert-tome
```

Tome clones the repository, parses every plugin manifest it finds, and
registers the catalog under a short name — here `midnight-expert`, thirteen
plugins in a single add. The output reports the resolved HEAD commit: the short
SHA in human output, the full 40-char SHA in `--json` under `added.commit`.

The source can be an `owner/repo` shorthand (GitHub), a git URL, or a local
path. The shorthand also takes a forge prefix, so GitLab and Bitbucket get the
same ergonomics as GitHub instead of being read as a local path:

```bash
tome catalog add gh:owner/repo          # GitHub (same as the bare owner/repo)
tome catalog add gl:owner/repo          # GitLab
tome catalog add bb:owner/repo          # Bitbucket
```

Pin a ref with `--ref`, or its aliases `--branch` and `--tag` (a branch, tag, or
SHA; default `main`), and override the display name with `--name`, short `-n`:

```bash
tome catalog add gh:owner/repo --tag v1.2.0 -n my-catalog
```

Inspect what was added:

```bash
tome catalog show midnight-expert
```

```text
midnight-expert-tome (v0.46.0)
  Converted from the midnight-expert Claude Code marketplace
  Owner: Aaron Bassett <aaron@devrel-ai.com>

Plugins:
  compact-cli-dev        compact-cli-dev
  compact-core           compact-core
  compact-examples       compact-examples
  core-concepts          core-concepts
  midnight-cq            midnight-cq
  midnight-dapp-dev      midnight-dapp-dev
  midnight-expert        midnight-expert
  midnight-fact-check    midnight-fact-check
  midnight-plugin-utils  midnight-plugin-utils
  midnight-status-codes  midnight-status-codes
  midnight-tooling       midnight-tooling
  midnight-verify        midnight-verify
  midnight-wallet        midnight-wallet
```

(Output abridged — the full listing also shows the catalog's source URL and
last sync time.)

Adding a catalog enables nothing by itself. The thirteen plugins are now
available, but none of them is enabled, and none of them takes up space in
any context window.

## Enable a plugin

```bash
tome plugin enable midnight-verify
```

Enabling parses the plugin's entries — skills, commands, agents — and indexes
them for [search](./search.md). Check the current state:

```bash
tome plugin list
```

```text
| Catalog         | Plugin                | Version | Status     | Entries                           | Last indexed | Last upstream change |
|-----------------|-----------------------|---------|------------|-----------------------------------|--------------|----------------------|
| midnight-expert | compact-cli-dev       | 0.4.0   | ✗ disabled | —                                 | —            | —                    |
| midnight-expert | compact-core          | 0.10.0  | ✗ disabled | —                                 | —            | —                    |
| midnight-expert | compact-examples      | 0.4.0   | ✗ disabled | —                                 | —            | —                    |
| midnight-expert | core-concepts         | 0.3.2   | ✗ disabled | —                                 | —            | —                    |
| midnight-expert | midnight-cq           | 0.4.0   | ✗ disabled | —                                 | —            | —                    |
| midnight-expert | midnight-dapp-dev     | 0.5.0   | ✗ disabled | —                                 | —            | —                    |
| midnight-expert | midnight-expert       | 0.6.0   | ✗ disabled | —                                 | —            | —                    |
| midnight-expert | midnight-fact-check   | 0.4.0   | ✗ disabled | —                                 | —            | —                    |
| midnight-expert | midnight-plugin-utils | 0.1.1   | ✗ disabled | —                                 | —            | —                    |
| midnight-expert | midnight-status-codes | 0.9.0   | ✗ disabled | —                                 | —            | —                    |
| midnight-expert | midnight-tooling      | 0.4.0   | ✗ disabled | —                                 | —            | —                    |
| midnight-expert | midnight-verify       | 0.13.0  | ✓ enabled  | (19 skills, 2 commands, 7 agents) | just now     | just now             |
| midnight-expert | midnight-wallet       | 0.5.0   | ✗ disabled | —                                 | —            | —                    |
```

One plugin is enabled: 19 skills, 2 commands, and 7 agents — 28 entries
indexed and searchable. The other twelve plugins stay disabled and cost
nothing until you enable them.

### Variations

- `tome plugin show <name>` prints one plugin's entries, grouped by kind with
  per-entry annotations — useful before deciding whether to enable it.
- `tome plugin disable <name>` reverses this: the plugin's entries are removed
  from the index, and your harnesses drop them on the next sync.
- Enable or disable **several plugins at once**, and use `*` wildcards to match
  by pattern:

  ```bash
  # Two plugins in one call
  tome plugin enable midnight-expert/compact-core midnight-expert/midnight-verify

  # Every plugin whose name starts with `compact-`
  tome plugin enable "midnight-expert/compact-*"

  # Every plugin in a catalog
  tome plugin enable "midnight-expert/*"
  ```

  `--catalog <name>` scopes bare or wildcard names to one catalog, so you can
  drop the `<catalog>/` prefix (`tome plugin enable "compact-*" --catalog
  midnight-expert`). A wildcard that matches nothing is an error rather than a
  silent no-op, and a batch that hits a bad id still processes the good ones and
  reports the failure at the end. The same applies to `tome plugin disable`,
  which asks for a single confirmation naming every plugin in the batch.
- `enable` and `disable` update the index only. Without `--sync` they print a
  reminder to run `tome sync`, so your harnesses drop or pick up the entries on
  the next sync. Pass `--sync` to apply the change to your harnesses inline: it
  runs the same propagation as `tome sync` over every bound project.
- `tome plugin list` can be narrowed. `--filter <substr>` keeps only plugins
  whose name or description contains the (case-insensitive) substring; `--tier
  <1|2|3>` keeps only plugins with at least one enabled entry at that
  [routing tier](../getting-started/concepts.md#routing-tier). Both compose with
  each other and with `--catalog`. `tome plugin show <name> --details` annotates
  each entry line with its routing tier.
- Bare `tome plugin` opens an interactive picker — catalog → plugin → action —
  useful when you want to pick from a list instead of typing names.

## Update catalogs

```bash
tome catalog update midnight-expert
```

This pulls the catalog's source repository and re-indexes it. Run it when the
catalog publishes new plugins or updated skills; your enabled plugins receive
the changes.

## Remove a catalog

```bash
tome catalog remove midnight-expert
```

If the catalog still has enabled plugins, removal fails (exit `53`), because
your harnesses still use those entries. Pass `--force` to cascade:
every enabled plugin is disabled first, then the catalog is removed.

## Catalogs to add today

**Midnight Expert** is the main catalog from the Midnight Foundation: a
collection of AI plugins for building, testing, and verifying
[Compact](https://docs.midnight.network) smart contracts on
[Midnight](https://midnight.network) — privacy patterns, formal verification,
devnet tooling, and more, available across every harness Tome supports.

```bash
tome catalog add devrelaicom/midnight-expert-tome
tome plugin enable midnight-expert
```

The catalog bundles **13 plugins** covering the Compact development lifecycle,
including:

- Compact language and standard-library guidance
- Privacy and disclosure patterns
- Formal verification and code review
- dApp and SDK development
- Local devnet and tooling

The verification plugin alone — `midnight-verify` — contains **19 skills,
2 commands, and 7 agents**: 28 searchable entries from a single
`plugin enable`.

Once the plugin is enabled, configure a harness:

```bash
tome harness use claude-code
```

and search for what you need:

```bash
tome query "verify a Compact contract"
```

See [Quickstart](../getting-started/quickstart.md) for the full flow and
[Search](./search.md) for how retrieval works.

## Pitfalls

| Exit code | What happened | What to do |
| --- | --- | --- |
| `4` | Catalog already added. | Use `tome catalog update <name>` to refresh it instead of re-adding. |
| `21` | Plugin already in the requested state. | Nothing — it was already enabled (or disabled). Check `tome plugin list`. |
| `53` | Catalog removal refused: it still has enabled plugins. | Disable them, or re-run `tome catalog remove <name> --force` to cascade. |
| `80` | Plugin not converted: a legacy `.claude-plugin/plugin.json` exists but no `tome-plugin.toml`. | Convert it — see [Converting](../authoring/convert.md). |

The full table is in the [exit codes reference](../reference/exit-codes.md).

## Where next

- [Search](./search.md) — find the right entry by meaning, not by name.
- [Harnesses](./harnesses.md) — write your enabled plugins to each agent's
  native config.
- [Converting](../authoring/convert.md) — already have plugins in another
  format? Convert them instead of rewriting them.

---
title: Creating
sidebar_position: 2
---

# Creating

Starting from a blank `SKILL.md` makes two mistakes easy: frontmatter the
parser rejects and structure the linter flags. `create` scaffolds skills,
plugins, and whole catalogs that are lint-clean by construction — you start
from a passing state and keep it passing as you edit.

## Scaffold a skill

The fastest start is a bare skill — one directory, one `SKILL.md`, no plugin
wrapper:

```console
$ tome skill create demo-skill --bare --output /tmp/tome-cap
Created skill `demo-skill` at /private/tmp/tome-cap/demo-skill
  SKILL.md
```

(macOS resolves `/tmp` to `/private/tmp` — the same location.) The command
created:

```console
$ ls /tmp/tome-cap
demo-skill
```

Open `demo-skill/SKILL.md`, fill in the frontmatter, and write the body. The
`when_to_use` frontmatter field is indexed for
[semantic search](../using-tome/search.md), so write it carefully — it is
how agents find the skill later.

A fresh scaffold lints clean, but its defaults are generic placeholders you
will usually want to replace. Without `--description`, the skill gets a
name-derived default (`The demo-skill scaffold.`); set a real one with
`--description "<text>"`. The description feeds
[semantic search](../using-tome/search.md), so a real one is how agents find
the skill later. To see the file list before writing anything, add `--dry-run`.

## Scaffold a plugin or a catalog

The same command works for plugins and catalogs:

```bash
tome plugin create my-plugin
tome catalog create my-catalog
```

`plugin create` scaffolds the plugin directory with its strict
[`tome-plugin.toml`](./overview.md#tome-plugintoml) manifest; `catalog create`
scaffolds a catalog directory with `tome-catalog.toml` at the root, ready
for plugins.

`--description` and `--author` apply here too. For a catalog, `--author` sets
the `[owner] name`; omit it and the manifest carries a `Your Name` placeholder
to edit before you publish. For a plugin, `--author` records the
`[author] name`; omit it and no `[author]` table is written. An empty or
whitespace-only `--author` is treated as absent, byte-identical to omitting the
flag, so it never writes an empty `[author]` `name = ""`.

## Flags

| Flag | Applies to | Effect |
| --- | --- | --- |
| `--bare` | `skill create` | Create only `<name>/SKILL.md`, with no plugin wrapper. |
| `--plugin-name <name>` | `skill create` | Wrap the new skill in a plugin with that name. |
| `--into <dir>` | `skill`, `plugin` | Scaffold into an existing parent — a skill into an existing plugin, a plugin into an existing catalog. Mutually exclusive with `--output`. |
| `--output <dir>` | all | The parent directory the new artifact is created in. Mutually exclusive with `--into`. |
| `--description <text>` | all | Set the manifest or skill description, replacing the generic name-derived default. Indexed for semantic search. |
| `--author <name>` | all | Set the catalog `[owner] name` or the plugin `[author] name`, replacing the `Your Name` placeholder you would otherwise edit by hand. An empty or whitespace-only value is treated as absent (no `[author]` table), so it never writes an empty `name = ""`. On `skill create --bare` or `--into` there is no wrapping plugin to record, so `--author` is silently ignored. |
| `--dry-run` | all | Preview the plan without writing. `--into` registration is skipped, and the report switches to `Would create…` (human) or `dry_run: true` (`--json`). |
| `--template <name>` | all | Select a built-in template. Only built-in templates are supported — anything else exits `82`. |
| `--force` | all | Overwrite an existing target instead of refusing with exit `81`. |

## Pitfalls

- **Exit `81` (`output_exists`)** — the target path already exists, and
  `create` refuses to overwrite it. Pass `--force` to overwrite.
- **Exit `82` (`template_invalid`)** — `--template` currently accepts only
  built-in templates; passing a path or URL exits `82` rather than fetching.

The full table lives in [Exit codes](../reference/exit-codes.md).

## Next steps

- Validate what you wrote: [Linting](./lint.md).
- Already have plugins in another harness's format? [Convert](./convert.md)
  instead of retyping.
- Ready to share it: [Distributing](./distributing.md).

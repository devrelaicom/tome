---
title: Distributing
sidebar_position: 5
---

# Distributing

A catalog is a git repository, so distributing one is just publishing a repo.
No registry, no packaging step, no account to create: push it somewhere your
users can clone, and `tome catalog add` does the rest — they add your
catalog once and use it from every harness they run.

## Before you tag

Two habits protect your users from the common failure modes:

1. **Commit the native manifests.** Ship `tome-plugin.toml` in every plugin
   directory, not just a legacy `.claude-plugin/plugin.json`. Tome reads only
   the native manifest — a legacy-only plugin causes exit `80`
   (`plugin_not_converted`) for every user. Run
   [`tome plugin convert`](./convert.md) yourself, once, and commit the
   result so they never see it.
2. **Lint in CI before tagging.** Run `tome catalog lint . --strict` as a
   release gate: errors exit `85`, warnings under `--strict` exit `86`, clean
   exits `0`. See [Linting](./lint.md) for the recipe.

## Publish it

1. Push your catalog repo to a host (for example, GitHub).
2. Tell people to add it:

   ```bash
   tome catalog add <your/repo>
   tome plugin enable <plugin-name>
   ```

That is the entire process — any git repository of plugins becomes a Tome
catalog, usable across Claude Code, Cursor, Codex, Gemini CLI, and OpenCode.
For what your users see on their side, read
[Plugins & catalogs](../using-tome/plugins-and-catalogs.md).

## A note on trust

Adding a catalog runs its instructions inside the user's agent. Tell your
users that adding your catalog means trusting your content — the same trust
they give `npm install` or any script they run. See the
[Security model](../reference/security-model.md) for the boundary Tome does
and does not defend.

## Pitfalls

- **Exit `80` (`plugin_not_converted`)** — a plugin in your repo contains only
  the legacy `plugin.json`. Convert and commit before publishing (above).
- **Exit `5` (`manifest_invalid`)** — `tome-catalog.toml` is parsed strictly;
  an unknown or misspelled field rejects the whole catalog at `add` time.
- **Exits `85`/`86`** — these are lint *verdicts*, not crashes: your CI gate
  working as intended. See [Linting](./lint.md) and
  [Exit codes](../reference/exit-codes.md).

## Next steps

- Tighten the release gate: [Linting](./lint.md).
- Migrating an existing marketplace into the repo you're about to publish:
  [Converting](./convert.md).

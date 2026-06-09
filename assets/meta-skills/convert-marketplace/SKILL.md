---
name: convert-marketplace
description: Guided conversion of a Claude Code marketplace into Tome's native plugin format. Drives `tome convert` and `tome lint` for the mechanical work, applies judgment to the parts Tome cannot represent, verifies the result, then reports to you and waits for explicit confirmation before registering anything in a workspace.
tome_prompt_name: add-tome-conversion-skill
---

# Convert a Claude Code marketplace to Tome

You are converting a Claude Code marketplace (a collection of plugins, each with
skills / commands / agents) into Tome's native format so the plugins work across
every harness Tome supports. Tome does the mechanical 80% — format translation,
harness-variable rewriting, manifest synthesis — through `tome convert` and
`tome lint`. Your job is the judgment 20%: the components Tome cannot represent,
the verification, and — above all — **stopping to report and get the user's
explicit confirmation before you register anything in a workspace.**

Work through the steps in order. Do not skip the report-and-confirm gate (Step 5)
even if everything converted cleanly with nothing left over.

## Step 1 — Inventory first, change nothing

Before touching anything, build a complete picture of what the source contains
and what Tome can and cannot carry over.

- Run the conversion in **dry-run** mode and read the plan:
  ```sh
  tome catalog convert <source> --dry-run --json
  ```
  `<source>` is the marketplace directory. (For a single plugin or skill rather
  than a whole marketplace, use `tome plugin convert` / `tome skill convert`
  instead.) The JSON plan enumerates every plugin and entry that will convert,
  and — critically —
  every **unsupported component** Tome flagged (monitors, themes, LSP servers,
  output-styles, status-line scripts, hooks Tome can't model, `bin/` helpers,
  channels, custom `userConfig`, and so on).
- Summarise for yourself: how many plugins, how many skills / commands / agents
  convert cleanly, and the full list of flagged unsupported components. This list
  is your Step 3 work queue — nothing leaves it without an explicit decision.

Do not run a non-dry-run convert yet.

## Step 2 — Mechanical conversion

Now run the real conversion — it lands a **fresh catalog directory** on disk (the
source is never modified) — and read what happened:

```sh
tome catalog convert <source> --output <parent-dir> --json
```

`--output` is the parent directory the new catalog lands under; the catalog itself
is created at `<parent-dir>/<name>/` (call that produced directory `<catalog>` in
the steps below). `--output` defaults to the current directory if omitted.

From the `--json` output, separate two buckets:

- **Auto-converted** — entries Tome translated for you: native `SKILL.md` /
  command / agent files written, `tome-plugin.toml` / `tome-catalog.toml`
  manifests synthesised, and harness-specific variables rewritten to their Tome
  equivalents automatically. You do not need to touch these.
- **Flagged** — the unsupported components from Step 1, which convert leaves
  behind with a warning rather than guessing. These are Step 3.

If you want Tome to refuse rather than warn on anything it cannot represent, add
`--strict` — it aborts before writing so you can decide deliberately. Without
`--strict`, convert writes the supported parts and warns about the rest.

## Step 3 — Judgment pass on the unsupported residue

For **each** flagged component, make and **write down** an explicit decision —
one of: **drop**, **hand-port**, or **document**. Consult
[`references/unsupported-component-rubric.md`](references/unsupported-component-rubric.md)
for a per-component decision aid. Briefly:

- **Drop** — the component has no Tome-native equivalent and no behavioural value
  worth preserving (e.g. a harness-only theme or output-style). Record that it was
  intentionally dropped.
- **Hand-port** — the behaviour matters and Tome has a native way to express it
  (e.g. a guardrail or a piece of agent guidance). Re-author it in the native
  form and note what you did.
- **Document** — the behaviour matters but Tome cannot represent it yet. Leave a
  clear note (in the plugin's docs or a `KNOWN-GAPS` note) so the user knows the
  gap exists.

Keep a running record of every decision — you will report all of them in Step 5.

## Step 4 — Verify

Lint the converted catalog and resolve everything until it is clean (`lint` is a
subcommand of `catalog`, not a top-level verb):

```sh
tome catalog lint <catalog> --json
```

- Fix any **errors** (these fail CI). Re-run until there are none.
- Resolve **strict warnings** (`tome catalog lint <catalog> --strict`) where they matter.
- For mechanical, safe fixes (residual harness-isms, name/dir mismatches), you can
  apply them automatically:
  ```sh
  tome catalog lint <catalog> --autofix
  ```
  Then re-lint to confirm. Never autofix blindly over a judgment call from Step 3 —
  review the diff.

Iterate Step 3 ↔ Step 4 until `tome catalog lint` is clean (or only carries
intentional, documented gaps).

## Step 5 — Report, then STOP and wait for confirmation

This gate is **unconditional**. It fires even when the conversion was perfectly
clean and there were zero unsupported components. **Register nothing yet.**

Report to the user, plainly:

- the plugins and entries that converted, and where the catalog landed on disk;
- what auto-converted vs. what you hand-touched;
- **every** judgment decision from Step 3 (drop / hand-port / document), with the
  reason;
- the final `tome lint` state and any documented gaps that remain.

Then ask the user, in your own words, whether they want you to **add the catalog
and enable its plugins in a workspace** — and wait for an explicit answer.

- If they **decline** (or don't answer): you are done. The converted artifacts
  stay on disk, fully usable, and **nothing is registered**. Do not proceed.
- If they **confirm**: continue to Step 6.

## Step 6 — Confirmed registration

Only after explicit confirmation. First ask whether they want a **new** workspace
or an **existing** one.

**New workspace:**
```sh
tome workspace init <name>
tome workspace use <name>
tome catalog add <catalog>
tome plugin enable <catalog>/<plugin>     # repeat per plugin they want
tome plugin show <catalog>/<plugin>       # confirm it resolved
```

**Existing workspace:**
```sh
tome workspace list                       # let the user pick
tome workspace use <chosen>
tome catalog add <catalog>
tome plugin enable <catalog>/<plugin>
tome plugin show <catalog>/<plugin>       # confirm
```

When running non-interactively (e.g. in CI, or before the embedding models are
downloaded), add `--yes` to `tome plugin enable` to skip its model-download
confirmation prompt.

Finish by confirming to the user what is now registered and enabled, and restate
any documented gaps from Step 3 so nothing is silently lost.

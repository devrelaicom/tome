# Harness Commands — Contract

**Spec source**: [spec.md FR-520 through FR-525](../spec.md)

Six harness-management commands. All honour `--json` for structured output.

## `tome harness` (bare)

Lists every supported harness in tabular form, regardless of effective list or workspace state.

**Output columns**: name; detected on this system (yes/no, derived from per-user dir existence check on `<home>/.{claude,codex,gemini,cursor,opencode}/`); rules-file target for the current project (or `—` if no project resolved); MCP config target (or `—`).

**Failure**: never; this is read-only inspection.

## `tome harness list [<workspace>]`

Reports a harness list. Two modes:

- **No argument**: reports the *effective* harness list for the current project, computed via the layered settings walk + composition expansion (see [settings-composition.md](./settings-composition.md)). Each entry is annotated with the contributing scope (project / workspace / global) and the reference chain that brought it into the list. `!`-prefixed exclusions are reported in a separate "excluded" section so the developer sees what was subtracted.
- **`<workspace>` argument**: reports that workspace's *directly-declared* harness list verbatim (no composition expansion). Useful for inspecting one workspace's intent.

`--json`:

```json
{
  "mode": "effective",
  "harnesses": [
    {"name": "claude-code", "source_chain": ["project"]},
    {"name": "codex", "source_chain": ["project", "[workspaces.shared]"]}
  ],
  "excluded": ["cursor"]
}
```

```json
{"mode": "as_written", "workspace": "<name>", "harnesses": ["claude-code", "[global]"]}
```

## `tome harness use <name> [--scope project|workspace|global] [--force]`

Adds a harness to a scope's settings file and runs the integration logic if the effective list changed.

**Algorithm**:

1. Validate `<name>` is one of the five supported harnesses — exit 18 otherwise.
2. Determine scope (default: `project`). For `project`: require a project marker. Exit 2 (usage) if no project marker found above CWD when `--scope project` is implicit and CWD is `<home>` or `/`.
3. Read the target settings file (project marker config / workspace settings / global settings) under the lockfile.
4. Append `<name>` to the `harnesses` array if not already present. Use the order-preserving editor (`toml_edit` for TOML, `serde_json` with `preserve_order` for JSON-formatted Tome-owned settings — though Tome-owned settings are TOML, so the editor is always `toml_edit` here).
5. Recompute the effective harness list for the current project (if any).
6. If the addition changed the effective list: run the sync algorithm for the newly-added harness(es).
7. If the addition did NOT change the effective list (e.g. `--scope global` but the project overrides without referencing global): print an informational notice naming the edited scope and reminding the developer that other projects may need an explicit `tome harness sync`.

`--force` overrides a harness-clash on the MCP config write (FR-502 → exit 19 without override).

## `tome harness remove <name> [--scope project|workspace|global]`

Mirror of `harness use`. Removes the name from the settings file at the given scope; recomputes the effective list; if removal changed it, tears down the integration in the current project for the removed harness(es).

The integration teardown is "remove the Tome block from the rules-file target (if no other harness still targets it) and remove the Tome entry from the MCP config (per FR-504)."

## `tome harness info <name>`

Reports per-harness details for the current project:

- Name; one-line description.
- Detected on system: yes/no, at which path.
- Rules-file target Tome would write to in this project.
- MCP config target Tome would write to.
- Currently integrated? (filesystem-derived: is the Tome block present? is the Tome entry in the MCP config?)
- Which of the three settings scopes reference this harness, and how (direct / via composition).

## `tome harness sync`

The reconciler. Runs from a project directory. Computes the effective harness list and reconciles the project's actual filesystem integration state to match.

Per FR-525, the command is **byte-for-byte idempotent**: when the current state matches the effective list, no file is rewritten (no `rename()` syscall on any output file).

Output reports added/removed/leave-alone per harness; `--json` exposes the same as a structured `changes` array.

See [sync-algorithm.md](./sync-algorithm.md) for the full algorithm.

## Permissions and TTY discipline

- Confirm prompts: none in v1; every destructive operation requires an explicit `--force` flag.
- Non-TTY context: every command is non-interactive by default. `--force` is not required for harness modifications because they are recoverable (the developer can re-run `harness use` / `harness remove`).
- Concurrency: every command that writes a settings file or a harness config file acquires the central DB's advisory lockfile for the duration of the read-modify-write. Two parallel `tome harness use` from two terminals serialise on the lock.

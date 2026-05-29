# Hooks Integration — Contract

**Spec source**: [spec.md FR-001 through FR-006](../spec.md), [research.md R-3, R-4](../research.md), [data-model.md §2](../data-model.md)

Real hooks are supported for **Claude Code only** (`hooks_strategy() == RealJson`; FR-001). Every other harness returns `GuardrailsOnly` and falls through to [guardrails.md](./guardrails.md) with no error. A Claude Code plugin shipping no `hooks/hooks.json` also falls through to guardrails for Claude Code with no error (FR-001).

## Source and target

| | Path | Notes |
|---|---|---|
| Source | `<plugin-root>/hooks/hooks.json` | Canonical Claude Code plugin location. Read leniently (NFR-010) on plugin enable and on `tome harness sync` for any project whose effective list includes `claude-code` (FR-001). |
| Target | `<project>/.claude/settings.local.json` under the `hooks` key | The **local, gitignored** settings file (FR-002). |

The target is `settings.local.json`, NOT `settings.json`. The committed `.claude/settings.json` is **never** written for this purpose (FR-002, SC-004): rewritten hooks carry machine-specific absolute paths (see below) and must not be committed. `hook_settings_path(project)` returns `Some(.claude/settings.local.json)` for `claude-code`, `None` otherwise.

If `settings.local.json` is absent, Tome creates it with a single `hooks` object (`{"hooks": {}}`) before merging (FR-002, SC-004). Its parent `.claude/` is created with mode 0700 on Unix if missing (Phase 4 discipline).

## Path-variable rewriting (FR-003, R-4)

At copy time, a **targeted two-variable textual substitution** runs over the string leaves of the hook JSON. This is **NOT** the Phase 5 substitution pipeline — it is a small `regex` replace over string values only, so argument/built-in semantics never leak into a config file.

| Source variable | Rewritten to |
|---|---|
| `${CLAUDE_PLUGIN_ROOT}` | Absolute installed-plugin root (the Phase 5 `${TOME_PLUGIN_DIR}` value). |
| `${CLAUDE_PLUGIN_DATA}` | The Phase 5 `${TOME_PLUGIN_DATA}` value (`~/.tome/plugin-data/<catalog>/<plugin>/`). |

**All other `${CLAUDE_*}`** variables (e.g. `${CLAUDE_PROJECT_DIR}`, `${CLAUDE_SESSION_ID}`) are left **verbatim** — Claude Code resolves those natively at runtime. Substitution applies only within JSON string values; keys and non-string scalars are untouched.

### Before / after example

Source `hooks/hooks.json`:

```json
{
  "PreToolUse": [
    {
      "matcher": "Bash",
      "hooks": [
        { "type": "command", "command": "${CLAUDE_PLUGIN_ROOT}/scripts/guard.sh --root ${CLAUDE_PROJECT_DIR}" }
      ]
    }
  ]
}
```

After rewrite (with the plugin installed at `/Users/me/.tome/catalogs/<sha>/midnight-expert`):

```json
{
  "PreToolUse": [
    {
      "matcher": "Bash",
      "hooks": [
        { "type": "command", "command": "/Users/me/.tome/catalogs/<sha>/midnight-expert/scripts/guard.sh --root ${CLAUDE_PROJECT_DIR}" }
      ]
    }
  ]
}
```

`${CLAUDE_PLUGIN_ROOT}` is resolved; `${CLAUDE_PROJECT_DIR}` is left for Claude Code (SC-004).

## Merge semantics — add (FR-004, NFR-001)

Both files are parsed with `serde_json` (`preserve_order` feature, enabled since Phase 4) so the round-trip is diff-stable. For each rewritten plugin hook entry, keyed by its event:

1. Render the entry to its final post-rewrite form (a `serde_json::Value` object).
2. Look under the same event key in the existing `hooks` object.
3. If a **structurally identical** entry already exists — deep `serde_json::Value` equality of the entire post-rewrite entry object — **skip** it.
4. Otherwise, **append** it to that event's array.

This is idempotent across repeated syncs (NFR-001) and never duplicates a hook the user already authored by hand (FR-004, SC-005): a hand-authored entry that is deep-equal to the plugin's rewritten entry counts as already-present and is skipped.

## Removal semantics — sync (FR-005, NFR-003)

On disable, when `claude-code` leaves the project's effective list, or when the plugin leaves the workspace:

1. Re-derive the plugin's rewritten hook entries (identical rendering to the add path).
2. For each, find the deep-equal entry under its event key and remove it.
3. If no exact structural match exists, **skip** — never remove near-matches or user-edited copies.

Ownership is established **solely by re-derivation + structural match** — no sidecar, no provenance marker (FR-005, NFR-003). The deliberate consequence: a hook the user hand-edited after Tome wrote it no longer matches and is left in place. Tome never deletes a hook it cannot prove it owns (SC-005).

## Post-removal pruning (FR-006)

After removal, prune any event array that is now empty (delete the event key). An otherwise-empty `hooks` object is **left in place** (harmless) — Tome does not delete the `hooks` key or the `settings.local.json` file itself (FR-006).

## Atomic write

Every write to `settings.local.json` follows the Phase 4 atomic-write discipline:

1. Read the existing file (or start from `{"hooks": {}}` if absent).
2. Construct the merged/pruned `serde_json::Value` in memory.
3. **Refuse to write through a symlink** — `symlink_metadata` check on the target before writing; exit 44 (`HookSettingsWriteFailed`) if the target is a symlink — a write-guard on the dedicated settings sink, reconciled with the authoritative [exit-codes-p6.md](./exit-codes-p6.md) (matching the parallel guardrails-target → 46 decision; code 7 is reserved for IO that is *not* the local Claude settings file).
4. Serialise to a sibling temp file on the same filesystem; preserve the existing file's mode (capture via `symlink_metadata`, chmod the staged temp before persist; new files get 0600 on Unix); fsync; atomic rename onto the target.

A failure at any read/merge/write step surfaces **exit 44** (hook settings-file read/merge/write failure). The write is all-or-nothing: a failure never leaves a partially-written settings file (FR-084).

## Errors

| Exit code | Trigger |
|---|---|
| 43 | Plugin `hooks/hooks.json` is malformed or unparsable. The message names the file. Sibling components of the same plugin still reconcile where possible (loud-but-isolated parse handling). |
| 44 | Read / merge / write failure on `.claude/settings.local.json`. The message names the file. |

Both codes are pinned in [exit-codes-p6.md](./exit-codes-p6.md) (FR-092). Neither reuses an occupied code.

## Testing strategy

- **Idempotence by re-run**: merge, then re-run sync; the second run appends and removes nothing and rewrites no bytes (NFR-001, SC-005). Idempotence tests follow the `MTIME_TICK` capture/sleep/re-run pattern.
- **User-edit preservation**: write a plugin hook, hand-edit the entry, then disable; the edited entry no longer matches and is left in place (FR-005, SC-005).
- **Create-if-absent**: merge against a project with no `settings.local.json`; the file is created with a single `hooks` object and the rewritten entry; `settings.json` is never touched (FR-002, SC-004).
- **Structural-match dedup**: a hand-authored entry deep-equal to the plugin's rewritten entry is not duplicated on add (FR-004, SC-005).
- **Path rewrite**: `${CLAUDE_PLUGIN_ROOT}`/`${CLAUDE_PLUGIN_DATA}` resolved to absolute paths; `${CLAUDE_PROJECT_DIR}`/`${CLAUDE_SESSION_ID}` left verbatim (FR-003, SC-004).
- **Pruning**: removing a plugin's only hook for an event prunes the empty array but leaves the `hooks` object (FR-006).
- **Malformed source**: an unparsable `hooks.json` surfaces exit 43 and does not corrupt `settings.local.json`.
- **Symlink refusal**: a symlinked `settings.local.json` is refused (exit 44 — a write-guard on the dedicated settings sink, per exit-codes-p6.md).

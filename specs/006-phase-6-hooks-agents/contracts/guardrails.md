# Guardrails Soft Fallback — Contract

**Spec source**: [spec.md FR-010 through FR-016](../spec.md), [research.md R-5, R-13](../research.md), [data-model.md §3](../data-model.md)

Guardrails are the honest degradation path for pre/post-action constraints everywhere a real Claude Code JSON hook can't run (FR-001, [hooks-integration.md](./hooks-integration.md)). They are advisory by construction — the agent can ignore them — and never masquerade as enforcement.

## Source (FR-010)

An optional file at `<plugin-root>/hooks/GUARDRAILS.md`. Plain Markdown, no required frontmatter. The body is copied **verbatim** into the target; Tome **never parses or interprets** its content (FR-010, read leniently per NFR-010). A plugin without this file contributes no guardrails region.

## Marker region (FR-011, FR-011a)

Each enabled plugin shipping `GUARDRAILS.md` renders its content into a per-plugin region delimited by the literal marker pair:

```markdown
<!-- START GUARDRAILS: <catalog>:<plugin> -->
<contents of the plugin's GUARDRAILS.md, verbatim>
<!-- END GUARDRAILS: <catalog>:<plugin> -->
```

- These markers are **distinct** from the Phase 4 `<!-- tome:begin -->` / `<!-- tome:end -->` rules block — both managed regions coexist on the same file without collision (R-5).
- The `<catalog>:<plugin>` text carries provenance and is the **sole per-plugin removal key** (FR-011a) — guardrails state is filesystem-inferred from the marker pairs, with no sidecar (FR-015, NFR-004).
- "catalog" is Tome's term for what Claude Code calls a *marketplace*; the markers use Tome's vocabulary throughout.
- **Match regexes** (line-anchored, trailing whitespace tolerated):
  - `^<!-- START GUARDRAILS: (?P<catalog>[^:]+):(?P<plugin>.+) -->\s*$`
  - `^<!-- END GUARDRAILS: <catalog>:<plugin> -->\s*$`

## Per-harness targets (FR-012)

Targets reuse the Phase 4 harness modules. `guardrails_target(project)` returns the placement:

| Harness | Placement | Target file | Suppress if hooks present |
|---|---|---|---|
| `claude-code` | in-file region | `CLAUDE.md` (Phase 4 correction, §FR-020) | **yes** |
| `codex` | in-file region | `AGENTS.md` | no |
| `gemini` | in-file region | `AGENTS.md` (preferred) else `GEMINI.md` | no |
| `opencode` | in-file region | `AGENTS.md` | no |
| `cursor` | standalone sibling | `.cursor/rules/TOME_GUARDRAILS.md` | no |

The Cursor sibling is **distinct** from the Phase 4 skills sibling (`TOME_SKILLS.md`); each plugin is still individually marker-wrapped inside it so per-plugin removal works (FR-012). Codex, Gemini, and OpenCode share one `AGENTS.md`, so two of those harnesses in the same effective list contribute their regions to one shared file.

## Claude Code suppression (FR-013)

For Claude Code specifically: if a plugin ships **any** `hooks/hooks.json`, its guardrails region is **NOT** rendered into `CLAUDE.md` — the real JSON hooks supersede the prose fallback (FR-013, SC-006). Presence of the JSON file alone suppresses; no per-event merge of "JSON for some events, prose for others" is attempted.

This suppression applies **only to Claude Code's own file** (`CLAUDE.md`). The same plugin's guardrails CAN be present simultaneously in the shared `AGENTS.md` for the other harnesses — the two files are distinct, so there is no contradiction (FR-013, SC-006).

## Per-file reconciliation (FR-014)

Reconciliation is **per target file** derived from the effective harness list. For each such file:

1. Ensure **exactly one** region per enabled-plugin-with-`GUARDRAILS.md`, **minus** any plugin suppressed for that file's harness (Claude Code suppression, FR-013), with current content. An existing region is overwritten **between its markers in place** — never duplicated (FR-014, SC-006).
2. **Remove** any region whose `<catalog>:<plugin>` marker corresponds to a plugin no longer enabled, a harness no longer in the effective list, or (on Claude Code) a plugin that now ships `hooks/hooks.json`.

### Deterministic placement (FR-011)

Within each file, content is ordered:

1. The Phase 4 rules-include block (`tome:begin/end`) first.
2. Then one guardrails region per contributing plugin, in **lexicographic `<catalog>:<plugin>` order**.

Deterministic ordering means re-syncs never reorder existing content, which is load-bearing for idempotence (FR-011, NFR-001). The guardrails regions are placed **adjacent to** — not inside — the Phase 4 rules block. The region find/replace reuses the `rules_file.rs` block machinery, generalised to a parameterised marker pair (R-5, R-19).

### Cursor sibling deletion (FR-015)

The Cursor `TOME_GUARDRAILS.md` sibling is **deleted entirely** when no enabled plugin contributes a guardrails region to it (FR-015, SC-006).

## Suppression ordering + transitions (FR-016, R-13)

Within a single harness sync, the hooks-presence determination that drives the Claude Code suppression predicate (FR-013) MUST be computed **before** guardrails are reconciled for `CLAUDE.md`, so the predicate never reads stale state (FR-016). This is the fixed cross-sink order **hooks → guardrails → agents** (R-13).

Both suppression transitions are handled symmetrically in the same sync:

| Transition | Hooks action ([hooks-integration.md](./hooks-integration.md)) | `CLAUDE.md` guardrails action |
|---|---|---|
| Plugin **starts** shipping `hooks/hooks.json` between syncs | hooks merged (FR-004) | region **removed** (FR-014) |
| Plugin **stops** shipping `hooks/hooks.json` between syncs | hook entries **removed** (FR-005) | region **(re-)rendered** (FR-014) |

The suppression is `CLAUDE.md`-only; the plugin's region on the shared `AGENTS.md` is unaffected by either transition.

## Atomic write

Every write to a rules-file target or the Cursor sibling follows the Phase 4 atomic-write discipline:

1. Read the existing file into memory (for in-file regions; the sibling is fully Tome-owned).
2. Construct the new content (region inserted / overwritten-in-place / removed).
3. **Refuse to write through a symlink** — `symlink_metadata` check on the target before writing; exit 7 (`Io`) if the target is a symlink.
4. Write to a sibling temp file on the same filesystem; preserve the existing file's mode (new files 0644 on Unix for in-file targets; the Cursor sibling created at 0644); fsync; atomic rename onto the target.

Each individual file write stays all-or-nothing: a failure never leaves partially-written guardrails state between markers (FR-084). A render or write failure for a rules file or the standalone sibling surfaces **exit 46** (guardrails render/write failure); the message names the file. Per the forward-progress discipline (FR-084, R-13), a failure on one harness/sink does not roll back sinks already reconciled in the same sync.

## Errors

| Exit code | Trigger |
|---|---|
| 46 | Guardrails render or write failure (rules file or Cursor sibling). The message names the file. |

Pinned in [exit-codes-p6.md](./exit-codes-p6.md) (FR-092). Reuses no occupied code.

## Testing strategy

- **All-five-harness render**: a plugin shipping `GUARDRAILS.md` and no JSON hooks, enabled across all five harnesses, produces a `<catalog>:<plugin>` region in `CLAUDE.md`, the shared `AGENTS.md`, and the Cursor sibling (FR-011/012, SC-006).
- **Suppression**: a plugin shipping both `GUARDRAILS.md` and JSON hooks has its region present in `AGENTS.md` and absent from `CLAUDE.md` (FR-013, SC-006).
- **Two-plugin distinct regions**: two guardrails-shipping plugins produce two distinct, independently delimited regions in the shared file, in lexicographic `<catalog>:<plugin>` order (FR-011, SC-006).
- **Per-plugin removal**: disabling one plugin removes only its region; other plugins' regions remain (FR-014, SC-006).
- **Overwrite in place + idempotence**: re-sync with changed source content overwrites between existing markers; re-sync with unchanged content rewrites nothing (FR-014, NFR-001, SC-006).
- **Suppression transitions**: a plugin that begins shipping `hooks.json` has its `CLAUDE.md` region removed while hooks merge; a plugin that ceases has its hooks removed while the region re-renders — both in one sync (FR-016).
- **Cursor sibling deletion**: when the last contributing plugin is disabled, `TOME_GUARDRAILS.md` is deleted entirely (FR-015, SC-006).
- **Symlink refusal**: a symlinked target is refused (exit 7).
- **Atomicity**: an injected write failure surfaces exit 46 and leaves no partial region between markers.

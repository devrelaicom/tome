# Rules-File Integration — Contract

**Spec source**: [spec.md FR-480 through FR-484](../spec.md)

Two strategies decided per harness by `HarnessModule::rules_file_strategy()`.

## Strategy 1: `BlockInExistingFile`

Tome maintains a single delimited block inside a harness-chosen rules file. The block is bounded by **exact** byte markers:

```text
<!-- tome:begin -->
<body>
<!-- tome:end -->
```

**Match regex** (line-anchored, trailing whitespace tolerated):
- `^<!-- tome:begin -->\s*$`
- `^<!-- tome:end -->\s*$`

**Emit format**:
- `<!-- tome:begin -->` followed by exactly one `\n`.
- Body content.
- `<!-- tome:end -->` followed by exactly one `\n`.

**Body content depends on `HarnessModule::block_body_style()`**:

### `AtInclude`

```text
@<relative-path-to-.tome/RULES.md>
```

The body is a single `@`-prefixed include directive pointing at the project marker's rules-file copy (relative path from **the rules-file's containing directory**, not from the project root). The implementation MUST compute the relative path at write time via `pathdiff::diff_paths` (or equivalent) using `rules_file_target.parent()` as the base and `<project_root>/.tome/RULES.md` as the target.

Examples:

| Rules-file target | Relative include body |
|-------------------|------------------------|
| `<project>/AGENTS.md` | `@.tome/RULES.md` |
| `<project>/CLAUDE.md` | `@.tome/RULES.md` |
| `<project>/.claude/CLAUDE.md` | `@../.tome/RULES.md` |
| `<project>/.gemini/GEMINI.md` | `@../.tome/RULES.md` |

The sync algorithm passes `(rules_file_target, project_root)` to the rules-file writer; the writer computes the relative-path body. The harness module's `block_body_style()` returning `AtInclude` is the signal to use this computed body; the harness module itself does NOT compute the path (the trait remains free of project-root parameters in `block_body_style`).

### `Inline`

```text
<full rules content verbatim>
```

The body is the full `RULES.md` content from the project marker, inserted between the markers. For harnesses without `@`-include support, sync must rewrite the block on every summary regeneration.

## Strategy 2: `StandaloneFile`

Tome writes a complete file at the path returned by `rules_file_target(project_root)`. No markers; the file is entirely Tome-owned.

- File contents: the project marker's `RULES.md` body, verbatim.
- Removal: delete the file (cleanly; the directory is untouched).
- Multiple `StandaloneFile`-strategy harnesses: each writes to its own path (the harness module's contract returns a harness-specific path; collisions are not possible).

## Block ownership

Content **inside** the block markers is fully Tome-owned. Hand-edits inside the block are overwritten on next sync. Content **outside** the block markers (above `<!-- tome:begin -->` or below `<!-- tome:end -->`) is preserved verbatim across syncs.

For standalone-file strategy, the whole file is Tome-owned; there is no "outside the block" concept.

## Multi-harness shared rules-file

When more than one harness in the effective list targets the same rules-file path (e.g. several harnesses all using `AGENTS.md`), the sync algorithm writes **one** block at that path. The block's body is identical to what any single one of those harnesses would have written. Per FR-482.

Removal logic (FR-483):
- The block stays as long as ANY harness in the effective list still targets that path.
- The block is removed only when NO harness in the effective list still targets that path.

The sync algorithm runs the rules-file reconciliation pass over the **set** of target paths (deduplicated), not over the list of harnesses, to avoid double-writing.

## Atomic file write

Every rules-file write follows the Phase 1 atomic-write discipline:

1. Read the existing file into memory.
2. Construct the new content (block-inserted or block-removed, depending on operation).
3. Write to a sibling temp file on the same filesystem.
4. fsync.
5. Atomic rename onto the original path.

On Unix, the temp file is created with mode 0644 (regular file). Existing file permissions are preserved through the rename when the target already existed.

## Edge cases

- **Rules file doesn't exist at the harness's target path, harness is in effective list**: sync creates the file with just the Tome block and a closing newline.
- **Rules file exists, no Tome block present, harness is in effective list**: sync appends the block to the end of the file (two `\n` separators between existing content and the block to avoid merging with prior text).
- **Rules file exists, Tome block present, harness no longer in effective list**: sync removes the block; if surrounding content is empty after removal, the file is left in place with empty content (not deleted — the developer authored it).
- **Rules file is a symlink**: sync MUST refuse to write through the symlink. Phase 4's security hardening (carried from Phase 3 P8 PR-F) extends to rules-file targets — `is_symlink()` check before any write; exit 7 (`Io`) with a clear message if hit.
- **Multiple Tome blocks in the same file** (developer pasted content twice): sync collapses to the canonical position (replaces the first block, removes subsequent ones).

## Test coverage

- `tests/rules_file_block_in_existing.rs` — block insertion, update, removal; AtInclude and Inline body styles; multi-harness shared file; surrounding content preservation; symlink refusal.
- `tests/rules_file_standalone.rs` — Cursor's standalone file creation, removal, no marker handling.
- `tests/sync_idempotence.rs` — re-run produces zero file writes when state matches effective list.

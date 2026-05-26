# Phase 5 — Exit codes

Authoritative numeric assignments for Phase 5 failure classes. Pin these here; do not re-litigate in implementation. Every Phase 5 `TomeError` variant maps to exactly one of these codes via `From<TomeError> for ExitCode` in `src/error.rs`.

## Scope

Phase 1–4 already enumerate codes 0, 2, 4–8, 10–20, 30–36, 41, 50–54, 60–61, 70–75. Phase 5 claims new codes for its closed-enum additions. **Code 24 is already taken by Phase 4's `SummariserFailure`**, and **codes 20–23 are already taken by Phase 2's plugin lifecycle** (`PluginNotFound`, `PluginAlreadyInState`, `PluginManifestParseError`, `SkillFrontmatterParseError`). Phase 5 reassigns its proposed slots accordingly — see "Reassigned slots" below.

## Reassigned slots (contract amendment)

The first draft of this contract proposed codes **21, 22, 23** for `EntryNotFound`, `SubstitutionFailed`, and `InvalidArgumentFrontmatter`. Those numbers collide with Phase 2's plugin lifecycle variants (`PluginAlreadyInState`, `PluginManifestParseError`, `SkillFrontmatterParseError`). Constitution principle II (NON-NEGOTIABLE pairwise-unique exit codes) requires distinct numbers, so F1 reassigned them to **27, 28, 29** — same precedent as Phase 4 F3 which moved `SummariserFailure` from contract-proposed 20 to actual 24 to dodge `PluginNotFound`.

After this amendment, Phase 5 occupies a clean contiguous cluster at **25–29**.

| Original proposal | Final assignment |
|---|---|
| 21 | 27 |
| 22 | 28 |
| 23 | 29 |
| 25 | 25 (unchanged) |
| 26 | 26 (unchanged) |

All references below have been updated to the final assignment.

## Assignments

| Code | Variant | Class | Description |
|---|---|---|---|
| 25 | `WorkspaceDataDirWriteFailed { path, source }` | I/O | `create_dir_all` on `${TOME_PLUGIN_DATA}` or `${TOME_WORKSPACE_DATA}` failed (covers both directory classes per Edge Cases resolution). The path field distinguishes which class triggered. |
| 26 | `PromptArgumentMismatch { expected, supplied }` | Caller-supplied | The caller (harness via prompts/get OR agent via get_skill with args) supplied more arguments than the entry's declared schema permits, or supplied named arguments that don't match declared names. |
| 27 | `EntryNotFound { catalog, plugin, name, kind }` | Lookup | Named entry not found in the active workspace's enabled set (covers Phase 5 read-tool, prompts/get, get_skill_info, plugin-show lookups). |
| 28 | `SubstitutionFailed { reason }` | Substitution | Substitution layer encountered an unrecoverable failure during a single rendering pass (e.g. invalid argument count beyond what the prompt schema declared, malformed regex match construction — unlikely in production but enumerated for completeness). |
| 29 | `InvalidArgumentFrontmatter { file, reason }` | Parse | The `arguments` frontmatter field is malformed (e.g. non-string list element, illegal argument name not matching `[a-z_][a-z0-9_]*`). |

### Reserved skips

- **24** is **Phase 4's `SummariserFailure`**; Phase 5 does NOT reuse it. The Phase 5 PRD pre-allocated 24 to "Prompt argument count exceeds caller-supplied args"; this contract finalises that slot as **26** to preserve Phase 4's existing semantics.
- **20–23** are **Phase 2's plugin lifecycle**; see Reassigned slots above.

## Reused codes

Phase 5 surfaces re-emit these existing variants where the failure mode maps cleanly:

| Code | Existing variant | Phase 5 site |
|---|---|---|
| 2 | clap usage error | `tome plugin show` / new flags / argument-schema deserialisation by harness |
| 7 | `Io` | File-system reads of entry bodies, frontmatter, resource enumeration listing |
| 13 | `WorkspaceNotFound` | All Phase 5 MCP surfaces when the resolved workspace doesn't exist (carryover) |
| 70 | `WorkspaceMalformed` | Frontmatter parse failure on Tome-owned config (not on third-party plugin frontmatter, which surfaces as 29) |
| 73 | `SchemaVersionTooNew` | Read-only DB open against an unknown future schema (carryover) |
| 74 | `SchemaMigrationFailed` | Phase 5 migration v2 → v3 failure |

## JSON envelope

CLI commands emit failures in the existing JSON envelope on `--json`:

```json
{
  "ok": false,
  "exit_code": 27,
  "error": {
    "code": "entry_not_found",
    "message": "entry not found: midnight-expert/compact-dev/circuits (kind: skill)",
    "data": {
      "catalog": "midnight-expert",
      "plugin": "compact-dev",
      "name": "circuits",
      "kind": "skill"
    }
  }
}
```

MCP tool / prompts handlers map these to MCP error envelopes per the existing Phase 3 pattern (`ErrorData.data.code` = the `"code"` slug from the table; numeric JSON-RPC code is generic `INVALID_PARAMS` / `INTERNAL_ERROR`).

## Code slugs (for MCP `data.code` field)

| Code | Slug |
|---|---|
| 25 | `workspace_data_dir_write_failed` |
| 26 | `prompt_argument_mismatch` |
| 27 | `entry_not_found` |
| 28 | `substitution_failed` |
| 29 | `invalid_argument_frontmatter` |

## Discipline

When implementing, the `From<TomeError> for ExitCode` mapping must:
1. Cover every Phase 5 variant exhaustively (no `_ => ExitCode(1)` fallback for the new variants).
2. Be tested via `tests/exit_codes.rs` (extended with five new assertion entries — one per Phase 5 code).
3. Be reflected in `tests/exit_codes_e2e.rs` (extended with CLI-binary coverage for at least codes 25, 26, 27, 29 — the substitution code 28 is harder to trigger via CLI binary and may stay library-API-only per the established split).

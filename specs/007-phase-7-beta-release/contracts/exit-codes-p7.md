# Contract: Exit Codes — Phase 7 (NO new codes; the reuse map)

**FRs**: FR-014, FR-015 · **NFRs**: NFR-002 · **Constitution**: §II (NON-NEGOTIABLE)

## Contract

Phase 7 introduces **no new `TomeError` variant and no new exit code**. Every new or corrected failure path reuses an existing, semantically-matching variant. This contract is the authoritative reuse map; it cross-references `src/error.rs::exit_code()` (the canonical truth — verified at planning time).

## Occupied exit-code set (verified, unchanged this phase)

```
0  success
2  usage
1,3,4,5,6,7,8,9
13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29
30,31,32,33,34,35,36,37
40,41,42,43,44,45,46
50,51,52,53,54
60,61
70,73,74,75
```

No code is added, removed, or repurposed (§II). The PRD-less Phase 7 reuses; it does not allocate.

## Reuse map

| Failure (Phase 7) | Reused variant | Code | Rule |
|---|---|---|---|
| Malformed `~/.tome/config.toml` (FR-014) | `ManifestInvalid::TomlParse { file, message }` | **5** | config TOML parse **is** a manifest-parse failure; replaces today's generic `Internal` (exit 1) collapse, which `Internal`'s own doc forbids |
| Non-array `hooks` event value (FR-015) | `HookSettingsWriteFailed { path }` | **44** | fail closed; do **not** coerce to `[]`; matches the module's otherwise fail-closed discipline |
| Meta row indicating corruption (FR-015) | a diagnostic *distinction* over existing variants (e.g. `IndexIntegrityCheckFailure` (51) vs. fresh-DB bootstrap) | existing | distinguish corruption from a fresh DB; **no new code** (the `current_schema_version` `.ok()` collapse is the bug) |
| Symlink intermediate-component refusal (FR-007) | the **calling sink's** existing write-guard variant | varies | settings → `HookSettingsWriteFailed` (44); guardrails → `GuardrailsWriteFailed` (46); agents → its existing write code; rules/mcp → their existing codes |
| Bounded-read overflow (FR-006) | the read's existing per-class parse/size error naming the file | existing | per-class cap (`PLUGIN_MANIFEST_MAX` / `HARNESS_MCP_MAX`), named error, never OOM |

## The CON-1 precedent (preserved)

A write-guard failure on a **dedicated sink** returns **that sink's** dedicated code, never a regression to generic `Io` (7). Phase 6 reconciled the hooks settings sink 7→44 to match the guardrails sink's 7→46. The FR-007 symlink-guard consolidation MUST honour this: the single symlink-safe primitive maps its refusal to the *caller's* dedicated code at each call site. A **source read** path (e.g. reading a plugin's `hooks.json`) deliberately stays generic `Io` (7) — only the **write** path gets the dedicated code (Phase 6 C2-3/T2-6).

## Test obligations

- A unit/integration assertion per reused mapping above (config-parse → 5; non-array-hooks → 44; meta-corruption distinguished).
- The FR-007 symlink refusal asserts the **sink-specific** code at each sink (not 7), exercised by `tests/symlink_intermediate_guard.rs`.
- `tests/exit_codes.rs` (the closed-set guard) gains no new arm — its existing exhaustiveness check confirms zero new codes.

## Anti-requirements

- MUST NOT add a `ConfigParse`, `SymlinkRefused`, or any new variant.
- MUST NOT change the meaning of any shipped code.
- MUST NOT collapse a Tome-owned-input parse failure to `Internal` (the specific-over-generic rule).

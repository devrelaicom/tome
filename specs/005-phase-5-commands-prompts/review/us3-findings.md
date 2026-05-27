# Phase 5 / US3 — Reviewer findings

Consolidated from 4 parallel reviewers. Source reports: `/tmp/tome-phase5-us3-{contract,rust,test,security}.md`.

## Tally

| Reviewer | Blockers | Majors | Minors |
|---|---|---|---|
| Contract  | 0 | 0 | 0 |
| Rust      | 0 | 4 | 5 |
| Test      | 0 | 1 critical gap | 3 medium |
| Security  | 0 | 0 (inherited hardening enforces) | 0 |
| **Total** | **0** | **5** | **8** |

Cleanest reviewer pass of Phase 5 so far. The single-sweep refactor from US2.d means the structurally-correct invariants are inherited; US3 just has small polish items.

## Majors

### R-M1: `body_references_arguments` still uses substring check (false positives in prompts/list)
**File**: `src/mcp/prompts.rs:94-96`
**Issue**: The doc comment said "US3 will replace this with a real regex check"; US3 has now landed but the substring check stayed. A body containing `$ARGUMENTS_HELP`, `$ARGUMENTS_SUFFIX`, etc. would falsely advertise a catch-all `args` argument in `prompts/list`.
**Fix**: dispatch through `regex_sets::combined_regex()` and check for `m.as_str() == "$ARGUMENTS"` (no capture groups set).

### R-M2: Dead code `_entry_identity_from_record` (zero callers)
**File**: `src/mcp/prompts.rs:932-942`
**Issue**: `#[doc(hidden)] pub` helper for "tests that want to fabricate a registry" — but no test consumes it. CLAUDE.md discipline requires `#[allow(dead_code)]` + named future-consumer comment, or deletion.
**Fix**: delete.

### R-M3: `coerce_arguments` extra `String::clone` per declared name
**File**: `src/substitution/arguments.rs:197-201`
**Issue**: Two clones of `value` per iteration. Reorder to save one allocation per declared arg per `prompts/get` call.
**Status**: DEFER — micro-optimization, save for polish.

### R-M4: `coerce_arguments` empty-Object on no-declared-args path returns confusing mismatch
**File**: `src/substitution/arguments.rs:152-170`
**Issue**: Edge case for library callers (not MCP); error message reads `expected: 0, supplied: 0`.
**Status**: DEFER — edge case.

### T-M1: NFR-007 Stage 2↔3 boundary untested
**File**: `tests/substitution_arguments.rs`
**Issue**: 3 no-rescan tests cover Stage 1↔3 + Stage 3↔3, but the Stage 2↔3 boundary isn't tested explicitly. The single-sweep architecture makes this structurally impossible to violate, but a test pins the invariant.
**Fix**: Add `stage_2_substituted_value_containing_dollar_pattern_is_not_rescanned_by_stage_3`.

## Deferred to v0.6+ backlog

- R-M3, R-M4 (above)
- 5 Rust minors (cosmetic refactors)
- Test minors: newline in arg values, unmatched-quote edge cases, JSON error envelope shapes
- Security inherits prior hardening; no new gaps

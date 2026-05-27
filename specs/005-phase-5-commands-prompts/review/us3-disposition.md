# Phase 5 / US3 — Disposition

Records which reviewer findings (from `us3-findings.md`) are applied in US3.d vs deferred. Mirrors US1.d + US2.d pattern.

## Applied in US3.d (0 BLOCKERS + 2 MAJORS + 1 TEST)

| ID | Severity | What | Where |
|---|---|---|---|
| R-M1 | MAJOR | Replace substring check with regex dispatch | `src/mcp/prompts.rs` |
| R-M2 | MAJOR | Delete dead `_entry_identity_from_record` | `src/mcp/prompts.rs` |
| T-M1 | MAJOR | Add Stage 2↔3 no-rescan test | `tests/substitution_arguments.rs` |

## Deferred to v0.6+ backlog

- **R-M3**: `coerce_arguments` clone micro-optimization
- **R-M4**: empty-Object library-API error message
- 5 Rust minors (m1–m5 cosmetic)
- Test newline-in-arg-value edge case
- Test unmatched-quote edge cases
- Test JSON error envelope shape coverage

All deferred items go into v0.6 polish backlog.

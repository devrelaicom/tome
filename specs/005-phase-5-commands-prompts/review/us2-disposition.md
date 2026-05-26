# Phase 5 / US2 — Disposition

Records which reviewer findings (from `us2-findings.md`) are applied in US2.d
vs deferred. Mirrors US1.d pattern.

## Applied in US2.d (2 BLOCKERS + 6 MAJORS)

| ID | Severity | What | Where |
|---|---|---|---|
| B1 | BLOCKER | Fix `WORKSPACE_DATA` error to use `WorkspaceDataDirCreationFailed` (exit 25) | `src/substitution/builtins.rs` |
| B2 | BLOCKER | Merge Stage 1+2 into single regex sweep — fixes no-rescan invariant + closes exfiltration vector | `src/substitution/{mod,builtins,env,regex_sets}.rs` |
| R-M1 + R-M5 | MAJOR | Subsumed by B2 fix (single sweep with shared fast-path) | (covered by B2) |
| R-M2 | MAJOR | Drop dead `plugin_data_dir` + `workspace_data_dir` fields from SubstitutionContext | `src/substitution/context.rs` + all builder call sites |
| R-M3 | MAJOR | Subsumed by B2 (merged regex doesn't fire debug log on `${TOME_ENV_*}`) | (covered by B2) |
| R-M4 | MAJOR | `caps.get(1).expect(...)` instead of `.unwrap_or("")` | `src/substitution/builtins.rs` + `src/substitution/env.rs` (1 line each) |
| T-M1 | MAJOR | Add NFR-007 no-rescan invariant test | `tests/substitution_env.rs` (or new `tests/substitution_no_rescan.rs`) |

## Deferred to v0.6+ backlog

### Majors deferred
- **C-M1, C-m1, C-m2**: contract polish + variant-reuse note (subsumed or doc-only)
- **R-S1–S6**: 6 Rust suggestions (drop `_default` arg, `body[..]` micro-opt, `u32::MAX` clip, `WorkspaceName::parse` storage, `rewrite_marker_workspace` allocation, `Arc<Paths>`)
- **T-M2**: `WorkspaceDataDirCreationFailed` variant untested (rare path)
- **T-M3+**: edge cases (empty body, adjacent references, Unicode values)
- **Security MEDIUM**: env-var exfiltration via legit `TOME_ENV_*` (intentional behaviour; document in v0.6 operator guide)
- **Security MEDIUM**: Unicode path component sanitisation hardening
- **Security LOW**: symlink check in rename, 0600 mode on plugin-data dirs

## New tests added in US2.d

- `tests/substitution_env.rs` extended with `stage_1_substituted_value_containing_tome_env_syntax_not_rescanned_by_stage_2` — verifies the no-rescan invariant directly

Expected test delta: +3–5 tests.

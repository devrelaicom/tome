# Phase 5 / US1 — Disposition

Records which reviewer findings (from `us1-findings.md`) are applied in US1.d
vs deferred. Mirrors the Phase 4 disposition pattern.

## Applied in US1.d (1 BLOCKER + 8 MAJORS)

| ID | Severity | What | Where | Notes |
|---|---|---|---|---|
| S-H1 | BLOCKER | Path traversal validation in `resolve_entry_body_path` | `src/index/skills.rs` | Reject `..` components; tests added |
| R-M1 | MAJOR | Add `PluginDataDirWriteFailed` variant (closed-set discipline) | `src/error.rs` + `src/mcp/prompts.rs` | New exit code allocated from free range |
| R-M2 + S-L1 | MAJOR | Frontmatter parse → `SkillFrontmatterParseError` (not `EntryNotFound`) | `src/mcp/prompts.rs` | Path no longer leaks into `EntryNotFound.name` |
| R-M3 + R-M4 | MAJOR | Cache `plugin_version` on `PromptEntry`; skip path round-trip | `src/mcp/prompts.rs` + `src/index/skills.rs` | Combined fix — removes DB double-open + lossy String round-trip |
| R-M5 | MAJOR | Drop dead `Box<SkillRecord>` from `LookupHit` | `src/mcp/tools/get_skill.rs` | Trivial cleanup |
| S-M1 | MAJOR | Cap `arguments` list at 256 in frontmatter parser | `src/plugin/frontmatter.rs` | DoS protection at enable time |
| T-M1 + T-M5 | MAJOR | Pin error-envelope JSON wire shapes | `tests/mcp_prompts_get_error_json_shape.rs` (new) | Per Phase 4 P8 JSON wire-pin discipline |
| T-M3 | MAJOR | Test registry-build degradation when entry files missing | `tests/mcp_prompts.rs` | Covers the warn-and-skip branches |

## Deferred to v0.6+ backlog

### Majors deferred

| ID | What | Why defer |
|---|---|---|
| C-M1 | Description-truncation timing comment | Cosmetic; semantics align with contract |
| C-M2 | `$ARGUMENTS` substring vs regex | Already explicitly deferred to US3 in the contract |
| R-M6 | `Arc<PromptEntry>` in registry | Optimization for high-prompt-count plugins; current implementations are low-count |
| T-M2 | Substitution failure path test | F3 stub returns Ok(body); real test materialises naturally in US2/US3 |
| S-M2 | YAML deserialisation panic safety | `serde_yaml` rarely panics; bounded reads + valid-UTF8 frontmatter mitigate; revisit if a real CVE emerges |

### Minors deferred

All 16 minor findings (8 Rust + 5 Test + 2 Contract + 1 Security) are deferred. They cluster around:

- Cosmetic doc-comment additions (C-m2)
- Edge-case test coverage (T-m1–T-m4) — multi-byte Unicode, NUL, path traversal in sanitisation, empty-string arguments
- Code-quality refactors (R-m1–R-m8) — substring vs regex heuristic, dead test helper, manifest-walk duplication, dead enum arm, missing-args validation, format-vs-write micro-opt, clone storms
- Documentation polish (R-m4 — builder-incomplete error variant)
- Implementation note (C-m1)
- Path disclosure in error messages (S-L1) — addressed indirectly via R-M2 (no longer surfaces relative path in `EntryNotFound.name`)

These are tracked here and will be batched into a Phase 5 Polish PR-C/D/E sweep at v0.5.0 cut, or rolled forward to v0.6.0 if Polish capacity is tight.

## New exit code allocation (R-M1)

| Code | Variant | Notes |
|---|---|---|
| 30 | `PluginDataDirWriteFailed` | First free slot after Phase 5's 25-29 cluster and before Phase 2's models 30-33 |

Wait — code 30 is already taken by Phase 2's `ModelMissing`. Need to pick another. Free slots in the 0–80 range:
- 9, 11, 12 (Phase 1 has 1–8, then 13+)
- 38, 39, 43–49, 55–59, 62–69, 71, 72, 76–80

Pick **9** for `PluginDataDirWriteFailed` (low + adjacent to Phase 1's I/O cluster which it semantically resembles). Document the choice in `src/error.rs` with a comment, and amend `contracts/exit-codes-p5.md` inline.

## Tests added in US1.d

- `tests/mcp_prompts_get_error_json_shape.rs` (new) — 4 tests pinning the JSON envelope shape for `prompt_not_found` / `prompt_argument_mismatch` / `substitution_failed` / `workspace_data_dir_write_failed`
- `tests/mcp_prompts.rs` (extended) — registry-build degradation tests (entry file missing on disk; frontmatter malformed)
- `tests/exit_codes.rs` (extended) — assertion for the new exit code 9
- `tests/security_hardening.rs` (extended) — path traversal rejection in `resolve_entry_body_path`
- Possibly `tests/path_validation.rs` (extended) for the relative-path validation helper

Expected test delta: +6–10 tests across 1 new suite + 2 extensions.

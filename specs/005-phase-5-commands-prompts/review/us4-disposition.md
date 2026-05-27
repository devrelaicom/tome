# Phase 5 / US4 — Disposition

## Applied in US4.d (0 BLOCKERS + 1 HIGH + 2 majors + 3 test gaps)

| ID | Severity | What | Where |
|---|---|---|---|
| C-2 + Security HIGH | HIGH (perf + DoS) | Rewrite truncate_description (bounded single-pass via char_indices) | `src/mcp/tools/search_skills.rs` |
| C-1 | MAJOR | Document walk_resources TOCTOU residual + accepted-risk rationale | `src/mcp/tools/get_skill_info.rs` |
| M-1 | MAJOR | Document MAX_DESCRIPTION_MAX_CHARS=100_000 in contract | `contracts/mcp-tools-p5.md` |
| Test gap | MAJOR | Add Unicode boundary truncation test | `tests/mcp_search_skills_truncation.rs` |
| Test gap | MAJOR | Add `invalid_kind` + `resource_enum_failed` envelope tests | `tests/mcp_get_skill_info.rs` |

## Deferred to v0.6+

- C-3 shared `tome_to_mcp` helper (refactor; not blocking)
- M-2 `max=0` contract drift (carve-out is test-pinned)
- M-3 `state.scope.scope` accessor cosmetic
- M-4 `prepare_cached` for KNN (perf; profile-needed)
- M-5 continue-on-fail in walk_resources (design choice)
- M-6 validation order
- 8 Rust minors (cosmetic)
- Security MEDIUM TOCTOU (covered by C-1 doc)
- Security LOW path-in-error-envelope

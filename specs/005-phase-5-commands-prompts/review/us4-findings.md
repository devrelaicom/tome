# Phase 5 / US4 — Reviewer findings

Consolidated from 4 parallel reviewers. Source: `/tmp/tome-phase5-us4-{contract,rust,test,security}.md`.

## Tally

| Reviewer | Blockers | Majors | Minors |
|---|---|---|---|
| Contract  | 0 | 0 | 0 |
| Rust      | 0 (3 "critical" not BLOCKER-tier; orchestrator classification) | 7 | 8 |
| Test      | 0 | 4 gaps | a few |
| Security  | 0 | 1 HIGH (overlaps with Rust C-2) | 2 |
| **Total** | **0** | **~12** | many |

## Headline issue (overlapping C-2 + Security HIGH)

`truncate_description` does TWO O(n) `chars()` passes for every result, even when no truncation occurs. Caller-controlled `description_max_chars` (sanity cap 100,000) on top_k=100 results × multi-KB descriptions = DoS amplifier. Fix: single-pass `char_indices`-based truncation.

## Selected for US4.d

### C-2 + Security HIGH: Rewrite `truncate_description` (bounded; single-pass)
**File**: `src/mcp/tools/search_skills.rs:321-331`
**Issue**: Two O(n) passes per result; megabyte description × top_k = significant CPU
**Fix**: Use `char_indices()` to walk past `max` chars then stop; no `chars().count()` early call.

### C-1: Document `walk_resources` TOCTOU residual
**File**: `src/mcp/tools/get_skill_info.rs::walk_resources`
**Issue**: After `file_type()` lstat check + path collection, a hostile concurrent `rename(2)` could swap subdir → symlink before `read_dir(sub)` follows it.
**Fix**: Doc-only — accept the residual race per Phase 4's trust model (catalog is trusted-on-enrol, not trusted-on-read). Document in walk_resources's comment.

### M-1: Document `MAX_DESCRIPTION_MAX_CHARS = 100_000` in the contract
**File**: `src/mcp/tools/search_skills.rs:68` + `contracts/mcp-tools-p5.md`
**Issue**: Contract only mentions `< 0` triggering `invalid_description_max_chars`. The 100_000 cap is undocumented.
**Fix**: Amend contract to specify the cap (sanity guard above documented surface).

### Test gaps to fill
- Unicode truncation boundary test (multi-byte char at the `max` boundary)
- `invalid_kind` error envelope test (gh-issue-able if we don't have one)
- `resource_enum_failed` error envelope test

## Deferred to v0.6+

### Rust (5 majors + 8 minors deferred)
- C-3: extract shared `tome_to_mcp` helper (refactor; both tools work)
- M-2: max=0 contract drift (carve-out is test-pinned)
- M-3: `state.scope.scope` accessor (cosmetic)
- M-4: `prepare_cached` for KNN query (perf; profile-needed)
- M-5: continue-on-fail in `walk_resources` (design choice)
- M-6: validation order (UX nicety)
- 8 minors (cosmetic)

### Security (2 deferred)
- MEDIUM TOCTOU (covered by C-1 doc; structural fix would need `cap-std`)
- LOW path-in-error-envelope (consistent with success-response shape)

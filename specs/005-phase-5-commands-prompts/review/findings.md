# Phase 5 Polish — Phase-wide reviewer findings

Consolidated from 4 parallel reviewers run against `main` at `abd0f48` (US5.c closeout merged). Source reports: `/tmp/tome-phase5-polish-{contract,rust,test,security}.md`.

This is the cross-cutting pass. The per-US passes (US1.d, US2.d, US3.d, US4.d, US5.c) already caught and fixed per-slice issues; this pass looked for cross-US drift that single-slice reviewers couldn't see.

## Tally

| Reviewer | BLOCKER | MAJOR | MINOR | NIT |
|---|---|---|---|---|
| Contract | 0 | 1 (doc) | 0 | — |
| Rust | 0 | 4 | 7 | 3 |
| Test | 0 | 2 GAP | 3 WEAK + 1 MINOR | — |
| Security | 0 | 0 | 0 | — |
| **Total** | **0** | **7** | **~14** | **3** |

Reviewer trend across Phase 5 closeouts:
- US1.d: 1B + 8M (path-traversal HIGH; closed)
- US2.d: 2B + 6M (exfiltration HIGH; closed)
- US3.d: 0B + 2M
- US4.d: 0B + 1H + 2M (truncate_description DoS; closed)
- US5.c: 0B + 4M
- **Polish (this pass): 0B + 7M** — slightly higher than US5.c because it's looking at the full surface; about-right shape for cross-US drift.

Security audit: **clean across the board** (0 HIGH / 0 MEDIUM / 0 LOW). The 4 critical security findings caught during per-US passes (US1.d S-H1, US2.d B2, US4.d C-2) all hold their fixes; no new exposure introduced by US5.

## Headline findings

### M-1 (Rust): `prompts::truncate_description` still ships US4.d's O(n) shape

**File**: `src/mcp/prompts.rs:72-79`
**Issue**: US4.d HIGH-severity fixed `truncate_description` in `search_skills.rs` to use a bounded `char_indices` walk (at most `max+1` chars). The function with the same name in `prompts.rs` still has the un-fixed O(n) shape — early `chars().count()` walk of full input, then `chars().take().collect()` allocation. Docstring claims "same Unicode-safe approach" but they no longer share an approach.

Not BLOCKER because this runs at registry-build time (not request path); a hostile catalog description is still bounded by US5.c's 100 KiB soft warning surface. But the **pattern drift** is the real issue — US4.d's lesson didn't propagate.

**Fix**: Port the US4.d bounded-walk shape into `prompts.rs::truncate_description`; update docstring to truthfully claim "mirrors search_skills' bounded-walk approach". (Smaller blast radius vs extracting a shared `util::text` helper.)

### M-2 (Rust): `build_get_context` and `build_substitution_context` are near-duplicates

**Files**: `src/mcp/prompts.rs:605-663` + `src/mcp/tools/get_skill.rs:410-449`

**Issue**: Both functions derive `entry_dir`, walk ancestors for `.claude-plugin/`, call `current_clock()`, and build the same `SubstitutionContext` with the same 12 setters. Divergences are exactly two: arg handling + plugin_version source. The `prompts.rs` version has a **stale** docstring forward-reference ("Real production callers in US2 will replace this") — US2 shipped without doing so.

**Fix**: Extract `build_context_for_entry` helper into `src/substitution/context.rs`. Both callers reduce to a one-line call. ~50 LOC duplication eliminated; single seam for future `.claude-plugin/`-walk replacement.

### M-3 (Rust): Two stringly-typed `match kind.as_str()` dispatchers should use `EntryKind`

**Files**: `src/doctor/checks.rs:483-485` + `src/commands/plugin/mod.rs:266-268`

**Issue**: Both do `match kind.as_str() { "skill" => ..., "command" => ..., _ => {} }` with silent fall-through on unknown kinds. The canonical sites (`src/index/skills.rs:189`, `:753`; `src/index/query.rs:106`; `src/mcp/prompts.rs:252`) all parse to `EntryKind` and surface unknowns as `IndexIntegrityCheckFailure`. Defence-in-depth for schema drift.

**Fix**: Parse to `EntryKind` at both sites; surface unknowns as `IndexIntegrityCheckFailure`. Both sites are inside `Result`-returning functions; `?` lands cleanly.

### M-4 (Rust): `plugin/show.rs::list_entries` partially duplicates `resolve_entry_body_path`'s safety check

**File**: `src/commands/plugin/show.rs:155-168`

**Issue**: Re-implements the absolute-path-and-`..`-component refusal from US1.d's S-H1 BLOCKER fix in `resolve_entry_body_path`. Documented as intentional ("boundary mirror" comment + docstring) but the divergence creates future-unsafe drift — if a third check (NUL refusal, UTF-8 component validation) is added to `resolve_entry_body_path`, this site silently lags. The second SSOT-via-comment-not-code instance in the same file family (M-2 being the first).

**Fix**: Extract `pub(crate) fn validate_db_stored_path(stored: &Path) -> Result<(), TomeError>` (or `is_safe_relative(...) -> bool`) as a helper in `src/index/skills.rs` next to `resolve_entry_body_path`. Both call sites use the helper.

### GAP-1 (Test): Exit-code e2e tests for Phase 5 codes 9, 25-29 missing

**File**: `tests/exit_codes_e2e.rs`
**Issue**: Phase 5 added 6 new exit codes; unit-level tests exist in `tests/exit_codes.rs` (variant → code mapping pinned), but no CLI-binary e2e tests trigger the codes through the production code path. The Phase 4 baseline (`exit_codes_e2e.rs`) covered codes 22/33/51/20/2/14/16/17/18/70/7; nothing for Phase 5 codes.

**Fix**: Add e2e tests that trigger each code through CLI binary (`assert_cmd::Command::cargo_bin("tome")`) and assert exit status. Code 9 (PluginDataDirWriteFailed), 25 (WorkspaceDataDirWriteFailed), 26 (PromptArgumentMismatch — requires MCP server stub; may defer), 27 (EntryNotFound — same), 28 (SubstitutionFailed — same), 29 (InvalidArgumentFrontmatter).

### GAP-2 (Test): `pending_re_embedding=0` zero-state untested

**File**: `tests/doctor_p5.rs`
**Issue**: Positive case `pending_re_embedding_count_matches_dirty_rows` (file touched → count >=1) is pinned; zero-state (file untouched → count == 0) is not. Regression-safety for the heuristic's clean state.

**Fix**: Add `pending_re_embedding_zero_when_no_files_touched` — enable, run doctor immediately, assert `counts.pending_re_embedding == 0`.

### CA-M1 (Contract): Stage 4 input-shape clarification

**File**: `specs/005-phase-5-commands-prompts/contracts/substitution-engine.md` § Stage 4
**Issue**: Contract table for Stage 4 ARGUMENTS append-fallback doesn't explicitly distinguish single-string-input vs Object-input cases. Current implementation correctly appends single-string verbatim and Object-input positional-joined; clarification keeps the contract truthful to behaviour.

**Fix**: Doc-only amendment to the contract.

## WEAK findings (test reviewer) — applied selectively

- **WEAK #2-1** (`SubsystemHealth` enum variants not byte-stable pinned): the enum is exercised through every doctor JSON test but no test pins the wire-shape names explicitly. Phase 4 carve-out; the existing JSON tests structurally cover the variants. **DEFER** (matches Phase 4 disposition; no regression risk).
- **WEAK #3-1** (`MAX_DESCRIPTION_MAX_CHARS=100_000` cap not validated at parse time): the cap is enforced only at *response truncation* time (search_skills), not at *parse* time. Documented as soft cap with `tracing::warn!` (US5.c R-M4). **DEFER** — cap is intentionally soft per trust model.
- **WEAK / GAP #2-1** (`ProjectBindingState`, `RulesCopyState`, `HarnessSubsystemReport` lack direct pins): Phase 4 types; pre-Phase-5 carve-out. **DEFER** — Phase 4 disposition holds.
- **MINOR organisational**: covered in deferred items.

## Minor Rust findings — selectively applied

- **m-1** (`Value::String(s) => s.clone()` could be `to_owned()`): cosmetic. **DEFER**.
- **m-2** (`body_references_arguments` one-line delegate): inline opportunity. **DEFER** (single-call wrapper; net cosmetic).
- **m-3** (`apply_arguments_match`'s `unwrap_or(usize::MAX)` deserves a comment): documentation improvement. **APPLY** (one-line comment).
- **m-4** (`PromptEntry.descriptor(name: String)` shadows `self.name`): naming clarification. **APPLY** (rename parameter to `prompt_name`).
- 3 cosmetic minors + 3 nits: **DEFER**.

## Deferred to v0.6+ / Polish backlog (carried forward)

- All from `us{1..5}-disposition.md` already-deferred items: R-M2 (orphan-workspace-dir flood), cap-std hardening, terminal escape sequences in names, prompt-injection trust boundary doc, etc.
- WEAK #2-1, #3-1, #4 (above) per their rationale.
- Rust nits m-1/m-2 + 3 cosmetic + 3 nits per us5-disposition.md pattern.

## Cross-US security observations

The security reviewer reported clean across the board — all 9 critical concerns investigated returned PASS:
1. Substitution engine single-sweep invariant intact (US2.d B2 structural fix held)
2. `get_skill_info` resource enumeration: symlink-skip + caps enforced
3. `detect_orphan_data_dirs`: symlink refusal at every level + dir-only + error-graceful
4. `resolve_entry_body_path` reach: 4 call sites all use it; no bypasses
5. MCP error envelopes: no path disclosure leaks
6. `tracing::warn!` content: no secrets exposed
7. `MAX_ARGUMENTS` / `MAX_DESCRIPTION_MAX_CHARS` ingress: caps enforced at parser boundary
8. Env passthrough namespace prefix `TOME_ENV_` is mandatory; no nested substitution
9. Schema v2→v3 migration: backfill defaults correct + transaction-wrapped

The 4 critical security findings caught during per-US passes all hold their fixes.

## Recommendation

Ship to v0.5.0 after applying:
- M-1, M-2, M-3, M-4 (Rust fixes — PR-B)
- GAP-1, GAP-2, CA-M1 (test fills + contract doc — PR-C)
- m-3, m-4 (Rust minor doc/naming touch-ups — fold into PR-B)

Then docs + release (PR-D) + closeout (PR-E).

# Phase 5 / US5 — Reviewer findings

Consolidated from 4 parallel reviewers run against `main` at `407d1f7`. Source: `/tmp/tome-phase5-us5-{contract,rust,test,security}.md`.

## Tally

| Reviewer | BLOCKER | MAJOR | MINOR | NIT |
|---|---|---|---|---|
| Contract  | 0 | 0 | 0 | — |
| Rust      | 0 | 4 | 6 | 3 |
| Test      | 0 | 2 GAP + 1 WEAK | 0 | — |
| Security  | 0 | 1 MEDIUM | 1 LOW | — |
| **Totals** | **0** | **~7** | **~7** | **3** |

**Cleanest US closeout of Phase 5.** Reviewer pass severity continued its descending trend: US1.d 1B+8M → US2.d 2B+6M → US3.d 0B+2M → US4.d 0B+1H+2M → **US5.b 0B+4M**.

Contract reviewer reported PASS at all severity levels — no contract drifts found in any of the 4 audited contracts.

## Headline findings

### R-M1: Layout constant duplication — `walk_plugin_data_for_orphans`
**File**: `src/doctor/checks.rs:312`
**Issue**: Inline `paths.root.join("plugin-data")` duplicates the convention that lives in `Paths::plugin_data_dir_for`. If on-disk layout moves, this site silently drifts.
**Fix**: Promote `Paths::plugin_data_root() -> PathBuf` accessor; consume from both writers + this walk.

### R-M3: `count_entries_by_kind` two-SELECT non-atomicity
**File**: `src/doctor/checks.rs:415-538`
**Issue**: Two SELECT statements over `conn` without a snapshot transaction. A concurrent writer between the two queries can produce inconsistent counts vs `pending_re_embedding`.
**Fix**: Wrap in `conn.unchecked_transaction()` (safe on `open_read_only` connections); both statements share the snapshot.

### R-M4: Unbounded description in `plugin show`
**File**: `src/commands/plugin/show.rs:114`
**Issue**: `EntryView.description` is the raw DB string. US4.d added `MAX_DESCRIPTION_MAX_CHARS = 100_000` cap to `search_skills`; `plugin show --json` would still ship a 50 MiB description from a hostile catalog. Trust-boundary is "catalog trusted-on-enrol" so this is accepted-risk — but a `tracing::warn!` would surface a misbehaving catalog to operators.
**Fix**: At `list_entries` row-conversion, log `warn!` if description > `MAX_DESCRIPTION_MAX_CHARS`.

### R-M5: `build_phase5_surfaces` ScopeSource comment ambiguity
**File**: `src/doctor/mod.rs:161-168`
**Issue**: The carve-out is correct per the contract (only `ScopeSource::GlobalFallback` returns `(None, None, None)`), but the comment reads as "outside a workspace context" which is broader than the actual behaviour. Explicit `--workspace global` correctly flows through the populated path.
**Fix**: Tighten doc comment to specifically reference `GlobalFallback`.

### T-G1: Missing negative test for `[dormant]` annotation
**File**: `tests/plugin_show_p5.rs`
**Issue**: Positive case is tested (both flags false → `[dormant]`); negative case (at least one flag true → no `[dormant]`) is not.
**Fix**: Add `dormant_not_annotated_when_searchable_true` test.

### T-G2: Empty-section behaviour untested
**File**: `tests/plugin_show_p5.rs`
**Issue**: Skills-only and commands-only tests don't assert the complementary section is an empty array. Contract specifies both arrays always present.
**Fix**: Extend `skill_default_flags_in_json` + `command_default_flags_and_derived_prompt_name` to assert the empty complementary section.

### T-W1: Entry-count assertion too loose
**File**: `tests/doctor_p5.rs:211`
**Issue**: `assert!(counts.skills >= 1, ...)` instead of exact count. Sample catalog is deterministic.
**Fix**: Verify exact fixture count, tighten to `assert_eq!`. (`pending_re_embedding` >=1 assertion is acceptable per reviewer — heuristic by design.)

### S-MEDIUM: Symlink path disclosure in doctor output
**File**: `src/commands/doctor.rs:309,315`
**Issue**: Doctor output emits absolute paths to orphan dirs. Walks SKIP symlinks (correct), but path is still disclosed to stdout. Under threat model this is accepted-risk; security reviewer recommends doc-only carve-out.
**Fix**: Defer to v0.6+ with cap-std hardening. No US5.c action.

### S-LOW: Terminal escape sequences in plugin/entry names
**File**: `src/commands/plugin/show.rs:299-327`
**Issue**: Plugin/entry names rendered as-is; could contain ANSI codes from a hostile catalog. Design choice, not a security boundary.
**Fix**: Defer. No US5.c action.

## Minor findings (Rust)

- **R-m2**: `n.max(0)` in `u32::try_from(n.max(0))` is unreachable defensive code — SQLite `COUNT(*)` is non-negative. Two sites: `src/doctor/checks.rs:448` + `src/commands/plugin/mod.rs:263`. **APPLY**
- **R-m3**: `read_dir` errors silently bail in 3 sites (`check_catalogs:98`, `walk_plugin_data_for_orphans:335`, `walk_workspace_plugin_data_for_orphans:386`). Add `tracing::warn!`. **APPLY**
- **R-m4**: `std::collections::HashSet<std::path::PathBuf>` fully qualified at 4 sites; `use` statement already covers it. Cosmetic. **APPLY**
- **R-m5**: `IndexHealth.size_bytes: 0` on synthetic check_index error path; should read `fs::metadata().len()`. **APPLY**
- **R-m10**: `skill_count: Option<u32>` annotated `#[allow(dead_code)]` in `commands/plugin/list.rs`; emitter doesn't consume it. Dead code. **APPLY**
- **R-m11**: `/mcp__tome__` prefix hard-coded at `commands/doctor.rs:272`. Promote to `pub const MCP_SLASH_PREFIX` in `src/mcp/mod.rs`. **APPLY**
- **R-n3**: `src/mcp/prompt_collision.rs:25` doc comment is now slightly stale (the type IS reached by doctor's `PromptsReport`). **APPLY** (one-line touch)

## Deferred — full rationale in `us5-disposition.md`

- **R-M2** (orphaned-workspace-dir flood): contract amendment + new field on `OrphanDataDirReport`. Out of US5.c scope; deferred to v0.6+ Polish.
- **R-m1** (per-iteration allocations in `walk_plugin_data_for_orphans`): perf-not-material for read-only diagnostic.
- **R-m6** (`out.dedup()` after `out.sort()` in `collect_detected_uninstalled`): cosmetic dead-defensive code; `inventory::submit!` already enforces uniqueness; documented invariant defence.
- **R-m7** (duplicate path-safety check vs `resolve_entry_body_path`): promote-helper refactor; defer until 3rd consumer at v0.6+.
- **R-m8** (`prompt_name` skip_serializing_if): contract reviewer confirmed `null` is acceptable wire shape; no action.
- **R-m9** (`Some().as_deref().map()` chain): cosmetic.
- **R-n1** (trailing comma): rustfmt-correct; no action.
- **R-n2** (`classify_pub` naming): works as-is; renaming is its own micro-refactor.
- **MINOR** (Subsystem owned-String alloc cost): perf-not-material; consistent with closed-set discipline.
- **MINOR** (`PromptsReport` doc comment relocation): cosmetic.
- **S-MEDIUM** (orphan-path symlink disclosure): documented as v0.6+ cap-std hardening dependency.
- **S-LOW** (terminal escape sequences in names): design surface; no security boundary.

## Pre-approved carve-outs not flagged

- Concurrent bind/enable tests (already covered by `tests/plugin_workspace_skills.rs` + `tests/workspace_use_concurrent.rs`).
- Real BGE model verification (deferred to manual per US1 disposition; SC-001 / SC-002).
- cap-std / filesystem TOCTOU hardening (deferred to v0.6+ per US4.d).
- Output `Serialize` types without `#[serde(deny_unknown_fields)]` (strictness boundary, Tome-owned inputs only).
- `pub static OnceLock<Mutex<...>>` test-injection slots, RAII guards, `PoisonError::into_inner` recovery (established pattern).
- `tempfile::Builder::new().prefix(".tome.tmp.")` + `keep()` + `std::fs::rename` atomic-populated-directory idiom.

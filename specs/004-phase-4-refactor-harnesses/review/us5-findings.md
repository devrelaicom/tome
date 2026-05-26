# US5 — Pre-Closeout Review Findings

Four reviewers dispatched in parallel against the merged US5 surface (PRs #99–#100).

Per-reviewer source files in `/tmp/tome-review-us5-{contract,rust,test,security}.md`.

Counts: **1 blocker, 21 majors, ~25 minors+nits.**

Triage in `us5-disposition.md`.

---

## Blockers (1)

| # | Source | One-line | File / cite |
|---|---|---|---|
| **T-B1** | test | No `apply`-path test for `Subsystem::Summariser` repair (FR-562 — five Phase 4 fix classes, only 4 tested). Without coverage of the summariser repair branch, a regression there would be invisible. | `tests/doctor_fix_p4.rs` — no `summariser_fix_redownloads` (env-gated OR stub) |

## Majors (21)

### Contract audit (3)

| # | One-line | File ref |
|---|---|---|
| C-M1 | `SubsystemHealth::NotApplicable` declared + documented but NEVER assigned in source. Outside-project case produces empty Vecs instead of per-harness `NotApplicable` entries; wire can't distinguish "no harnesses declared" from "outside project". | `src/doctor/mod.rs:120` + `data-model.md §15` |
| C-M2 | `SchemaTooNew(52)` propagates out of `doctor::assemble_report` via `check_index?`, violating FR-561 "doctor never crashes". Every other check collapses gracefully; only `check_index` short-circuits. | `src/doctor/mod.rs:95` |
| C-M3 | `repair_binding_rules_copy` calls `workspace::sync::sync_one` which walks EVERY bound project of the workspace — contradicts FR-562 "project-local" + Fix Classes table's singular `<project>/.tome/RULES.md` phrasing. Silently overwrites sibling projects. | `src/doctor/fixes.rs:262-273` |

### Rust-lens (7)

| # | One-line | File ref |
|---|---|---|
| R-M1 | `--force` without `--fix` returns exit 7 (Io) instead of exit 2 (Usage). | `src/commands/doctor.rs:50` |
| R-M2 | `apply()` calls `repair_harness_sync` once per HarnessRules/HarnessMcp suggestion — 10 redundant sync passes when 1 would do (5 harnesses × 2 types). | `src/doctor/fixes.rs:167-184` |
| R-M3 | `re_assemble` never refreshes `report.drift`; future Schema-arm migrations won't have post-repair classification updated. | `src/doctor/mod.rs` |
| R-M4 | `&scope.scope.name().clone()` gratuitous clone-then-borrow. | `src/doctor/mod.rs:115` |
| R-M5 | `compare_rules` collapses "source missing" with "copy missing" → `--fix` infinite-loops when only the workspace source is absent. | `src/doctor/binding.rs` |
| R-M6 | Orphan-cleanup TOCTOU window with paused writers — flag in `STAGING_AGE_GATE` docs. | `src/doctor/orphan_cleanup.rs` |
| R-M7 | `repair_catalog` uses writable `index::open` for read-only enrolment lookup; should use `open_read_only`. | `src/doctor/fixes.rs` |

### Test audit (7)

| # | One-line | File ref |
|---|---|---|
| T-M1 | No CLI binary test for user-owned-MCP exit 75 (only library API). | `tests/doctor_fix_p4.rs` |
| T-M2 | No test for `--force` without `--fix` rejection. | "no test" |
| T-M3 | `accepts_query_exactly_at_cap` has only conditional assertion that passes on broken impls. | `tests/mcp_input_length_caps.rs` |
| T-M4 | T376b 5×9 matrix has 3 cells effectively non-checks (`rules_file_target` accepts any project-rooted path; `mcp_config_path` only checks non-empty; `description_starts_with: ""` is vacuous). | `tests/harness_modules.rs` |
| T-M5 | No Summariser-drifted test via `--verify` (FR-561 third state). | "no test" |
| T-M6 | FR-563 read-only walk excludes directories. | `tests/doctor_read_only_by_default.rs` |
| T-M7 | STAGING_AGE_GATE boundary untested. | `tests/doctor_orphan_tmp_cleanup.rs` |

### Security audit (4)

| # | One-line | File ref |
|---|---|---|
| S-M1 | Orphan-cleanup uses `workspace_projects.project_path` DB rows without re-canonicalisation at read time; canonicalisation only happens at `bind_project` write. Future migration / direct edit could leave non-canonical entries sweep follows blindly. | `src/doctor/orphan_cleanup.rs` |
| S-M2 | `--fix --force` rewrites EVERY user-owned harness MCP entry in one pass even when only one harness's specific fix triggered consent. | `src/doctor/fixes.rs` |
| S-M3 | Developer-authored harness config files read unbounded (S2-M3 carry-over with five user-stories of accumulated exposure). | multi-site |
| S-M4 | `repair_catalog::remove_dir_all` against a DB-derived path is safe today but the cache-path-inside-catalogs-dir invariant is implicit. Add `debug_assert!` for defence in depth. | `src/doctor/fixes.rs` |

---

## Verdict

US5 surface is solid. The orphan-cleanup `remove_dir_all` path was the highest-risk new attack surface; reviewers confirm it's well-built (STAGING_PREFIX + 1h mtime + symlink-skipping + is_dir() check + 0o700 staging perms compose correctly). The `--force` MCP override is precisely scoped to HarnessMcp(_). Classification ladder is contract-correct.

Plan: US5.c-1 ships the 1 blocker + ~10 selected majors; US5.c-2 closeout (sdd:map + retro + CLAUDE.md). After US5.c-2 closes, Phase 4 feature work is fully complete; Polish phase per `tasks.md` opens.

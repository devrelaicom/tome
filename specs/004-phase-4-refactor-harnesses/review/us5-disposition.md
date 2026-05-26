# US5 — Disposition

Maps findings in `us5-findings.md` to actions.

## Blockers — applied in US5.c-1

| # | Disposition |
|---|---|
| T-B1 | Apply: add `summariser_fix_redownloads_or_documents_env_gate` test to `tests/doctor_fix_p4.rs`. Two paths: (a) env-gated `TOME_TEST_REAL_MODELS=1` path that actually re-downloads; (b) stub path via `SummariserOverrideGuard` or by pre-creating a placeholder model file + asserting `--fix` invokes the download branch. Pick (b) for CI coverage; document (a) as the manual integration path. |

## Majors — applied in US5.c-1 (10)

| # | Disposition |
|---|---|
| C-M1 | Apply: in `assemble_report`, when scope source is GlobalFallback (no project), emit per-harness entries with `SubsystemHealth::NotApplicable` for every harness in the global effective list. Wire distinguishes "no harnesses declared globally" (empty Vec) from "harnesses declared but no project context" (NotApplicable). |
| C-M2 | Apply: change `check_index?` in `assemble_report` (line 95) to `check_index.unwrap_or_else(|err| IndexHealth::Broken { reason: err.to_string() })` so SchemaTooNew(52) collapses gracefully like every other check. Doctor never crashes per FR-561. |
| C-M3 | Apply: `repair_binding_rules_copy` should NOT iterate every bound project. Add `workspace::sync::sync_one_project(project_root, source_rules_md, paths)` that targets ONE project. Use this in the binding-rules-copy repair. Existing `sync_one` keeps the workspace-broadcast semantic; the per-project variant is new. |
| R-M1 | Apply: in `commands::doctor::run`, detect `args.force && !args.fix` and return `TomeError::Usage("--force requires --fix")` (exit 2). |
| R-M2 | Apply: deduplicate harness sync invocations in `apply()`. Collect all `HarnessRules(name)` + `HarnessMcp(name)` suggestions; run `sync_for_project_root` ONCE; clear all affected fixes from the residual list. |
| R-M5 | Apply: in `binding::compare_rules`, distinguish "source missing" (workspace's RULES.md absent) from "copy missing" (project's `.tome/RULES.md` absent). Return a typed enum or two separate `RulesCopyState` variants. The `--fix` path should NOT attempt to copy when source is absent (would infinite-loop); instead surface a different SuggestedFix message: "workspace `<name>`'s RULES.md is empty — run `tome workspace regen-summary <name>` first". |
| R-M7 | Apply: `repair_catalog` uses `index::open_read_only` for the enrolment lookup; `index::open` only when actually writing. |
| S-M2 | Apply: `--fix --force` rewrites only the HarnessMcp entries that have outstanding `UserOwned` SuggestedFix entries in this run. Don't blanket-rewrite every user-owned entry. |
| S-M4 | Apply: add `debug_assert!(cache_dir.starts_with(&paths.catalogs_dir))` to `repair_catalog::remove_dir_all`. |
| T-M1 | Apply: extend `tests/exit_codes_e2e.rs` with CLI binary test for user-owned-MCP exit 75 + `--fix --force` rewrite exit 0. |

## Majors — deferred to follow-up issue

| # | Reason for deferral |
|---|---|
| R-M3 | re_assemble doesn't refresh drift — future Schema migration concern; defer until first real Phase 4+ migration arrives. |
| R-M4 | Gratuitous clone-then-borrow — cosmetic. |
| R-M6 | Orphan-cleanup TOCTOU doc note — documentation only. |
| S-M1 | Re-canonicalisation of workspace_projects.project_path at read time — defensive against future migration / direct DB edit; defer. |
| S-M3 | Unbounded reads on harness config files — multi-site fix; carries forward from US1.d-2a S-M1 / US2.d-1 S-M3 / US3.d-1 S-M3 deferrals. Phase 4 Polish phase. |
| T-M2/T-M3/T-M4/T-M5/T-M6/T-M7 | Various test gaps that don't represent functional regressions; defer to test-hardening follow-up. |

## Minors + nits

Bulk-deferred to a tracking issue per individual reviewer files.

## Net effect of US5.c-1

- ~6 production-source touches (`commands/doctor.rs`, `doctor/mod.rs`, `doctor/fixes.rs`, `doctor/binding.rs`, `workspace/sync.rs`)
- ~3-5 new tests + 1 test extension
- 2 review artefacts (this + findings.md)

US5.c-2 runs `/sdd:map incremental` + retro/CLAUDE.md updates. After that closes, Phase 4 feature work is fully complete and the Polish phase opens per `tasks.md`.

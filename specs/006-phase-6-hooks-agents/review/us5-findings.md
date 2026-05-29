# US5 (Privilege governance + doctor extensions) — Reviewer findings

4-reviewer parallel pass against the US5 surface (`39153b2..HEAD`). Recorded before any fix. No BLOCKER; security APPROVE (0 findings).

## Contract
- **[MAJOR] C5-1** `--fix` does not refresh the Phase 6 surfaces when harness fixes also run. The Phase 6 refresh block (`fixes.rs:208`) is gated on `!harness_sync_ran`; when a HarnessRules/HarnessMcp fix runs, `harness_sync_ran` is true and the block is skipped — yet that same sync already re-rendered guardrails / re-emitted agents, so `report.hooks/guardrails/agents` retain pre-repair values. The post-`--fix` report (human + JSON) then misreports the Phase 6 state (contradicts "the affected check is re-run after each repair", FR-091). Fix: refresh the Phase 6 project surfaces whenever a project-context sync succeeded, regardless of which branch triggered it.
- **[MINOR] C5-2** `build_guardrails_report` `suppressed` = `present_keys ∩ plugins_with_hooks_json`, but a correctly-synced Claude Code plugin shipping `hooks.json` has NO region on disk, so its key is absent from `present_keys` → `suppressed` only ever populates in the stale/drift case, never steady-state. Confirm intent: if `suppressed` should audit "enabled plugins currently suppressed on CLAUDE.md", derive it from the enabled set (plugins shipping BOTH GUARDRAILS.md + hooks.json) independent of on-disk presence. (Rust-lens flagged the same as a dead branch + misleading comment.)

## Rust-lens — APPROVE
- Privilege strip provably leak-free: `emit_agents_for_harness` clears the three fields on a per-emission `agent.canonical.clone()`; the shared `prepared: &[PreparedAgent]` is borrowed immutably (borrow-checker-enforced), so the audit source is untouched. `m.name()=="claude-code"` gate correct.
- The 5 check fns are read-only, graceful (per-entry `continue`), no quadratic blowup; `build_phase6_surfaces` mirrors Phase 5 correctly.
- **[MINOR] R5-1** `build_privilege_escalation_report` grouping is O(agents×plugins) via linear `find` — bounded by privileged-agent count + order-preserving; accept, note.
- **[MINOR] R5-2** = C5-2 (dead suppress branch / misleading comment).
- **[MINOR] R5-3** `phase6_has_repairable_drift` triggers a full `sync_project` re-run on ANY present region / any enabled agent (not just drift) — idempotent + safe (`force=false`, structural-match-only, no catalog repair reachable), so needless-work not destructive. Could tighten to orphaned-only; defer.
- Confirmed: closed-set dispatch no-catch-all; JSON byte-stability (appended `Option` + skip-if-none); promotions clean; no bad unwrap/panic; overflow handling correct.

## Test
- **[MAJOR] T5-1** `hooks_report_contributed_and_drift` tests drift at EVENT granularity (drops a whole event, keeps a sibling event). The impl IS entry-identity correct (`arr.iter().any(|e| e == entry)`) but the test would pass a buggy event-granularity impl. Add a WITHIN-event case: an event with two entries, one user-edited (so it won't `==`) → same event in BOTH `contributed` (1) and `missing` (1).
- **[MAJOR] T5-2** `AgentsReport.dropped_fields` never surfaced non-empty through a real translation (claude-code drops nothing; the JSON pin hand-builds it). Add a Codex/Cursor harness + an enabled agent with `model:`/`tools:` → assert `build_agents_report` populates `dropped_fields`.
- **[MINOR] T5-3** Doctor `PersonaReport` clash-prefix branch not integration-tested (only the non-clash case + a hand-built pin). Add two same-named agents from two plugins → `clash_prefixed==true`, `<plugin>-reviewer-persona`.
- **[MINOR] T5-4** Post-sync suppressed-region assertion punted (`hooks_and_guardrails_and_agents_reports_after_sync` only asserts `is_some()`). Fold into the C5-2 fix's test.
- Well-covered: strip passthrough/strip/no-op/non-claude-code (both scopes); privilege report unchanged-when-strip-on (reads source); persona toggle; read-only creates-no-dirs; outside-project None; all 5 JSON pins + plugin-show pin; exit-45 via binary; --fix re-render/re-emit/orphan-removal/never-removes-unowned-hook/never-deletes-user-content. exit-75 correctly NOT a Phase 6 gap (informational surfaces).

## Security — APPROVE (0 findings)
- FR-051 audit reads the agent SOURCE independent of the strip setting (the strip is a per-emission clone) — the escalation surface is never hidden.
- `--fix` re-sync: `force=false`, hook removal structural-match-only, orphan removal literal `<plugin>__*` prefix (not glob), all writes/deletes symlink-refused, single-path `remove_file` (no `remove_dir_all`). Cannot delete user data or follow symlinks in a real scenario.
- The chunk-2 "symlinked-cache" hazard is a TEST-FIXTURE artifact: `remove_dir_all` lives only in `repair_catalog` (Phase 4, unreachable from the Phase 6 `--fix` branch) and is content-addressed under Tome's own catalogs dir. Not user data, not a US5 regression.
- Read-only (FR-124) holds; strip correctly documented as config convenience not enforcement; plugin show read-only no info-leak.

**Overall**: no blocker, security clean; 1 contract MAJOR (`--fix` stale Phase 6 report), the suppress-semantic MINOR (2 reviewers), 2 test MAJORs (entry-identity drift; real dropped_fields). All fixable cleanly.

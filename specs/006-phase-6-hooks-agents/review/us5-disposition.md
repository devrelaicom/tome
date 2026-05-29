# US5 (Privilege governance + doctor extensions) — Disposition

Decisions on the [findings](./us5-findings.md). Committed before fixes. Final user-story reviewer pass.

## Fix now

| ID | Sev | Decision | Action |
|---|---|---|---|
| C5-1 | MAJOR | **Fix** | Refresh the Phase 6 project surfaces (`report.hooks`/`guardrails`/`agents`) whenever a project-context `sync_project` succeeded during `--fix`, regardless of whether it was triggered by the harness-fix branch or the Phase 6 branch — so the post-`--fix` report reflects post-repair state (FR-091). |
| C5-2 / R5-2 | MINOR→fix | **Fix** | Make `build_guardrails_report.suppressed` a steady-state audit: for the Claude Code target, list enabled plugins that ship BOTH `GUARDRAILS.md` AND `hooks/hooks.json` (region intentionally absent because hooks supersede), independent of on-disk presence. Drop the misleading dead early-out/comment. |
| T5-1 | MAJOR | **Fix** | Add a within-event hooks-drift test (two entries under one event, one user-edited → same event in both `contributed` and `missing`). |
| T5-2 | MAJOR | **Fix** | Add a real non-empty `dropped_fields` test (Codex or Cursor harness + an agent carrying `model:`/`tools:` → `build_agents_report` populates `dropped_fields`). |
| T5-3 | MINOR→fix | **Fix** | Add the doctor `PersonaReport` clash-prefix integration test. |
| T5-4 | MINOR→fix | **Fix** | Assert the steady-state suppressed entry in the post-sync guardrails report (folds with the C5-2 fix). |

## Defer / accept

| ID | Decision | Rationale |
|---|---|---|
| R5-1 | **Accept + note** | Privilege grouping O(agents×plugins) is bounded by privileged-agent count and preserves enumeration order; a HashMap would not. |
| R5-3 | **Accept (note for Polish)** | The `--fix` re-sync heuristic re-runs on any present region/enabled agent — idempotent + safe (`force=false`, no destructive op reachable); needless-work only. Could tighten to orphaned-only in Polish. |
| chunk-2 symlinked-cache | **No action** | Confirmed by security as a test-fixture artifact; `repair_catalog` is Phase 4, unreachable from the Phase 6 `--fix` branch, content-addressed under Tome's own dir. |

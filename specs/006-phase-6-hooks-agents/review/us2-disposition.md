# US2 (Real Claude Code hooks) — Disposition

Decisions on the [findings](./us2-findings.md). Committed before fixes.

## Fix now

| ID | Sev | Decision | Action |
|---|---|---|---|
| R2-2 | MINOR→fix | **Fix** | Fail closed: if either rewrite-target path is non-UTF-8, return `HookSettingsWriteFailed` (exit 44) instead of `to_string_lossy` emitting a silently-broken executed command. |
| R2-1 | MAJOR | **Document + defer** | No clean fix under the no-sidecar ownership model (NFR-003): removal needs the source to re-derive the deep-equal entry. Expand the code comment to cover BOTH the `Err` and `NotFound→Ok(None)` arms; add a CONCERNS entry; the US5 doctor `HooksReport` is the surfacing path. (No code behavior change — accepted limitation.) |
| T2-1 | MAJOR | **Fix** | Add symlink-refusal tests for `settings.local.json` write (merge + remove) → exit 7, target unchanged. |
| T2-2 | MAJOR | **Fix** | Add hooks forward-progress test (one malformed + one good plugin → good entry merges, sync exits 43). |
| T2-4 | MINOR→fix | **Fix** | Add exit-44 behavioral test via a malformed / wrong-type existing `settings.local.json`; assert original left byte-intact. |
| T2-3 | MINOR→fix | **Fix** | Add multi-plugin single-sync merge test. |
| T2-5 | MINOR→fix | **Fix** | Add multi-event partial-prune test (event A pruned, user entry under event B survives). |
| T2-6 | MINOR→fix | **Fix** | Add source-symlink refusal test → exit 7 (cheap). |

## Defer / accept

| ID | Decision | Rationale |
|---|---|---|
| R2-3 / C2-2 | **Accept (no code change)** | Empty-contribution = no `settings.local.json` is better hygiene; note the contract divergence in the retro. |
| R2-4 | **Accept** | Contrived (a dir literally named `${CLAUDE_PLUGIN_DATA}`); impossible for a real `~/.tome/...` path. |
| C2-1 | **Accept** | Fixed-needle `str::replace` is outcome-identical, safe, and ReDoS-free; contract text is descriptive. Keep. |
| C2-3 | **Accept** | Unreadable-source→43 is intentional and documented in code. |

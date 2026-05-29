# US4 (Agent personas via MCP prompts) — Disposition

Decisions on the [findings](./us4-findings.md). Committed before fixes.

## Fix now

| ID | Sev | Decision | Action |
|---|---|---|---|
| C4-1 + R4-1 | MAJOR | **Fix together** | Add `plugin_version` AND `indexed_at` to the `enabled_agents_for_workspace` SELECT + the `EnabledAgent` struct. Thread `plugin_version` onto the persona `PromptEntry` (so `${TOME_PLUGIN_VERSION}` resolves) and the real `indexed_at` onto the persona `EntryIdentity` (so persona-vs-command/skill collisions tie-break by `indexed_at ASC` per FR-062). Keep `indexed_at: String::new()` ONLY for the reserved `drop-persona` identity. |
| R4-2 | MAJOR | **Fix** | Promote `load_project_marker`/`load_workspace_settings`/`load_global_settings` to a shared `settings` module (`pub(crate)`); call from `resolve_expose_personas`, `commands::harness::list`, and `harness::sync`. Single source for the NotFound/parse-error arms. |
| T4-1 | MAJOR | **Fix** | Add an FR-067 startup-scope integration test: write project/workspace/global settings files, assert `resolve_expose_personas` returns the startup-scope value (project layering has no effect), and that the persona registry is built only when the resolved scope declares `true`. |
| C4-2 + T4-4 | MINOR→fix | **Fix** | Protect the `-persona` suffix from the 48-char override truncation (truncate the base, keep the suffix). Add the length-backstop test. |
| T4-2 | MINOR→fix | **Fix** | Assert the frontmatter is stripped in `get_wraps_and_substitutes`. |
| T4-3 | MINOR→fix | **Fix** | Add the persona-path warn-and-skip test (unresolvable agent → registry builds, survivors + drop-persona present). |

## Defer / accept

| ID | Decision | Rationale |
|---|---|---|
| R4-3 | **Defer to US5** | Doctor's `false` is acceptable for the read-only collision surface; US5's `PersonaReport` makes doctor persona-aware. |
| C4-3 | **Accept (doc nit)** | Tests exist under `tests/personas*.rs`; the contract table's filenames are stale. Note only. |
| S4-1 | **Accept + document** | Conversational closing-tag/display-name breakout is an in-band LLM-context limitation (not a re-parsed file); double opt-in + advisory caveat. Note in retro, parity with the Phase 5 `$ARGUMENTS` caveat. |

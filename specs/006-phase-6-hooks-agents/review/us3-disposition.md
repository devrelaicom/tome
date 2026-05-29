# US3 (Guardrails + rules-file correction) — Disposition

Decisions on the [findings](./us3-findings.md). Committed before fixes.

## Fix now

| ID | Sev | Decision | Action |
|---|---|---|---|
| B-1 | BLOCKER | **Fix** | Fail closed in `read_guardrails_source`: scan each body line against the guardrails START regex, the key-agnostic END regex, and `rules_file::BLOCK_MARKER_REGEX` (`tome:begin/end`); if any matches, return `GuardrailsWriteFailed { path: <source> }` (exit 46) — loud-but-isolated (sibling plugins still reconcile via forward-progress). Add negative tests: region-escape body, stray-END wedge body, `tome:begin` body — assert exit 46 naming the source AND that a legitimate sibling plugin's region still reconciles + a re-sync stays convergent. |
| R3-1 | MAJOR | **Fix** | `compose_in_file` parse-error must carry the real `target` path, not `PathBuf::new()`. Thread `target` in (or wrap at the caller as the other arms do). |
| T3-2 | MAJOR | **Fix** | Add the atomicity test: injected mid-write failure → exit 46, target byte-for-byte unchanged (no partial region between markers). |
| T3-3 | MAJOR | **Fix** | Extend `both_transitions_in_one_sync` with codex: assert both plugins' regions persist on the shared `AGENTS.md` across both suppression transitions (CLAUDE.md-only suppression). |
| C3-1 | MINOR→fix | **Fix (doc)** | Update `contracts/guardrails.md:87,109` symlink refusal exit 7 → exit 46, reconciling with the authoritative `exit-codes-p6.md` (guardrails-target IO → 46). No code change. |
| R3-2 | MINOR→fix | **Fix** | Drop the `classify` `unwrap_or(false)` re-parse swallow; thread `had_regions`/`seen_keys` out of `compose_in_file`. |
| T3-4 | MINOR→fix | **Fix** | Add gemini guardrails target test (no AGENTS.md → GEMINI.md; with AGENTS.md → shared). |
| T3-6 | MINOR→fix | **Fix** | Add changed-source overwrite-in-place integration test. |
| T3-5 | MINOR→fix | **Fix** | Add two-plugin Cursor-sibling lex-ordered render test. |
| T3-7 | MINOR→fix | **Fix** | Add verbatim-body fidelity test (frontmatter/heading/include/trailing-ws — bytes preserved). Note: bodies with marker-shaped lines are now rejected by B-1, so this uses non-marker parseable-looking content. |

## Defer / accept

| ID | Decision | Rationale |
|---|---|---|
| R3-3 | **Defer (Polish)** | Clone-heavy rebuild is a pre-existing `compose_block_write` pattern; allocation optimisation is Polish-scope, correctness is fine. |
| R3-4 | **Accept + note** | The Cursor sibling is contractually a fully-Tome-owned file at a Tome-specific path (`.cursor/rules/TOME_GUARDRAILS.md`), deleted-when-empty per FR-015; symlink-refused. A user manually authoring content at that exact Tome path is an accepted edge; note in retro. |

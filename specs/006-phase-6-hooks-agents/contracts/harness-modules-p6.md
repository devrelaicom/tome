# Phase 6 — HarnessModule trait extensions

How the existing Phase 4 `HarnessModule` trait (`src/harness/mod.rs`) grows to host hooks, guardrails, and native-agent capabilities, plus the Phase 4 rules-file correction. No new top-level module — these methods cluster on the existing trait behind which the five harness modules already live (research R-2, plan §Structure Decision).

## Trait additions

All defaults make a brand-new harness **safe-by-default**: guardrails-only, no real hooks, no native agents. New harnesses inherit the conservative behaviour without touching a line.

| Method | Returns | Default impl | FR |
|---|---|---|---|
| `hooks_strategy()` | `HooksStrategy { RealJson, GuardrailsOnly }` | `GuardrailsOnly` | FR-001, FR-013 |
| `hook_settings_path(project)` | `Option<PathBuf>` | `None` | FR-002 |
| `guardrails_target(project)` | `GuardrailsTarget` | `InFileRegion` on the harness's rules-file target, `suppress_if_hooks_present = false` | FR-011, FR-012 |
| `supports_native_agents()` | `bool` | `false` | FR-030 |
| `agent_dir(project)` | `Option<PathBuf>` | `None` | FR-031 |
| `agent_format()` | `Option<AgentFormat { MarkdownYaml, Toml }>` | `None` | FR-030, FR-033 |
| `translate_agent(canonical)` | `TranslatedAgent` | only ever called when `supports_native_agents()` is true; non-supporting harnesses need no override | FR-030, FR-032 |

`GuardrailsTarget` (data-model §3): `{ placement: GuardrailsPlacement, suppress_if_hooks_present: bool }` where `GuardrailsPlacement` is `InFileRegion { file }` or `StandaloneSibling { file }`. The `suppress_if_hooks_present` flag is true only for claude-code (FR-013).

`detect_path(&home)` already exists (Phase 4 PR-C) and is unchanged.

## Per-harness capability table

Exact directories and formats are verified against current harness documentation **at implementation time** — the Phase 4 ecosystem caveat: the ecosystem moves fast (spec Assumptions; FR-031 explicitly defers final path confirmation to implementation).

| Harness | `hooks_strategy` | Guardrails target file | `supports_native_agents` | `agent_dir` | `agent_format` |
|---|---|---|---|---|---|
| `claude-code` | `RealJson` | `CLAUDE.md` (in-file region, suppress-if-hooks) | `true` | `.claude/agents/` | `MarkdownYaml` |
| `codex` | `GuardrailsOnly` | `AGENTS.md` (in-file region) | `true` | `.codex/agents/` | `Toml` |
| `cursor` | `GuardrailsOnly` | `.cursor/rules/TOME_GUARDRAILS.md` (standalone sibling) | `true` | `.cursor/agents/` | `MarkdownYaml` |
| `gemini` | `GuardrailsOnly` | `AGENTS.md` preferred else `GEMINI.md` (in-file region) | `false` | — | — |
| `opencode` | `GuardrailsOnly` | `AGENTS.md` (in-file region) | `true` | `.opencode/agent/` (singular `agent/`) | `MarkdownYaml` |

Notes:

- Only **claude-code** returns `RealJson`; its `hook_settings_path` is `.claude/settings.local.json` (gitignored, machine-local — never the committed `.claude/settings.json`). Every other harness is `GuardrailsOnly` with `hook_settings_path = None` (FR-001, FR-002).
- The **Cursor** guardrails sibling (`TOME_GUARDRAILS.md`) is distinct from the Phase 4 skills sibling (`TOME_SKILLS.md`); each plugin is still individually marker-wrapped so per-plugin removal works, and the file is deleted when no plugin contributes (FR-012, FR-015).
- **Gemini** has no native agent support — guardrails and optional personas only (it is sunsetting for individuals; PRD non-goals). Antigravity and the Phase 2 harnesses are likewise excluded from native agents.
- **OpenCode**'s directory is the singular `agent/`, and its agent name is filename-derived, so its agents are always plugin-prefixed (FR-042); see agent-translation.md.

## Phase 4 rules-file correction (FR-020 / FR-021 / FR-022, research R-6)

Claude Code does not natively read `AGENTS.md`. Phase 4 listed `claude_code`'s rules-file candidate array as `AGENTS.md > CLAUDE.md`, so a project with `AGENTS.md` and no `CLAUDE.md` had its `tome:begin/end` rules-include block (and would have had its guardrails) invisible to Claude Code. Phase 6 corrects the existing `claude_code` candidate array:

- **New candidate precedence**: `CLAUDE.md` > `.claude/CLAUDE.md` (first existing wins; create `CLAUDE.md` when none exist). The existing-rules-file precedence from Phase 4 continues to apply **only within this corrected Claude-Code-own set** — an existing `CLAUDE.md` is reused and its block updated in place.
- **`AGENTS.md` MUST NOT appear** anywhere in `claude_code`'s candidate set under any precedence. This is the substance of the correction.
- **Codex, Gemini, and OpenCode** keep sharing a single `AGENTS.md` rules-include block. The `claude_code` `CLAUDE.md` block and the shared `AGENTS.md` block both resolve the **same** `.tome/RULES.md` — two small include directives, no duplicated rules content (NFR-009).
- **No `@AGENTS.md` scaffolding**: Tome MUST NOT introduce a transitive `CLAUDE.md → @AGENTS.md → @RULES.md` chain and MUST NOT depend on Claude Code ever shipping native `AGENTS.md` support. Scaffolding a `CLAUDE.md` that imports a user's hand-written `AGENTS.md` is out of scope.

Both the Phase 4 rules-include block and the Phase 6 guardrails region for claude-code therefore land in `CLAUDE.md`.

## Sync reconciliation order (FR-016, FR-084, FR-081, research R-13)

Within a single harness sync, `harness::sync` reconciles the three sinks per harness in the **fixed order**:

```
hooks  →  guardrails  →  agents
```

- The hooks-presence determination that drives Claude Code's guardrails suppression predicate (FR-013) MUST be computed **before** guardrails are reconciled for `CLAUDE.md`, so the predicate never reads stale state (FR-016). Hooks-first ordering guarantees this. Both suppression transitions are handled symmetrically in one sync: a plugin that begins shipping `hooks.json` has its `CLAUDE.md` guardrails region removed while its hooks are merged; a plugin that ceases to ship `hooks.json` has its hook entries removed and its `CLAUDE.md` region (re-)rendered.
- **Cross-sink forward progress** (FR-084, the Phase 4 binding-then-sync discipline, FR-403): a failure reconciling one sink for one harness MUST NOT roll back sinks already reconciled successfully in the same sync. Reconciliation continues across the remaining harnesses and sinks where possible and surfaces the **first** failure's exit code (43/44/45/46). Each individual file write stays all-or-nothing (atomic, with mode preservation + symlink refusal per the Phase 4 discipline), so a failure never leaves a partially-written file or marker region.
- Sync is **idempotent** across all three sinks (FR-081): a second run with no underlying change rewrites and removes nothing.

## StubHarness (tests, research R-16)

`StubHarness` (`src/harness/stub.rs`) implements the new trait methods so the dispatch pipeline is exercisable without the five real modules, via the existing `HARNESS_MODULES_OVERRIDE` + `HarnessModulesGuard` seam (Phase 4). The stub is configurable per-test to return any combination of `hooks_strategy`, `guardrails_target`, `supports_native_agents`, `agent_dir`, `agent_format`, and a canned `translate_agent` result, so suppression transitions, removal globs, and forward-progress behaviour can be tested against a synthetic harness registry.

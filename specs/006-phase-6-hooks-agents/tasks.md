---
description: "Phase 6 task list — Hooks and Agents"
---

# Tasks: Phase 6 — Hooks and Agents

**Input**: Design documents from `/specs/006-phase-6-hooks-agents/`
**Prerequisites**: plan.md, spec.md, research.md, data-model.md, contracts/ (all present)
**Source PRD**: [PRDs/phase-6.md](../../PRDs/phase-6.md)

**Tests**: INCLUDED. The spec's Development Standards + NFR-011 require integration tests and byte-stable JSON wire-shape pins for every new emit-only type; the project's constitution §VIII mandates integration tests against real fixtures. Heavy paths use the library API + `StubEmbedder` + `HARNESS_MODULES_OVERRIDE`/`StubHarness`; light/exit-code paths use the CLI binary.

**Organization**: Tasks are grouped by user story (priority order from spec.md) to enable independent implementation and testing. The slice shape mirrors plan.md § Pre-emptive slice plan.

## Format: `[ID] [P?] [Story] Description (use <agent>)`

- **[P]**: parallelizable (different files, no incomplete-task dependency)
- **[Story]**: US1–US5; Setup/Foundational/Polish/[GIT] tasks carry no story label
- All Rust source/test work uses the **devs:rust-dev** agent; markdown/contract/doc tasks need no agent.

## Path conventions

Single Rust project: `src/`, `tests/` at repo root. No workspace split.

---

## Phase 1: Setup

**Purpose**: Confirm the feature branch + commit the planning artifacts. (The branch `006-phase-6-hooks-agents` was created during `/sdd:specify`; the planning artifacts from `/sdd:specify` + `/sdd:plan` are currently uncommitted.)

- [X] T001 [GIT] Verify current branch is `006-phase-6-hooks-agents` (`git branch --show-current`); confirm the working tree holds only the expected planning artifacts (`git status --short`: `specs/006-phase-6-hooks-agents/`, `PRDs/phase-6.md`, modified `CLAUDE.md`).
- [X] T002 [GIT] Commit planning artifacts: `git add PRDs/phase-6.md specs/006-phase-6-hooks-agents/ CLAUDE.md && git commit` — `docs(phase-6): spec, plan, research, data-model, 9 contracts, quickstart, tasks`.

**Checkpoint**: Branch is clean, planning artifacts committed, ready for foundational work.

---

## Phase 2: Foundational (blocking prerequisites)

**Purpose**: Pre-allocate the closed-set error codes, land the load-bearing `EntryKind` widening BEFORE any agent row can exist, and stand up the extended `HarnessModule` trait surface. No user story can proceed correctly until F2 lands.

- [X] T003 Create `specs/006-phase-6-hooks-agents/retro/P2.md` from the retro template.
- [X] T004 [GIT] Commit: `docs(phase-6): init P2 retro`.

### F1 — Error codes 43–46 (`contracts/exit-codes-p6.md`)

- [X] T005 Add four `TomeError` variants (illustrative names `HookSpecParseError`, `HookSettingsWriteFailed`, `AgentTranslationFailed`, `GuardrailsWriteFailed`) in `src/error.rs` (use devs:rust-dev agent).
- [X] T006 Map the four variants to exit codes 43, 44, 45, 46 in the `ExitCode` mapping in `src/error.rs`; add the `// Phase 6 — hooks + agents (codes 43–46)` cluster comment (use devs:rust-dev agent).
- [X] T007 [P] Extend `tests/exit_codes.rs` with variant→code assertions for 43–46 (use devs:rust-dev agent).
- [X] T008 [GIT] Commit: `feat(phase-6): pre-allocate hooks/agents error variants + exit codes 43–46`.

### F2 — `EntryKind::Agent` widening (load-bearing, FR-070a)

- [X] T009 Add the `Agent` variant to `EntryKind` in `src/plugin/identity.rs`; extend `FromStr`/`Display` with `"agent"` (use devs:rust-dev agent).
- [X] T010 Widen every exhaustive `match EntryKind` site to handle `Agent` (per-kind count aggregation in `src/commands/plugin/mod.rs` + `src/commands/plugin/show.rs`; the doctor entry-count surface in `src/doctor/checks.rs`); no catch-all — preserve canonical-enum-dispatch (use devs:rust-dev agent).
- [X] T011 [P] Add a unit test in `src/plugin/identity.rs` for `EntryKind::Agent` round-trip (`FromStr`/`Display`) (use devs:rust-dev agent).
- [X] T012 [P] Add `tests/entry_kind_agent_indexing.rs` asserting an indexed `kind='agent'` row does not break `plugin list`/`plugin show`/`doctor` per-kind counts (use devs:rust-dev agent).
- [X] T013 Register a marker-only schema migration (bump schema version; no DDL/data change) in `src/index/migrations.rs` so the migration registry + doctor schema check agree the `kind` domain widened (research R-11) (use devs:rust-dev agent).
- [X] T014 [P] Extend `tests/schema_migration_*.rs` with a test that the marker migration applies and bumps the version (use devs:rust-dev agent).
- [X] T015 [GIT] Commit: `feat(phase-6): widen EntryKind with Agent variant + marker migration`.

### F3 — `HarnessModule` trait + `StubHarness` skeleton

- [ ] T016 Add the `HooksStrategy`, `GuardrailsTarget`/`GuardrailsPlacement`, and `AgentFormat` types in `src/harness/mod.rs` (use devs:rust-dev agent).
- [ ] T017 Extend the `HarnessModule` trait in `src/harness/mod.rs` with `hooks_strategy`, `hook_settings_path`, `guardrails_target`, `supports_native_agents`, `agent_dir`, `agent_format`, `translate_agent`, with safe defaults (`GuardrailsOnly` / `None` / `false`) (use devs:rust-dev agent).
- [ ] T018 Implement the new trait methods on `StubHarness` in `src/harness/stub.rs` for test injection (use devs:rust-dev agent).
- [ ] T019 [P] Add `tests/harness_trait_p6.rs` exercising the default impls + `StubHarness` overrides via `HARNESS_MODULES_OVERRIDE` (use devs:rust-dev agent).
- [ ] T020 [GIT] Commit: `feat(phase-6): extend HarnessModule trait + StubHarness for hooks/guardrails/agents`.

### Foundational closeout

- [ ] T021 Run codebase mapping for Phase 2 changes (`/sdd:map incremental`).
- [ ] T022 Review `retro/P2.md` and extract critical learnings to `CLAUDE.md` (conservative).
- [ ] T023 [GIT] Commit: `docs(phase-6): P2 closeout — mapping + retro`.
- [ ] T024 [GIT] Push branch to origin (ensure pre-push hooks pass).
- [ ] T025 [GIT] Create/update PR to main with Phase 2 summary.
- [ ] T026 [GIT] Verify all CI checks pass; report PR ready status.

**Checkpoint**: Error codes, the `Agent` kind, and the trait surface all exist. User stories can now wire real behaviour onto a compiling skeleton.

---

## Phase 3: User Story 1 — Native agents across four harnesses (P1)

**Goal**: Enabling a plugin with `agents/*.md` emits native agent files into each natively-supported harness's agent directory, plugin-namespaced, with same-vendor model mapping and unsupported fields dropped.

**Independent test**: Enable a plugin with an `agents/` dir in a workspace whose effective list includes claude-code/codex/cursor/opencode; assert each harness's agent dir holds `<plugin>__<name>.<ext>` in the right format/body location; assert model mapping + dropped fields; disable removes exactly `<plugin>__*`; re-sync is a no-op. (SC-001, SC-002, SC-003)

- [ ] T027 Create `specs/006-phase-6-hooks-agents/retro/P3.md` from template.
- [ ] T028 [GIT] Commit: `docs(phase-6): init P3 retro`.

### Agent indexing

- [ ] T029 [US1] Walk `agents/*.md` in `src/plugin/components.rs` alongside skills/ and commands/ (use devs:rust-dev agent).
- [ ] T030 [US1] Insert agent rows in `src/index/skills.rs` with `kind='agent'`, `searchable=0`, `user_invocable=0`, embedding skipped; name = frontmatter `name` else filename stem; description = else first non-empty body line (use devs:rust-dev agent).
- [ ] T031 [US1] Plumb agent kind through enable/disable/reindex in `src/plugin/lifecycle.rs` (use devs:rust-dev agent).
- [ ] T032 [US1] Add the workspace-scoped clash-set query (≥2 enabled agent rows sharing `<name>`) in `src/index/skills.rs`, computed once per sync (FR-072) (use devs:rust-dev agent).
- [ ] T033 [GIT] Commit: `feat(phase-6): index plugin agents (kind=agent, non-searchable)`.

### Translation core + naming + removal

- [ ] T034 [US1] Create `src/harness/agents.rs`: `CanonicalAgent` parse (frontmatter + body), the per-harness emit dispatch, and the `<plugin>__<name>.<ext>` filename builder (sole provenance) (use devs:rust-dev agent).
- [ ] T035 [US1] Add the same-vendor `ModelAliasTable` in `src/harness/agents.rs` per `contracts/agent-translation.md` (opus→opencode `anthropic/claude-opus-4.7`; opus→codex drop; inherit→drop everywhere); unmapped ⇒ drop (use devs:rust-dev agent).
- [ ] T036 [US1] Implement the read-only-intent inference rule (from tool allowlist/disallowed-tools) in `src/harness/agents.rs` (FR-036) (use devs:rust-dev agent).
- [ ] T037 [US1] Implement displayed/registered name logic (clean `<name>`; `<plugin>-<name>` only on clash) in `src/harness/agents.rs` (FR-041) (use devs:rust-dev agent).
- [ ] T038 [US1] Implement removal glob `<plugin>__*.<ext>` per agent dir in `src/harness/agents.rs` (FR-043) (use devs:rust-dev agent).
- [ ] T039 [GIT] Commit: `feat(phase-6): agent translation core + model-alias table + naming/removal`.

### Per-harness translation

- [ ] T040 [P] [US1] Implement `translate_agent` + `agent_dir`/`agent_format`/`supports_native_agents` for claude-code (MD+YAML, `.claude/agents/`) in `src/harness/claude_code.rs` (use devs:rust-dev agent).
- [ ] T041 [P] [US1] Implement Codex agent emission (TOML, `developer_instructions` triple-quoted body via `toml_edit`, `.codex/agents/`) in `src/harness/codex.rs` (use devs:rust-dev agent).
- [ ] T042 [P] [US1] Implement Cursor agent emission (MD+YAML, `.cursor/agents/`) in `src/harness/cursor.rs` (use devs:rust-dev agent).
- [ ] T043 [P] [US1] Implement OpenCode agent emission (MD+YAML, `.opencode/agent/`, `mode: subagent` default, filename-derived name, first-non-empty-line description fallback) in `src/harness/opencode.rs` (use devs:rust-dev agent).
- [ ] T044 [US1] Wire agent reconciliation into `src/harness/sync.rs` (emit present, remove orphaned), preserving the hooks→guardrails→agents order skeleton (use devs:rust-dev agent).
- [ ] T045 [GIT] Commit: `feat(phase-6): per-harness native agent emission + sync reconciliation`.

### Tests (US1)

- [ ] T046 [P] [US1] `tests/agent_translate_claude_code.rs` — emission + field passthrough + privilege passthrough default (use devs:rust-dev agent).
- [ ] T047 [P] [US1] `tests/agent_translate_codex.rs` — TOML `developer_instructions` body + `model: opus` dropped (use devs:rust-dev agent).
- [ ] T048 [P] [US1] `tests/agent_translate_cursor.rs` — MD+YAML + dropped unsupported field logged (use devs:rust-dev agent).
- [ ] T049 [P] [US1] `tests/agent_translate_opencode.rs` — `mode: subagent`, filename name, `model: opus`→`anthropic/claude-opus-4.7` (use devs:rust-dev agent).
- [ ] T050 [P] [US1] `tests/agent_naming_clash.rs` — two plugins, same agent name → both files namespaced, displayed names prefixed (use devs:rust-dev agent).
- [ ] T051 [P] [US1] `tests/agent_removal.rs` — disable removes `<plugin>__*` only; other plugins' agents remain (use devs:rust-dev agent).
- [ ] T052 [P] [US1] Extend `tests/entry_kind_agent_indexing.rs` — end-to-end enable → agent row present, searchable=0 (use devs:rust-dev agent).
- [ ] T053 [P] [US1] JSON wire-shape pin for the agent `dropped_fields` doctor sub-record (placeholder until US5 doctor lands; assert shape) in `tests/agent_translate_claude_code.rs` (use devs:rust-dev agent).
- [ ] T054 [GIT] Commit: `test(phase-6): US1 native agent translation + naming + removal`.

### US1 closeout

- [ ] T055 [US1] 4-reviewer parallel pass (contract / Rust-lens / test / security) against the merged US1 surface; write `review/us1-findings.md` + `review/us1-disposition.md`.
- [ ] T056 [US1] Apply US1 blockers + selected majors (use devs:rust-dev agent).
- [ ] T057 [GIT] Commit: `fix(phase-6): apply US1 reviewer findings`.
- [ ] T058 [US1] Run codebase mapping for Phase 3 changes (`/sdd:map incremental`).
- [ ] T059 [US1] Review `retro/P3.md`; extract critical learnings to `CLAUDE.md` (conservative).
- [ ] T060 [GIT] Commit: `docs(phase-6): US1 closeout — mapping + retro`.
- [ ] T061 [GIT] Push; create/update PR with US1 summary; verify CI; report PR ready status.

**Checkpoint**: Native agents work end-to-end across the four harnesses. MVP-deliverable.

---

## Phase 4: User Story 2 — Real hooks for Claude Code (P2)

**Goal**: A plugin shipping `hooks/hooks.json` merges into `.claude/settings.local.json` with plugin-root variable rewriting, idempotently, removing only what Tome can prove it owns.

**Independent test**: Enable a hook-shipping plugin in a claude-code project; assert `settings.local.json` has the rewritten hook (plugin-root absolute, other `${CLAUDE_*}` intact); re-sync adds no duplicate; user-identical hook not duplicated; disable removes only structural matches; committed settings file never written. (SC-004, SC-005)

- [ ] T062 Create `specs/006-phase-6-hooks-agents/retro/P4.md` from template.
- [ ] T063 [GIT] Commit: `docs(phase-6): init P4 retro`.
- [ ] T064 [US2] Create `src/harness/hooks.rs`: read `hooks/hooks.json` (`serde_json`), targeted two-variable rewrite (`${CLAUDE_PLUGIN_ROOT}`/`${CLAUDE_PLUGIN_DATA}` → absolute; all other `${CLAUDE_*}` verbatim) over string leaves via `regex` (use devs:rust-dev agent).
- [ ] T065 [US2] Implement structural-match merge (add): append under event only if no deep-equal entry exists; create `settings.local.json` with a single `hooks` object if absent; atomic write + mode-preservation + symlink-refusal (FR-002/004) (use devs:rust-dev agent).
- [ ] T066 [US2] Implement structural-match removal in `src/harness/hooks.rs`: remove deep-equal entries, skip non-matches; prune empty event arrays; keep empty `hooks` object (FR-005/006) (use devs:rust-dev agent).
- [ ] T067 [US2] Set `hooks_strategy() = RealJson` + `hook_settings_path()` for claude-code in `src/harness/claude_code.rs`; wire hooks reconciliation as the first step in `src/harness/sync.rs` (use devs:rust-dev agent).
- [ ] T068 [GIT] Commit: `feat(phase-6): real Claude Code hooks merge into settings.local.json`.
- [ ] T069 [P] [US2] `tests/hooks_rewrite.rs` — two-variable rewrite; other `${CLAUDE_*}` untouched (use devs:rust-dev agent).
- [ ] T070 [P] [US2] `tests/hooks_merge.rs` — add/idempotence/user-edit-preservation/dedup/create-if-absent/prune (use devs:rust-dev agent).
- [ ] T071 [P] [US2] Extend `tests/exit_codes_e2e.rs` — malformed `hooks.json` → exit 43; settings write failure → exit 44 (use devs:rust-dev agent).
- [ ] T072 [GIT] Commit: `test(phase-6): US2 hooks merge + rewrite + exit codes`.
- [ ] T073 [US2] 4-reviewer pass; write `review/us2-findings.md` + `review/us2-disposition.md`.
- [ ] T074 [US2] Apply US2 blockers + selected majors (use devs:rust-dev agent).
- [ ] T075 [GIT] Commit: `fix(phase-6): apply US2 reviewer findings`.
- [ ] T076 [US2] Run codebase mapping for Phase 4 changes (`/sdd:map incremental`).
- [ ] T077 [US2] Review `retro/P4.md`; extract learnings to `CLAUDE.md` (conservative).
- [ ] T078 [GIT] Commit: `docs(phase-6): US2 closeout — mapping + retro`.
- [ ] T079 [GIT] Push; PR with US2 summary; verify CI; report PR ready status.

**Checkpoint**: Real hooks enforce on Claude Code.

---

## Phase 5: User Story 3 — Guardrails fallback + Phase 4 rules-file correction (P3)

**Goal**: `GUARDRAILS.md` renders as per-plugin marker regions in each harness's rules file (Cursor → sibling), suppressed on `CLAUDE.md` when the plugin ships JSON hooks; and Claude Code's rules sink is corrected to `CLAUDE.md`.

**Independent test**: Enable a `GUARDRAILS.md`-only plugin across all five harnesses → region in `CLAUDE.md`, shared `AGENTS.md`, Cursor sibling; a both-shipping plugin → region in `AGENTS.md`, absent from `CLAUDE.md`; disable removes only its region; re-sync overwrites in place; the rules-include block is in `CLAUDE.md` not `AGENTS.md`. (SC-006, SC-007)

- [ ] T080 Create `specs/006-phase-6-hooks-agents/retro/P5.md` from template.
- [ ] T081 [GIT] Commit: `docs(phase-6): init P5 retro`.
- [ ] T082 [US3] Correct claude-code rules-file candidate list to `CLAUDE.md` > `.claude/CLAUDE.md` (drop `AGENTS.md`) in `src/harness/claude_code.rs` (FR-020/022) (use devs:rust-dev agent).
- [ ] T083 [P] [US3] `tests/rules_file_claude_correction.rs` — block lands in `CLAUDE.md`, AGENTS.md project keeps one block, both resolve `.tome/RULES.md` (use devs:rust-dev agent).
- [ ] T084 [GIT] Commit: `fix(phase-6): Claude Code rules sink is CLAUDE.md, not AGENTS.md`.
- [ ] T085 [US3] Create `src/harness/guardrails.rs`: verbatim `GUARDRAILS.md` read; marker region render (`<!-- START GUARDRAILS: <catalog>:<plugin> -->` … `END`); reuse the `rules_file.rs` region find/replace generalised to a parameterised marker pair (use devs:rust-dev agent).
- [ ] T086 [US3] Implement per-harness `guardrails_target` (claude-code→CLAUDE.md; codex/opencode→AGENTS.md; gemini→AGENTS.md else GEMINI.md; cursor→`.cursor/rules/TOME_GUARDRAILS.md` sibling) across the five harness modules (use devs:rust-dev agent).
- [ ] T087 [US3] Implement per-file reconciliation in `src/harness/guardrails.rs`: deterministic placement (rules block first, then regions in lexicographic `<catalog>:<plugin>` order), overwrite-in-place, orphan removal, Cursor sibling deletion when empty; atomic write + mode-preservation + symlink-refusal (FR-011/014/015) (use devs:rust-dev agent).
- [ ] T088 [US3] Implement the Claude Code suppression predicate (plugin ships `hooks.json` ⇒ no `CLAUDE.md` region) and the hooks→guardrails ordering + both suppression transitions in `src/harness/sync.rs` (FR-013/016) (use devs:rust-dev agent).
- [ ] T089 [GIT] Commit: `feat(phase-6): GUARDRAILS.md per-plugin regions + suppression`.
- [ ] T090 [P] [US3] `tests/guardrails_render.rs` — region in CLAUDE.md/AGENTS.md/Cursor sibling; two plugins → two regions; disable removes only one; re-sync in place (use devs:rust-dev agent).
- [ ] T091 [P] [US3] `tests/guardrails_suppression.rs` — both-shipping plugin suppressed on CLAUDE.md present on AGENTS.md; both start/stop-hooks transitions (use devs:rust-dev agent).
- [ ] T092 [P] [US3] Extend `tests/exit_codes_e2e.rs` — guardrails write failure → exit 46 (use devs:rust-dev agent).
- [ ] T093 [GIT] Commit: `test(phase-6): US3 guardrails render + suppression + correction`.
- [ ] T094 [US3] 4-reviewer pass; write `review/us3-findings.md` + `review/us3-disposition.md`.
- [ ] T095 [US3] Apply US3 blockers + selected majors (use devs:rust-dev agent).
- [ ] T096 [GIT] Commit: `fix(phase-6): apply US3 reviewer findings`.
- [ ] T097 [US3] Run codebase mapping for Phase 5 changes (`/sdd:map incremental`).
- [ ] T098 [US3] Review `retro/P5.md`; extract learnings to `CLAUDE.md` (conservative).
- [ ] T099 [GIT] Commit: `docs(phase-6): US3 closeout — mapping + retro`.
- [ ] T100 [GIT] Push; PR with US3 summary; verify CI; report PR ready status.

**Checkpoint**: Guardrails degrade honestly everywhere; Claude Code reads its rules.

---

## Phase 6: User Story 4 — Agent personas via MCP prompts (P4)

**Goal**: An opt-in global flag exposes each enabled agent as a `<name>-persona` MCP prompt plus a single global `drop-persona`, reusing the Phase 5 prompt + substitution machinery.

**Independent test**: Off (default) → no persona prompts; on → each agent appears as `<name>-persona` (clash-prefixed where required) + one `drop-persona`; `prompts/get` returns the wrapped frontmatter-stripped body with Phase 5 substitution + a free-form `args`. (SC-008)

- [ ] T101 Create `specs/006-phase-6-hooks-agents/retro/P6.md` from template.
- [ ] T102 [GIT] Commit: `docs(phase-6): init P6 retro`.
- [ ] T103 [US4] Add `expose_agents_as_personas: bool` (default false) to `GlobalSettings`/`WorkspaceSettings`/`ProjectMarkerConfig` in `src/settings/mod.rs`; add the first-declarer-wins scalar priority-walk resolver (project→workspace→global), distinct from the `harnesses` composition grammar (FR-053/060) (use devs:rust-dev agent).
- [ ] T104 [US4] Implement the persona registry in `src/mcp/prompts.rs`: build `<name>-persona` entries from enabled agent rows when the flag (resolved against the server startup scope, FR-067) is on; reuse `build_context_for_entry` + the Phase 5 substitution pipeline (use devs:rust-dev agent).
- [ ] T105 [US4] Implement persona name resolution (`<name>` from frontmatter name else filename stem; `<plugin>-<name>-persona` only on clash) and the role-assumption body wrapper + single catch-all `args` + ARGUMENTS fallback in `src/mcp/prompts.rs` (FR-061/062) (use devs:rust-dev agent).
- [ ] T106 [US4] Add the reserved global `drop-persona` prompt and fold persona derived names into the single Phase 5 collision namespace (union of command+skill+persona; agent-clash prefix before counter-suffix backstop) in `src/mcp/prompts.rs` + `src/mcp/prompt_collision.rs` (FR-063/066) (use devs:rust-dev agent).
- [ ] T107 [US4] Document the advisory-state caveat (persona is context, not enforced) in the prompt description + user docs (FR-065).
- [ ] T108 [GIT] Commit: `feat(phase-6): agent personas as MCP prompts + drop-persona`.
- [ ] T109 [P] [US4] `tests/personas.rs` — off→none; on→`<name>-persona` + one `drop-persona`; `prompts/get` body wrap + substitution + args (use devs:rust-dev agent).
- [ ] T110 [P] [US4] `tests/personas_collision.rs` — clash prefixing; `drop-persona` reserved vs a colliding command; collision-namespace union (use devs:rust-dev agent).
- [ ] T111 [P] [US4] JSON wire-shape pins for the persona `PromptDescriptor` entries (`tests/personas.rs`) (use devs:rust-dev agent).
- [ ] T112 [GIT] Commit: `test(phase-6): US4 personas + collision + wire pins`.
- [ ] T113 [US4] 4-reviewer pass; write `review/us4-findings.md` + `review/us4-disposition.md`.
- [ ] T114 [US4] Apply US4 blockers + selected majors (use devs:rust-dev agent).
- [ ] T115 [GIT] Commit: `fix(phase-6): apply US4 reviewer findings`.
- [ ] T116 [US4] Run codebase mapping for Phase 6 changes (`/sdd:map incremental`).
- [ ] T117 [US4] Review `retro/P6.md`; extract learnings to `CLAUDE.md` (conservative).
- [ ] T118 [GIT] Commit: `docs(phase-6): US4 closeout — mapping + retro`.
- [ ] T119 [GIT] Push; PR with US4 summary; verify CI; report PR ready status.

**Checkpoint**: Personas reach harnesses without native agents.

---

## Phase 7: User Story 5 — Privilege governance + doctor extensions (P5)

**Goal**: Plugin-agent privileged fields pass through to Claude Code by default but are auditable via doctor and strippable via an opt-in layered setting; doctor reports hooks/guardrails/agents/personas and `--fix` repairs the safe cases.

**Independent test**: Privileged agent emitted intact by default + listed in the doctor privilege report; with `strip_plugin_agent_privileges` on (workspace/global), the same agent is emitted without the three fields; doctor accurately reports all subsystems and `--fix` repairs only safe cases. (SC-009, SC-010)

- [ ] T120 Create `specs/006-phase-6-hooks-agents/retro/P7.md` from template.
- [ ] T121 [GIT] Commit: `docs(phase-6): init P7 retro`.
- [ ] T122 [US5] Add `strip_plugin_agent_privileges: bool` (default false) to the three settings structs in `src/settings/mod.rs`, reusing the scalar priority-walk resolver (FR-052/053) (use devs:rust-dev agent).
- [ ] T123 [US5] Implement privilege passthrough (default) + strip-when-set for `hooks`/`mcpServers`/`permissionMode` in claude-code agent emission in `src/harness/claude_code.rs` (FR-050/052) (use devs:rust-dev agent).
- [ ] T124 [GIT] Commit: `feat(phase-6): plugin-agent privilege passthrough + strip setting`.
- [ ] T125 [US5] Implement `HooksReport`, `GuardrailsReport`, `AgentsReport`, `PrivilegeEscalationReport`, `PersonaReport` (emit-only `Serialize`) in `src/doctor/` (FR-090) (use devs:rust-dev agent).
- [ ] T126 [US5] Wire the five reports into `assemble_report` in `src/commands/doctor.rs` (human + JSON), `None` only on `GlobalFallback` scope where applicable (use devs:rust-dev agent).
- [ ] T127 [US5] Implement `--fix` safe repairs (re-render stale guardrails, re-emit missing agents, remove orphaned `<plugin>__*`); never remove a non-matching hook, never delete user content (FR-091) in `src/doctor/fixes.rs` (use devs:rust-dev agent).
- [ ] T128 [US5] Extend `tome plugin show` in `src/commands/plugin/show.rs`: list agents + hooks.json/GUARDRAILS.md presence + resolved persona name when personas on (FR-083) (use devs:rust-dev agent).
- [ ] T129 [GIT] Commit: `feat(phase-6): doctor hooks/guardrails/agents/personas/privilege reports + --fix + plugin show`.
- [ ] T130 [P] [US5] `tests/agent_privilege.rs` — passthrough default; strip when set; doctor privilege report (use devs:rust-dev agent).
- [ ] T131 [P] [US5] `tests/doctor_p6.rs` — all five report surfaces + `--fix` safe cases + read-only-by-default invariant (use devs:rust-dev agent).
- [ ] T132 [P] [US5] `tests/doctor_json.rs` extensions — byte-stable JSON pins for the five new records (use devs:rust-dev agent).
- [ ] T133 [P] [US5] `tests/plugin_show_p6.rs` — agents listed + hooks/guardrails presence + persona name; JSON shape pin (use devs:rust-dev agent).
- [ ] T134 [P] [US5] Extend `tests/exit_codes_e2e.rs` — agent translation failure → exit 45 (use devs:rust-dev agent).
- [ ] T135 [GIT] Commit: `test(phase-6): US5 privilege + doctor + plugin show + wire pins`.
- [ ] T136 [US5] 4-reviewer pass; write `review/us5-findings.md` + `review/us5-disposition.md`.
- [ ] T137 [US5] Apply US5 blockers + selected majors (use devs:rust-dev agent).
- [ ] T138 [GIT] Commit: `fix(phase-6): apply US5 reviewer findings`.
- [ ] T139 [US5] Run codebase mapping for Phase 7 changes (`/sdd:map incremental`).
- [ ] T140 [US5] Review `retro/P7.md`; extract learnings to `CLAUDE.md` (conservative).
- [ ] T141 [GIT] Commit: `docs(phase-6): US5 closeout — mapping + retro`.
- [ ] T142 [GIT] Push; PR with US5 summary; verify CI; report PR ready status.

**Checkpoint**: All five user stories feature-complete. Only Polish remains.

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Phase-wide review (catches cross-US drift the per-US passes can't — Phase 5 P8 lesson), test gaps, security hardening, docs, and the v0.6.0 cut.

- [ ] T143 Create `specs/006-phase-6-hooks-agents/retro/P8.md` from template.
- [ ] T144 [GIT] Commit: `docs(phase-6): init P8 retro`.
- [ ] T145 Phase-wide 4-reviewer parallel pass (contract / Rust-lens / test / security) against the full Phase 6 surface; consolidate `review/findings.md` + `review/disposition.md` (reviewers read the us{1..5}-disposition docs first).
- [ ] T146 [GIT] Commit: `docs(review): Phase 6 phase-wide reviewer findings + disposition`.
- [ ] T147 Apply phase-wide blockers + selected majors (use devs:rust-dev agent).
- [ ] T148 [GIT] Commit: `fix(phase-6): apply phase-wide reviewer findings`.
- [ ] T149 [P] Add `tests/harness_sync_p6_idempotence.rs` — hooks+guardrails+agents idempotent across a re-sync (mtime capture pattern) (use devs:rust-dev agent).
- [ ] T150 [P] Add `tests/entry_e2e_p6.rs` — enable → reconcile all sinks → assert hooks/guardrails/agents/persona end-to-end (use devs:rust-dev agent).
- [ ] T151 Evaluate cap-std hardening across the four new file sinks (`openat`/`O_NOFOLLOW`) per the Phase 5 P8 security backlog; apply or document deferral (use devs:rust-dev agent).
- [ ] T152 [GIT] Commit: `test(phase-6): idempotence + e2e + security hardening`.
- [ ] T153 Update `README.md` (Phase 6 surface) + `CHANGELOG.md` (`[0.6.0]` entry: hooks, guardrails, agents, personas, Phase 4 correction; exit codes 43–46; no new deps; test delta).
- [ ] T154 Bump `Cargo.toml` `0.5.0` → `0.6.0`; constitution check (PASS, no amendment).
- [ ] T155 [GIT] Commit: `chore(release): v0.6.0 — version bump, CHANGELOG, README`.
- [ ] T156 Run final codebase mapping (`/sdd:map incremental`, 4 parallel mappers, all 8 docs).
- [ ] T157 Review `retro/P8.md`; extract critical learnings to `CLAUDE.md` (conservative); update the current-phase line + Recent Changes for the v0.6.0 cut.
- [ ] T158 [GIT] Commit: `docs(phase-6): Phase 6 closeout — mappers, retro P8, CLAUDE.md`.
- [ ] T159 [GIT] Push; create/update PR with the full Phase 6 summary; verify all CI checks pass; report PR ready status. (The v0.6.0 git tag + `cargo publish` are USER-RESERVED — do not run them.)

**Checkpoint**: Phase 6 shipped (v0.6.0). Hooks + agents complete.

---

## Dependencies & completion order

```
Setup (T001–T002)
   └─> Foundational (T003–T026)   ← F2 (EntryKind widening) is the hard gate
          ├─> US1 native agents (T027–T061)     [P1, MVP]
          ├─> US2 real hooks (T062–T079)        [P2] — independent of US1
          ├─> US3 guardrails + correction (T080–T100) [P3] — depends on F3 trait; guardrails reuse rules_file
          ├─> US4 personas (T101–T119)          [P4] — depends on F2 agent rows (US1 indexing) + Phase 5 prompt machinery
          └─> US5 privilege + doctor (T120–T142)[P5] — depends on US1 (agents) + US2/US3 (reports) + US4 (persona report)
                 └─> Polish (T143–T159)
```

- **F2 (EntryKind widening) is the load-bearing gate**: it must land before any slice writes a `kind='agent'` row (US1 indexing).
- **US1, US2, US3 are mutually independent** after Foundational and can be developed in any order / parallel branches. US3's guardrails reuse `rules_file.rs` region machinery (F3-era).
- **US4 personas depend on US1's agent indexing** (persona registry reads agent rows) and the Phase 5 prompt machinery.
- **US5 depends on US1–US4** because its doctor reports cover all four subsystems; the privilege strip depends on US1's claude-code agent emission.

## Parallel execution examples

- **Foundational**: T007, T011, T012, T014, T019 are `[P]` (distinct test files / unit tests).
- **US1**: T040–T043 (the four per-harness `translate_agent` impls, distinct files) run in parallel; T046–T053 (distinct test files) run in parallel.
- **US3**: T090–T092 parallel. **US4**: T109–T111 parallel. **US5**: T130–T134 parallel. **Polish**: T149–T150 parallel.

## Implementation strategy

- **MVP = US1** (native agents): the largest portable win, independently demonstrable across four harnesses. Ship it first.
- **Incremental delivery**: each user story is a complete, independently testable increment behind its own PR (the phase-end PR block). US2 and US3 can land in either order; US4/US5 build on the agent surface.
- **Discipline carried from Phase 5**: per-US 4-reviewer pass dispatched as one message; findings + disposition committed before fixes; `/sdd:map incremental` at every closeout; JSON wire-shape pins for every new emit-only type; phase-wide reviewer pass at Polish even when per-US passes were clean.

## Notes

- **Tests**: every new emit-only type gets a byte-stable JSON wire-shape pin (NFR-011). Heavy paths use the library API + `StubEmbedder` + `HARNESS_MODULES_OVERRIDE`/`StubHarness`; light/exit-code paths use the CLI binary (`tests/exit_codes_e2e.rs`). Non-UTF8 path tests gate on `#[cfg(target_os = "linux")]`.
- **No new top-level dependencies** and **no new top-level module** — `hooks.rs`/`guardrails.rs`/`agents.rs` live inside `src/harness/`.
- **User-reserved**: the v0.6.0 git tag, `cargo publish`, and release-notes posting (constitution §Release tooling).

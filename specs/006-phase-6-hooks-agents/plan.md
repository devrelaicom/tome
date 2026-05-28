# Implementation Plan: Phase 6 — Hooks and Agents: Real Hooks, Soft Guardrails, Native Agent Translation, and Personas

**Branch**: `006-phase-6-hooks-agents` | **Date**: 2026-05-28 | **Spec**: [spec.md](./spec.md)
**Input**: Feature specification from `/specs/006-phase-6-hooks-agents/spec.md`
**Source PRD**: [PRDs/phase-6.md](../../PRDs/phase-6.md) (authoritative for HOW; spec is authoritative for WHAT)

## Summary

Phase 6 closes out the two component types Tome deferred since the start — **hooks** and **agents** — and corrects a Phase 4 mistake about where Claude Code reads its rules. The approach is deliberately asymmetric, mirroring the PRD's mental model: enforcement (hooks) doesn't port, so it is delivered only where it is real (Claude Code JSON hooks) and degrades honestly to a prose `GUARDRAILS.md` fallback everywhere else; portability (agents) mostly ports, so agents get full native translation across four harnesses plus an optional MCP-prompt "persona" fallback.

The technical approach reuses Phase 4's harness-module machinery and Phase 5's MCP-prompt + substitution machinery, and adds no new top-level dependency. The `HarnessModule` trait gains hook, guardrails, and agent-emission capabilities; a new `src/harness/hooks.rs` does structural-match JSON merge into `.claude/settings.local.json` (via the existing `serde_json`/`preserve_order`); a new `src/harness/guardrails.rs` renders per-plugin marker regions into each harness's rules file (reusing the `rules_file.rs` region machinery and the Phase 4 atomic-write discipline); a new `src/harness/agents.rs` plus per-harness `translate_agent` implementations emit native agent files (`MarkdownYaml` for three harnesses, `Toml` via `toml_edit` for Codex). The `EntryKind` enum (`src/plugin/identity.rs`) gains an `Agent` variant and every exhaustive match over it is widened; the plugin scan walks `agents/*.md` and indexes agent rows with `searchable = 0`. The MCP server gains a specialised persona path in `src/mcp/prompts.rs` (off by default, governed by a config flag resolved against the server's startup scope) reusing the Phase 5 prompt-naming, collision, and substitution machinery. Claude Code's `rules_file_target` candidate list is corrected to `CLAUDE.md` > `.claude/CLAUDE.md`, dropping `AGENTS.md`.

Four new `TomeError` variants claim the exit-code run **43–46** (the PRD-proposed 30–33 collide with the model-on-disk cluster; 34–37 are also taken). No new schema columns or tables are required — the only storage-layer change is widening the free-text `kind` column's domain to admit `'agent'`; whether a marker migration is registered is a research decision (R-11), but no data migration is needed.

## Technical Context

**Language/Version**: Rust stable (MSRV 1.93, pinned in `Cargo.toml`). No MSRV change for Phase 6.

**Primary Dependencies (existing, consumed in Phase 6)**:
- `serde_json` (`preserve_order`) — `hooks.json` parse + structural-match merge into `.claude/settings.local.json`; existing dep, first used in Phase 4 `mcp_config.rs`.
- `toml_edit` — Codex agent emission (TOML with a triple-quoted `developer_instructions` body); comment/order-preserving edits. Existing dep.
- `serde` + `serde_yaml` — agent + `hooks.json` + `GUARDRAILS.md`-adjacent frontmatter parsing (lenient third-party boundary).
- `regex` — guardrails marker matching + the targeted two-variable hook-path rewrite. Direct dep since Phase 5.
- `rusqlite` (`bundled`) — agent rows on the existing `skills` table; widened `kind` domain.
- `rmcp` (`transport-io`, `schemars`) + `tokio` (single-threaded, `src/mcp/` only) + `schemars` — persona prompts on the existing `prompts` capability.
- `tempfile` — atomic writes for all new file sinks (settings, rules files, sibling guardrails file, agent files).
- `tracing` + `tracing-subscriber` — dropped-field / fallback / suppression diagnostics.

**Primary Dependencies (new direct)**: None. Phase 6 introduces no new top-level crate (constitution §Dependencies + §Complexity budget satisfied without an amendment). If the contracts phase finds a target harness's file format unrepresentable with the existing deps, any addition is constitution-gated (Development Standards, last bullet) — not anticipated.

**Storage**: Existing central SQLite database (`<home>/.tome/index.db`, WAL + advisory lockfile). No new columns, no new tables. The `kind` column's permitted domain widens to admit `'agent'`; agent rows reuse existing columns and the existing `(catalog, plugin, kind, name)` uniqueness constraint, with `searchable = 0` and embedding skipped. Hooks/guardrails/agent files are reconciled on the filesystem with no sidecar (Phase 4 model). New on-disk write sinks (all under the bound project, not the central tree): `.claude/settings.local.json` (hooks), the per-harness rules file (guardrails regions), `.cursor/rules/TOME_GUARDRAILS.md` (Cursor sibling), and the four agent directories.

**Testing**: `cargo test` (existing). Unit tests in source modules; integration tests under `tests/`. New integration test files expected: `hooks_merge.rs`, `hooks_rewrite.rs`, `guardrails_render.rs`, `guardrails_suppression.rs`, `rules_file_claude_correction.rs`, `agent_translate_*.rs` (per harness), `agent_naming_clash.rs`, `agent_removal.rs`, `agent_privilege.rs`, `personas.rs`, `personas_collision.rs`, `entry_kind_agent_indexing.rs`, `doctor_p6.rs`, `harness_sync_p6_idempotence.rs`, `entry_e2e_p6.rs`. JSON wire-shape pins for every new emit-only type (the doctor hooks/guardrails/agents/persona/privilege records, the persona `PromptDescriptor` entries). Heavy paths use the library API + `StubEmbedder` + `HARNESS_MODULES_OVERRIDE`/`StubHarness`; light/exit-code paths use the CLI binary via `tests/exit_codes_e2e.rs`.

**Target Platform**: macOS (`macos-latest`) and Linux (`ubuntu-latest`) — CI verified. Non-UTF8 path refusal tests gate on `#[cfg(target_os = "linux")]` (APFS rejects non-UTF8 names at `mkdir(2)`, per the Phase 4 P3 retro).

**Project Type**: Single Rust project (binary + library; no workspace split).

**Performance Goals**:
- Hook merge: linear in the plugin's hook-entry count × the existing settings file's entry count for the structural-match scan; settings files are small.
- Guardrails reconciliation: linear in (enabled-plugins-with-guardrails × target files); region edits are in-place between markers.
- Agent translation/emission: linear in the plugin's agent count per harness; each translation is a bounded field-map pass.
- Persona `prompts/list`: linear in the active workspace's enabled-agent count when the flag is on; built once at server startup like the Phase 5 prompt registry.
- No new asymptotic cost classes; no per-request DB scans beyond a single query.

**Constraints**:
- Sync only outside `src/mcp/` (constitution §Async). Hooks/guardrails/agents reconciliation is sync; only the persona prompt path rides the existing `src/mcp/` async island via `spawn_blocking`.
- Atomic writes with mode-preservation + symlink-refusal at every read/write entry point (Phase 4 S-M3 + `refuse_symlink` discipline) for all four new file sinks.
- 50 MB binary cap (constitution §Binary size). No new deps; size projection +0 MiB. Budget margin stays ≈ 23 MiB on macOS arm64 from the Phase 4 / v0.5.0 baseline.
- Closed-error-enum discipline (`TomeError`). Phase 6 adds four variants claiming exit codes 43–46; numbers pinned in `contracts/exit-codes-p6.md`.
- Strictness boundary (constitution §Strict Schemas). Agent frontmatter / `hooks.json` / `GUARDRAILS.md` are third-party (lenient, fail loudly on malformed recognised structures). The two new settings fields are Tome-owned (`deny_unknown_fields`, strict).
- Filesystem-inferred state, no sidecar (Phase 4 model): hooks by structural re-derivation, guardrails by marker pairs, agents by the `<plugin>__*` filename glob.

**Scale/Scope**: 5 user stories (P1–P5), 54 functional requirements, 11 non-functional requirements, 11 success criteria, 4 new exit codes (43–46), 0 new schema columns (one widened `kind` domain + one widened Rust enum), 2 new config settings, 1 corrected Phase 4 behaviour (Claude Code rules sink). Three new top-level files under `src/harness/` (`hooks.rs`, `guardrails.rs`, `agents.rs`); extensions to the `HarnessModule` trait + all five harness modules; extensions to `src/mcp/prompts.rs`, `src/plugin/{identity,components,lifecycle}.rs`, `src/index/skills.rs`, `src/settings/mod.rs`, `src/commands/{doctor,plugin/show}.rs`. No new top-level module.

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

Each principle from `CONSTITUTION.md` (v1.3.0):

- **I. Unix Philosophy**: PASS. No new top-level commands (FR-080); the new behaviour rides `tome harness sync`, `tome plugin enable/disable`, `tome plugin show`, and `tome doctor`, each still describable in one sentence. `--json` preserved on every new output path.
- **II. Predictable Exit Codes (NON-NEGOTIABLE)**: PASS with attention. Four new failure classes (FR-092): malformed `hooks.json`; settings-file read/merge/write failure; agent frontmatter/translation failure; guardrails render/write failure. Each gets a dedicated variant + a code in the free run **43–46**. **Action**: pin the numerics in `contracts/exit-codes-p6.md`; reuse none of 1–9, 13–37, 40–42, 50–54, 60–61, 70, 73–75.
- **III. Scriptable by Default**: PASS. No new interactive prompts. Hooks/guardrails/agents reconcile non-interactively during sync; personas are enumerated by the server, triggered on the harness side.
- **IV. Strict Schemas, Helpful Errors**: PASS. Agent frontmatter / `hooks.json` / `GUARDRAILS.md` parse on the lenient third-party boundary, failing loudly on malformed recognised structures (FR-092, NFR-010). The two new settings fields and every new emit-only JSON record are Tome-owned and strict.
- **V. Fail Fast, Fail Clear**: PASS. Each new failure surfaces a dedicated code naming the file/agent; partial writes are never left (NFR-004, FR-084). Dropped agent fields and unset-env-style fallbacks are surfaced via `tracing`, not swallowed.
- **VI. KISS / YAGNI**: PASS. No per-event JSON/prose merge (whole-file suppression only, FR-013); no agent-semantics emulation; no cross-vendor model heuristics (FR-034); no semantic search over agents. The privilege-strip and persona surfaces are off by default. The Rule of Three governs any helper promotion (Phase 4 P3 lesson).
- **VII. Modular by Boundary**: PASS. Hooks/guardrails/agents land as new files inside the existing `src/harness/` capability module behind the `HarnessModule` trait surface; personas extend `src/mcp/prompts.rs`; the enum change is local to `src/plugin/identity.rs`. No circular dependencies, no new top-level module.
- **VIII. Test What Matters**: PASS. Integration tests for every shipped behaviour (merge, suppression transitions, translation per harness, removal globs, privilege strip, personas, idempotence) against real fixtures + `StubHarness`/`StubEmbedder`. JSON wire-shape pins for every new emit-only type (Phase 4 P8 + Phase 5 P8 lesson). CLI-binary coverage for the four new exit codes.
- **IX. Conventional Commits**: PASS. `cog` hook enforces.
- **X. CI Gates Every Merge**: PASS. Existing `fmt + clippy + build + test` matrix continues; the slim pre-push (Phase 5 #126) stays.
- **XI. Documentation Is Part of the Change**: PASS. Each slice updates its contract under `specs/006-phase-6-hooks-agents/contracts/`; final Polish updates README + CHANGELOG. The persona advisory-state caveat (FR-065) is a documentation requirement.
- **XII. Inherit, Don't Reimplement**: PASS. `serde_json` for the JSON merge, `toml_edit` for Codex TOML, `regex` for markers, rmcp's prompt router for personas. No new merge/templating library — the persona path reuses the Phase 5 substitution module (NFR-007). The harness owns all hook/agent runtime behaviour; Tome never executes (NFR-002).
- **XIII. Never Log Secrets**: PASS. The hook-path rewrite touches only `${CLAUDE_PLUGIN_ROOT}`/`${CLAUDE_PLUGIN_DATA}` → absolute paths (no credentials); diagnostics log file paths + agent names + dropped field names, never `hooks.json` command bodies or env values. Existing scrubbing discipline carries to any surfaced IO error.

Operational Constraints:
- **Async**: Hooks/guardrails/agents are sync. Only the persona `prompts/get` path is async, reusing the Phase 3 `spawn_blocking` pattern inside `src/mcp/`.
- **Binary size**: No new deps; expected delta 0 MiB.
- **Paths**: All new write sinks are inside the bound project (`.claude/`, `.cursor/`, `.codex/`, `.opencode/`, the rules file). The plugin-data path used in the hook rewrite resolves under `<home>/.tome/` per constitution §Paths (the Phase 5 `${TOME_PLUGIN_DATA}` value).
- **Licensing**: No new deps; no licence review.

**Verdict**: PASS. No deviations require Complexity Tracking. (Contrast Phase 4, which needed two: a bundled summariser + the §Paths amendment. Phase 6 needs neither.)

## Project Structure

### Documentation (this feature)

```text
specs/006-phase-6-hooks-agents/
├── spec.md                               # /sdd:specify output (frozen)
├── plan.md                               # This file (/sdd:plan output)
├── research.md                           # Phase 0 output (/sdd:plan)
├── data-model.md                         # Phase 1 output (/sdd:plan)
├── quickstart.md                         # Phase 1 output (/sdd:plan)
├── contracts/                            # Phase 1 output (/sdd:plan)
│   ├── exit-codes-p6.md
│   ├── harness-modules-p6.md             # HarnessModule trait extensions + Claude Code rules-file correction
│   ├── hooks-integration.md              # real-hooks merge into .claude/settings.local.json
│   ├── guardrails.md                     # GUARDRAILS.md per-plugin region rendering + suppression
│   ├── agent-translation.md              # native agent emission + field/value maps + naming/removal
│   ├── agent-personas.md                 # persona MCP prompts + drop-persona
│   ├── entry-schema-p6.md                # kind='agent' + EntryKind widening + indexing pipeline
│   ├── settings-p6.md                    # expose_agents_as_personas + strip_plugin_agent_privileges
│   └── doctor-extensions-p6.md           # hooks/guardrails/agents/personas/privilege reports + --fix
├── checklists/
│   └── requirements.md                   # /sdd:specify output (frozen)
├── retro/                                # Created per user story by closeout PRs
├── review/                               # Created by US-closeout reviewer passes
└── tasks.md                              # /sdd:tasks output — NOT created by /sdd:plan
```

### Source Code (repository root)

```text
src/
├── error.rs                  # (existing) +4 TomeError variants + exit codes 43–46
├── plugin/
│   ├── identity.rs           # (existing) EntryKind gains Agent variant; FromStr/Display/all matches widened
│   ├── components.rs         # (existing) walk agents/*.md alongside skills/ and commands/
│   └── lifecycle.rs          # (existing) plumb agent kind through enable/disable/reindex; trigger sync reconciliation
├── index/
│   └── skills.rs             # (existing) agent rows inserted with searchable=0, embedding skipped; per-kind count matches widened
├── settings/
│   └── mod.rs                # (existing) +2 bool fields on Global/Workspace/ProjectMarker; priority-walk resolver for scalars
├── harness/
│   ├── mod.rs                # (existing) HarnessModule trait extended (hooks_strategy, hook_settings_path, guardrails_target, supports_native_agents, agent_dir, agent_format, translate_agent)
│   ├── sync.rs               # (existing) reconcile hooks → guardrails → agents per harness; cross-sink forward progress
│   ├── rules_file.rs         # (existing) region machinery reused by guardrails; claude correction touches candidate list here/in claude_code.rs
│   ├── hooks.rs              # NEW — hooks.json read + 2-var rewrite + structural-match merge/removal into settings.local.json
│   ├── guardrails.rs         # NEW — per-plugin marker region render + per-file reconciliation + Cursor sibling + suppression
│   ├── agents.rs             # NEW — agent source read + per-harness emit dispatch + naming/removal glob + model-alias table
│   ├── claude_code.rs        # (existing) rules_file_target → CLAUDE.md (correction); RealJson hooks; native agents; privilege passthrough/strip
│   ├── codex.rs              # (existing) GuardrailsOnly; native agents (TOML, developer_instructions)
│   ├── cursor.rs             # (existing) GuardrailsOnly (sibling file); native agents (MD+YAML)
│   ├── gemini.rs             # (existing) GuardrailsOnly; NO native agents (personas only)
│   ├── opencode.rs           # (existing) GuardrailsOnly; native agents (MD+YAML, filename-derived name, mode: subagent)
│   └── stub.rs               # (existing) StubHarness extended for the new trait methods (tests)
├── mcp/
│   ├── prompts.rs            # (existing) specialised persona path: <name>-persona + drop-persona; reuses naming/collision/substitution
│   └── prompt_collision.rs   # (existing) collision namespace now includes persona derived names (FR-066)
└── commands/
    ├── doctor.rs             # (existing) hooks/guardrails/agents/personas/privilege reports + --fix safe cases
    └── plugin/show.rs        # (existing) list agents + hooks.json/GUARDRAILS.md presence + resolved persona name

tests/
├── hooks_merge.rs                    # NEW (US2)
├── hooks_rewrite.rs                  # NEW (US2)
├── guardrails_render.rs              # NEW (US3)
├── guardrails_suppression.rs         # NEW (US3)
├── rules_file_claude_correction.rs   # NEW (US3) — CLAUDE.md not AGENTS.md
├── agent_translate_claude_code.rs    # NEW (US1)
├── agent_translate_codex.rs          # NEW (US1) — TOML developer_instructions
├── agent_translate_cursor.rs         # NEW (US1)
├── agent_translate_opencode.rs       # NEW (US1) — mode: subagent, filename-derived name
├── agent_naming_clash.rs             # NEW (US1)
├── agent_removal.rs                  # NEW (US1)
├── agent_privilege.rs                # NEW (US5)
├── personas.rs                       # NEW (US4)
├── personas_collision.rs             # NEW (US4)
├── entry_kind_agent_indexing.rs      # NEW (US1) — kind='agent', searchable=0, EntryKind widening
├── doctor_p6.rs                      # NEW (US5)
├── harness_sync_p6_idempotence.rs    # NEW — hooks+guardrails+agents idempotent
├── entry_e2e_p6.rs                   # NEW — enable → reconcile → all sinks
└── (existing tests) extended where Phase 6 changes affect them (doctor_*, plugin_show_*, harness_*, exit_codes*)
```

**Structure Decision**: Single Rust project (binary + library). Phase 6 adds **no new top-level module** — `hooks.rs`, `guardrails.rs`, and `agents.rs` cluster inside the existing `src/harness/` capability module behind the `HarnessModule` trait, exactly as `rules_file.rs`/`mcp_config.rs`/`sync.rs` did in Phase 4. Persona work extends the existing `src/mcp/prompts.rs`. The `EntryKind` change is local to `src/plugin/identity.rs`. This is the smallest structure that satisfies the spec (constitution §VI, §VII).

## Complexity Tracking

> No constitution violations. No entries. (Phase 6 adds zero top-level dependencies and zero new top-level modules — the leanest phase since Phase 1.)

| Violation | Why Needed | Simpler Alternative Rejected Because |
|-----------|------------|-------------------------------------|
| _(none)_  | _(n/a)_    | _(n/a)_                              |

## Pre-emptive slice plan

Per the Phase 4 P3 + Phase 5 lessons (encode the slice shape in the plan so `/sdd:tasks` and the per-slice agents inherit ≤ 8 KB briefs), the implementation is sliced as:

- **Foundational** (before user stories):
  - **F1** — pre-allocate the 4 `TomeError` variants + exit codes 43–46 (`contracts/exit-codes-p6.md`), so downstream slices reference real codes (Phase 4 F3 / Phase 5 F1 precedent).
  - **F2** — widen `EntryKind` with the `Agent` variant + update every exhaustive match (the FR-070a blocker fix), shipped with the per-kind-count tests, BEFORE any agent row is written. This is the load-bearing change; landing it first means no later slice can introduce a crashing `kind='agent'` row.
  - **F3** — extend the `HarnessModule` trait + `StubHarness` with the new methods (default `GuardrailsOnly` / `supports_native_agents=false`), so slices wire real behaviour onto a compiling trait surface (Phase 4 F7 precedent).
- **US1 (P1) — native agents**: `agents.rs` + per-harness `translate_agent` (claude_code, codex, cursor, opencode) + the model-alias table + naming/removal glob + agent indexing. Slices: US1.a translation core + claude_code; US1.b codex (TOML) + cursor; US1.c opencode (filename-name + subagent default); US1.d indexing + naming-clash + removal; US1.e reviewer pass + closeout.
- **US2 (P2) — real hooks**: `hooks.rs` (rewrite + structural merge/removal) + claude_code `RealJson`. Slices: US2.a rewrite + merge; US2.b removal + empty-array prune; US2.c reviewer pass + closeout.
- **US3 (P3) — guardrails + Phase 4 correction**: `guardrails.rs` + per-harness `guardrails_target` + Cursor sibling + suppression + the `claude_code` rules-file candidate-list correction. Slices: US3.a correction (CLAUDE.md) + render; US3.b suppression + reconciliation transitions (FR-016); US3.c reviewer pass + closeout.
- **US4 (P4) — personas**: persona path in `prompts.rs` + drop-persona + `expose_agents_as_personas` setting (resolved against server startup scope) + collision-namespace union. Slices: US4.a persona registry + naming; US4.b drop-persona + reserved name + collision; US4.c reviewer pass + closeout.
- **US5 (P5) — privilege governance + doctor**: `strip_plugin_agent_privileges` setting + privilege passthrough/strip in claude_code agent emission + doctor extensions (hooks/guardrails/agents/personas/privilege reports + `--fix` safe cases) + `plugin show` extensions. Slices: US5.a privilege strip + report; US5.b doctor surfaces + `--fix`; US5.c reviewer pass + closeout.
- **Polish**: phase-wide 4-reviewer pass (the Phase 5 P8 lesson — run it even when per-US passes were thorough) → blockers → selected majors → test gaps → security hardening (incl. the deferred cap-std evaluation, Phase 5 P8 backlog) → docs/README/CHANGELOG + v0.6.0 cut → `/sdd:map incremental` + retro + CLAUDE.md.

Each user-story closeout runs the 4-reviewer parallel pass (contract / Rust-lens / test / security) dispatched as ONE message, with findings + disposition committed before fixes (Phase 5 P7/P8 lesson), and `/sdd:map incremental` at closeout.

---
description: "Phase 5 implementation tasks — commands as prompts, unified entries, variable substitution"
---

# Tasks: Phase 5 — Commands as Prompts, Unified Entries, and Variable Substitution

**Input**: Design documents from `/specs/005-phase-5-commands-prompts/`
**Prerequisites**: plan.md (✓), spec.md (✓), research.md (✓ 20 R-decisions), data-model.md (✓), contracts/ (✓ 9 files), quickstart.md (✓)
**Created**: 2026-05-26
**Tests**: This project uses TDD. Library-API tests + integration tests are part of each user story's slices. JSON wire-shape pin tests land in the SAME PR as the type they pin (Phase 4 P8 retro lesson).

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story. The slice structure matches research §R-17 + plan §Pre-emptive slice plans. All Rust implementation tasks reference the `devs:rust-dev` agent.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: Which user story this task belongs to (US1, US2, US3, US4, US5)
- **[GIT]**: Git workflow task at a phase or slice boundary
- File paths are repository-root-relative

## Branch & feature

- **Feature number**: `005`
- **Feature slug**: `phase-5-commands-prompts`
- **Branch**: `005-phase-5-commands-prompts` (already created and carries the spec + plan + research + data-model + 9 contracts + quickstart + this tasks.md — see `git log`)

## ID allocation

- `T001`–`T099` Setup + Foundational (F1–F3)
- `T100`–`T199` US1 — Commands as MCP prompts
- `T200`–`T249` US2 — Substitution layer (paths/env)
- `T250`–`T299` US3 — Argument substitution
- `T300`–`T349` US4 — Middle-tier discovery + when_to_use indexing
- `T350`–`T399` US5 — Per-entry invocability flags + doctor extensions
- `T400`+ Polish (final phase)

Block numbering with buffer space — refine within blocks as needed; the buffer makes "find me the US3 tasks" trivial.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Branch hygiene + baseline verification before Foundational work begins.

### Phase Start

- [ ] T001 [GIT] Verify on `005-phase-5-commands-prompts` branch and working tree is clean
- [ ] T002 [GIT] Pull and rebase on origin/main; resolve any conflicts
- [ ] T003 [GIT] Confirm `cargo test` is green at HEAD (Phase 4 v0.4.0-complete baseline; expect 954 tests across 127 suites, 16 ignored)

### Implementation

- [ ] T004 Bump `Cargo.toml` version from `0.4.0` to `0.5.0-dev` to mark Phase 5 work-in-progress (use devs:rust-dev agent)
- [ ] T005 [GIT] Commit: `chore: bump version to 0.5.0-dev for phase 5 work`

### Phase Completion

- [ ] T006 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T007 [GIT] Open PR to main with Phase 1 summary (planning + version bump)
- [ ] T008 [GIT] Verify all CI checks pass (binary-size gate + cargo-deny + clippy + test matrix)
- [ ] T009 [GIT] Report PR ready status

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Land the 5 new `TomeError` variants + exit codes, promote `regex` to direct dep, ship the `src/substitution/` module skeleton with `StubSubstituter` + override seams. Every user story depends on this phase.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

### Phase Start

- [ ] T010 [GIT] Verify working tree is clean before starting Phase 2
- [ ] T011 Create `specs/005-phase-5-commands-prompts/retro/P2.md` from the standard retro template

### F1: Pre-allocate Phase 5 `TomeError` variants + exit codes

- [ ] T012 Add 5 new variants to `TomeError` in `src/error.rs`: `EntryNotFound`, `SubstitutionFailed`, `InvalidArgumentFrontmatter`, `WorkspaceDataDirWriteFailed`, `PromptArgumentMismatch` per `contracts/exit-codes-p5.md` (use devs:rust-dev agent)
- [ ] T013 Extend `From<TomeError> for ExitCode` mapping in `src/error.rs` to assign codes 25, 26, 27, 28, 29 per `contracts/exit-codes-p5.md` (codes 21/22/23 originally proposed but reassigned to 27/28/29 to avoid collision with Phase 2's `PluginAlreadyInState`/`PluginManifestParseError`/`SkillFrontmatterParseError`) (use devs:rust-dev agent)
- [ ] T014 Extend `tests/exit_codes.rs` with 5 assertions pinning each new code (use devs:rust-dev agent)
- [ ] T015 Run `cargo test --test exit_codes` to confirm all assertions pass (use devs:rust-dev agent)
- [ ] T016 [GIT] Commit: `feat(error): pre-allocate 5 phase 5 error variants + exit codes`

### F2: Promote `regex` from transitive to direct dependency

- [ ] T017 Run `cargo tree -i regex` to confirm `regex` is currently a transitive dep via `catalog::git::scrub_credentials` (use devs:rust-dev agent)
- [ ] T018 Add `regex = "1"` to `[dependencies]` in `Cargo.toml` with version matching the existing transitive (per research §R-2) (use devs:rust-dev agent)
- [ ] T019 Confirm `cargo deny check` still passes (regex's licence is already in the allowlist via transitive use) (use devs:rust-dev agent)
- [ ] T020 Run `cargo build --release` and record stripped binary size in `RELEASE-BINARY-SIZE.md` (expect minimal delta from 26.32 MiB baseline) (use devs:rust-dev agent)
- [ ] T021 [GIT] Commit: `chore(deps): promote regex to direct dependency for phase 5 substitution`

### F3: `src/substitution/` module skeleton

- [ ] T022 Create `src/substitution/mod.rs` with `pub use` re-exports, `SubstitutionError` enum (5 variants per data-model.md §3.1), and a stub `pub fn render()` returning `Ok(body.to_string())` (use devs:rust-dev agent)
- [ ] T023 Create `src/substitution/context.rs` with `SubstitutionContext` struct (12 fields per data-model.md §3.2), `SubstitutionContextBuilder`, and `ArgumentValues` enum (`Single(String)` / `Object { named, declared_order }`) (use devs:rust-dev agent)
- [ ] T024 Create `src/substitution/builtins.rs` with stub `pub(super) fn apply_builtins(body, ctx) -> Result<String, SubstitutionError>` returning body unchanged (use devs:rust-dev agent)
- [ ] T025 Create `src/substitution/env.rs` with stub `pub(super) fn apply_env(body) -> Cow<'_, str>` returning `Cow::Borrowed(body)` (use devs:rust-dev agent)
- [ ] T026 Create `src/substitution/arguments.rs` with stub `pub(super) fn apply_arguments(body, args, declared) -> (String, bool)` returning `(body.to_string(), false)` (use devs:rust-dev agent)
- [ ] T027 Create `src/substitution/regex.rs` with three `OnceLock<Regex>` slots (built-ins, env, arguments) — uncompiled in F3, populated in US2/US3 (use devs:rust-dev agent)
- [ ] T028 Create `src/substitution/data_dir.rs` with stub `pub(super) fn ensure_plugin_data(...)` and `ensure_workspace_data(...)` returning `PathBuf` without actually creating dirs (use devs:rust-dev agent)
- [ ] T029 Add `SUBSTITUTION_CLOCK_OVERRIDE`, `PLUGIN_DATA_DIR_OVERRIDE`, `WORKSPACE_DATA_DIR_OVERRIDE` `#[doc(hidden)] pub static OnceLock<Mutex<Option<...>>>` slots in `src/substitution/mod.rs` per research §R-16 (use devs:rust-dev agent)
- [ ] T030 Add `pub mod substitution;` to `src/lib.rs` (use devs:rust-dev agent)
- [ ] T031 Create `tests/common/mod.rs` extensions: `ClockOverrideGuard`, `PluginDataDirGuard`, `WorkspaceDataDirGuard` RAII helpers, each with `install()` constructor + `Drop` impl clearing the slot per the established `HarnessModulesGuard` pattern (use devs:rust-dev agent)
- [ ] T032 Create `tests/substitution_skeleton.rs` with smoke tests confirming `render()` returns body unchanged, the three guards install + clear correctly, and the override slots are reachable from integration tests (use devs:rust-dev agent)
- [ ] T033 Run `cargo test --test substitution_skeleton` to verify all 4-6 smoke tests pass (use devs:rust-dev agent)
- [ ] T034 [GIT] Commit: `feat(substitution): src/substitution/ module skeleton with override seams`

### F-close: Codebase mapping + retro

- [ ] T035 Run `/sdd:map incremental` to refresh `.sdd/codebase/` against Phase 2 changes (4 parallel mappers)
- [ ] T036 Review `retro/P2.md` and extract critical learnings to CLAUDE.md (conservative — only universal patterns)
- [ ] T037 [GIT] Commit: `docs: refresh codebase docs + finalize phase 2 retro`

### Phase Completion

- [ ] T038 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T039 [GIT] Open or update PR to main with Phase 2 summary (F1+F2+F3 — error variants pre-allocated, regex direct dep, substitution skeleton + override seams)
- [ ] T040 [GIT] Verify all CI checks pass
- [ ] T041 [GIT] Report PR ready status

---

## Phase 3: US1 — Commands as MCP prompts (P1)

**Story goal**: A developer with a workspace containing a plugin that ships both `skills/` and `commands/` can launch a Claude Code session in a bound project, see the plugin's commands in the slash menu, invoke one (with or without args), and have the rendered body land in the conversation. Skills do NOT appear in the slash menu by default.

**Independent test**: From a fresh install with the fixture catalog: enable a plugin containing `skills/foo/SKILL.md` + `commands/bar.md`, bind a project, launch Claude Code, observe that `/mcp__tome__phase5_demo__bar` is in the slash menu and selecting it inserts the rendered body. SKILL.md entries remain absent from the slash menu by default (FR-012).

**MVP scope**: This phase by itself delivers the headline Phase 5 deliverable (commands accessible as slash commands across harnesses).

### Phase Start

- [ ] T100 [GIT] Verify working tree is clean before starting Phase 3
- [ ] T101 Create `specs/005-phase-5-commands-prompts/retro/P3.md` from the standard retro template
- [ ] T102 [GIT] Commit: `docs: initialize phase 3 retro`

### Slice US1.a — Schema migration v2→v3 + frontmatter widening + commands directory walk

- [x] T103 [US1] Add `EntryKind` enum to `src/plugin/identity.rs` (or `src/index/mod.rs`) per data-model.md §1.1; derive `Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize` with `#[serde(rename_all = "lowercase")]` (use devs:rust-dev agent)
- [x] T104 [US1] Extend `EntryFrontmatter` (existing `SkillFrontmatter`) in `src/plugin/frontmatter.rs` with the widened Phase 5 lenient field set per `contracts/frontmatter-p5.md` § Recognised fields (use devs:rust-dev agent)
- [x] T105 [US1] Add custom `deserialize_with` for the `arguments` field accepting both space-separated string AND YAML list, producing `Vec<String>` (use devs:rust-dev agent)
- [x] T106 [US1] Add `resolved_searchable()` and `resolved_user_invocable(kind)` helper methods on `EntryFrontmatter` per `contracts/frontmatter-p5.md` § Resolved defaults (use devs:rust-dev agent)
- [x] T107 [US1] Add argument-name validation: each name must match `^[a-z_][a-z0-9_]*$`; illegal names produce `InvalidArgumentFrontmatter` (exit 29) (use devs:rust-dev agent)
- [x] T108 [US1] Register Phase 5 migration in `src/index/migrations.rs::MIGRATIONS` with `from: 2, to: 3, name: "phase5_entry_kind_unification", apply: phase5_v3_apply` per `contracts/schema-migration-p5.md` (use devs:rust-dev agent)
- [x] T109 [US1] Implement `phase5_v3_apply` function executing the 5-statement DDL (ALTER TABLE ×4 + DROP/CREATE INDEX) inside the migration's transaction (use devs:rust-dev agent)
- [x] T110 [US1] Extend `src/plugin/components.rs` to walk `<plugin>/commands/*.md` non-recursively (flat directory listing, filtered to `*.md`) alongside the existing `skills/*/SKILL.md` recursive walk (use devs:rust-dev agent)
- [x] T111 [US1] Extend `src/plugin/lifecycle.rs::enable_plugin` to plumb `EntryKind` through the enable pipeline; UPSERT both kinds into `skills` table keyed on `(catalog, plugin, kind, name)` (use devs:rust-dev agent)
- [x] T112 [US1] Extend `index::skills::upsert_skill` to write `kind`, `searchable`, `user_invocable`, `when_to_use` columns; UPSERT honours the widened unique constraint (use devs:rust-dev agent)
- [x] T113 [US1] Update `index::skills` queries to filter by `kind` where applicable (lookups by identity tuple include kind) (use devs:rust-dev agent)
- [x] T114 [US1] Update `embedding_text` composer in `src/index/skills.rs` (or wherever Phase 4 placed it) to include `when_to_use` per `contracts/entry-schema-p5.md` § Embedding text composition (use devs:rust-dev agent)
- [x] T115 [GIT] Commit: `feat(schema): v3 migration adds kind/searchable/user_invocable/when_to_use; walks commands directory`
- [x] T116 [US1] Create `tests/schema_migration_v3.rs` with 4-6 tests per `contracts/schema-migration-p5.md` § Testing (v2→v3 happy path, backfill defaults, identity preservation, FK preservation, mid-tx failure rollback) (use devs:rust-dev agent)
- [x] T117 [US1] Create `tests/frontmatter_p5_fields.rs` with the 8 tests per `contracts/frontmatter-p5.md` § Tests (use devs:rust-dev agent)
- [x] T118 [US1] Create `tests/entry_kind_indexing.rs` with 5-7 tests covering: both directories index, kind discriminator, same-name-different-kind, workspace_skills syncs both kinds, when_to_use contributes to embedding_text, content_hash invalidated when when_to_use changes (use devs:rust-dev agent)
- [x] T119 [US1] Run `cargo test --test schema_migration_v3 --test frontmatter_p5_fields --test entry_kind_indexing` and confirm all green (use devs:rust-dev agent)
- [x] T120 [GIT] Commit: `test(phase5): schema migration + frontmatter + entry kind indexing`
- [ ] T121 [GIT] Push US1.a slice and open sub-PR

### Slice US1.b — MCP prompts capability + `prompts/list` + name derivation + collisions

- [x] T122 [US1] Verify rmcp prompts API shape against current rmcp version (per research §R-14): inspect rmcp crate's `ServerCapabilities`, `PromptsCapability`, `#[prompt_router]` macro, request/response types; record findings in a new `specs/005-phase-5-commands-prompts/notes/rmcp-prompts-api.md` (use devs:rust-dev agent)
- [x] T123 [US1] Create `src/mcp/prompt_name.rs` with `derive_name(entry, override)` + `sanitise(s)` + `sanitise_trunc(s, max)` per `contracts/mcp-prompts.md` § Prompt name derivation. Constants `PLUGIN_PORTION_MAX = 16`, `ENTRY_PORTION_MAX = 32`, `SEPARATOR = "__"` (use devs:rust-dev agent)
- [x] T124 [US1] Create `src/mcp/prompt_collision.rs` with `CollisionRecord`, `EntryIdentity`, `resolve_collisions(entries) -> (Vec<PromptDescriptor>, Vec<CollisionRecord>)` per `contracts/mcp-prompts.md` § Collision handling (use devs:rust-dev agent)
- [x] T125 [US1] Create `src/mcp/prompts.rs` with `PromptDescriptor`, `PromptArgument`, `PromptListResponse`, `PromptGetResponse`, `PromptMessage`, `PromptContent` (Serialize, JsonSchema derives per data-model.md §4.4) (use devs:rust-dev agent)
- [x] T126 [US1] Add `PromptRegistry` struct to `src/mcp/state.rs` (HashMap<String, EntryRow> + Vec<CollisionRecord>) and extend `McpState` with `prompt_registry: Arc<PromptRegistry>` field (use devs:rust-dev agent)
- [x] T127 [US1] Implement `PromptRegistry::build_for_workspace(workspace_id, conn)` that queries enabled-and-user-invocable entries, derives prompt names (applying overrides), and resolves collisions (use devs:rust-dev agent)
- [x] T128 [US1] Extend MCP server initialization in `src/mcp/server.rs` to declare `PromptsCapability { list_changed: Some(false) }` per `contracts/mcp-prompts.md` § Capability declaration (use devs:rust-dev agent)
- [x] T129 [US1] Implement `prompts/list` handler in `src/mcp/prompts.rs` using rmcp's `#[prompt_router]` (or equivalent) macro; dispatches sync work via `spawn_blocking` (use devs:rust-dev agent)
- [x] T130 [US1] Implement argument schema derivation for prompts: named case (FR-070) + catch-all `args` case (FR-071/072) per `contracts/mcp-prompts.md` § Argument schema derivation (use devs:rust-dev agent)
- [x] T131 [US1] Wire `PromptRegistry` construction at MCP startup in `src/mcp/mod.rs::run` (after preflight checks) (use devs:rust-dev agent)
- [ ] T132 [GIT] Commit: `feat(mcp): prompts capability + prompts/list + name derivation + collisions`
- [x] T133 [US1] Create `tests/prompt_naming.rs` with 6-8 tests: sanitisation, truncation per portion, override replaces both portions, harness prefix preserved by harness (use devs:rust-dev agent)
- [x] T134 [US1] Create `tests/prompt_collision.rs` with 4-6 tests: counter starts at 2, tie-break on (catalog, plugin, kind, name) lex order, collision logged at warn level (use devs:rust-dev agent)
- [x] T135 [US1] Create `tests/mcp_prompts.rs` with tests for `prompts/list` shape, user_invocable filter, both kinds included, named-args schema, catch-all-args schema, listChanged: false declared (use devs:rust-dev agent)
- [x] T136 [US1] Create `tests/mcp_prompts_list_json_shape.rs` byte-stable JSON pin per research §R-19 (use devs:rust-dev agent)
- [ ] T137 [GIT] Commit: `test(mcp): prompts/list + name derivation + collisions`
- [ ] T138 [GIT] Push US1.b slice and update PR

### Slice US1.c — `prompts/get` + substitution wiring (placeholder substitution for now)

- [ ] T139 [US1] Implement `prompts/get` handler in `src/mcp/prompts.rs`: resolve name via `PromptRegistry.by_name`, read entry body, run `substitution::render()` (currently a no-op stub from F3), wrap in `PromptGetResponse` per `contracts/mcp-prompts.md` (use devs:rust-dev agent)
- [ ] T140 [US1] Build `SubstitutionContext` for a `prompts/get` call: populate all 12 built-in values + clock from `SUBSTITUTION_CLOCK_OVERRIDE` or `time::OffsetDateTime::now_local()` + caller args (use devs:rust-dev agent)
- [ ] T141 [US1] Map rmcp's prompts/get arguments parameter to `ArgumentValues::Object { named, declared_order }` for entries with declared arguments; map to `ArgumentValues::Single(s)` for entries with no declared arguments (per FR-041–FR-043) (use devs:rust-dev agent)
- [ ] T142 [US1] Add MCP error responses per `contracts/mcp-prompts.md` § Error responses (prompt_not_found / prompt_argument_mismatch / substitution_failed / workspace_data_dir_write_failed) (use devs:rust-dev agent)
- [ ] T143 [GIT] Commit: `feat(mcp): prompts/get wired to substitution layer (no-op stub)`
- [ ] T144 [US1] Extend `tests/mcp_prompts.rs` with `prompts/get` tests: structured args, single-string arg, no args, error cases (use devs:rust-dev agent)
- [ ] T145 [US1] Create `tests/mcp_prompts_get_json_shape.rs` byte-stable JSON pin (use devs:rust-dev agent)
- [ ] T146 [GIT] Commit: `test(mcp): prompts/get + JSON wire-shape pin`
- [ ] T147 [GIT] Push US1.c slice and update PR

### Slice US1.d — Reviewer pass + US1 closeout

- [ ] T148 [US1] Dispatch 4 reviewer agents in parallel (contract / Rust-lens / test / security) with per-scope briefs writing to `/tmp/tome-phase5-us1-{contract,rust,test,security}.md` (per research §R-18)
- [ ] T149 [US1] Consolidate reviewer findings into `specs/005-phase-5-commands-prompts/review/us1-findings.md` + `us1-disposition.md`
- [ ] T150 [US1] Apply all reviewer blockers and selected majors; commit fixes
- [ ] T151 [GIT] Commit: `fix(phase5/us1): reviewer-pass blockers and selected majors`
- [ ] T152 [US1] Run `/sdd:map incremental` to refresh `.sdd/codebase/` against US1 changes
- [ ] T153 [US1] Review `retro/P3.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T154 [GIT] Commit: `docs: refresh codebase docs + finalize phase 3 retro`

### Phase Completion

- [ ] T155 [GIT] Push final US1 PRs (per-slice)
- [ ] T156 [GIT] Verify all CI checks pass on each US1 sub-PR
- [ ] T157 [GIT] Report US1 PR ready status

---

## Phase 4: US2 — Substitution layer (paths/env) (P2)

**Story goal**: A skill author writes a body containing `${TOME_SKILL_DIR}/scripts/audit.py`, `${TOME_PLUGIN_DATA}`, `${TOME_ENV_DEPLOY_TARGET:-staging}`, and `${GITHUB_TOKEN}`. Retrieving the body through either `get_skill` or `prompts/get` resolves the Tome built-ins to absolute paths, resolves the env passthrough to the host value or default, and leaves `${GITHUB_TOKEN}` verbatim.

**Independent test**: Author the substitution-bearing body, retrieve via both `get_skill` (no args) and `prompts/get` (no args), verify substitutions correct and secret left untouched. Verify `${TOME_PLUGIN_DATA}` directory exists on disk after retrieval.

### Phase Start

- [ ] T200 [GIT] Verify working tree is clean before starting Phase 4
- [ ] T201 Create `specs/005-phase-5-commands-prompts/retro/P4.md` from the standard retro template
- [ ] T202 [GIT] Commit: `docs: initialize phase 4 retro`

### Slice US2.a — Built-ins stage + clock injection

- [ ] T203 [US2] Compile `BUILTIN_REGEX` (`\$\{TOME_([A-Z0-9_]+)(?::-(.*?))?\}`) in `src/substitution/regex.rs::builtin_regex()` (OnceLock-cached per research §R-2) (use devs:rust-dev agent)
- [ ] T204 [US2] Implement whitelisted 12-built-in resolver in `src/substitution/builtins.rs::resolve_builtin(name, ctx, default)`: SKILL_DIR / SKILL_PATH / SKILL_NAME / PLUGIN_DIR / PLUGIN_NAME / PLUGIN_VERSION / PLUGIN_DATA / CATALOG_NAME / WORKSPACE_NAME / WORKSPACE_DATA / DATE / TIMESTAMP per `contracts/substitution-engine.md` § Stage 1 (use devs:rust-dev agent)
- [ ] T205 [US2] Implement `apply_builtins(body, ctx)` using regex `replace_all` callback that resolves each match via `resolve_builtin`; unknown names pass through with `tracing::debug!` (FR-023) (use devs:rust-dev agent)
- [ ] T206 [US2] Implement path-component sanitisation in `src/substitution/data_dir.rs::sanitise_path_component(s)`: replace non-`[A-Za-z0-9._-]` with `_` per FR-024 (use devs:rust-dev agent)
- [ ] T207 [US2] Implement `ensure_plugin_data(paths, catalog, plugin)` with lazy `create_dir_all` + `PLUGIN_DATA_DIR_OVERRIDE` consultation per research §R-9 (use devs:rust-dev agent)
- [ ] T208 [US2] Wire `apply_builtins` into `substitution::render()` stage 1 (use devs:rust-dev agent)
- [ ] T209 [US2] Wire clock injection: `context.clock` defaults to `time::OffsetDateTime::now_local()` unless `SUBSTITUTION_CLOCK_OVERRIDE` is set (use devs:rust-dev agent)
- [ ] T210 [GIT] Commit: `feat(substitution): built-ins stage + clock injection + path sanitisation`
- [ ] T211 [US2] Create `tests/substitution_builtins.rs` with tests per `contracts/substitution-engine.md` § Stage 1 (12 builtins resolve, unknown pass through, default syntax, path sanitisation, clock injection produces deterministic DATE/TIMESTAMP) (use devs:rust-dev agent)
- [ ] T212 [GIT] Commit: `test(substitution): built-ins stage`
- [ ] T213 [GIT] Push US2.a slice

### Slice US2.b — Env passthrough stage + lazy data directory creation + workspace rename relocation

- [ ] T214 [US2] Compile `ENV_REGEX` (`\$\{TOME_ENV_([A-Z0-9_]+)(?::-(.*?))?\}`) in `src/substitution/regex.rs::env_regex()` (OnceLock-cached) (use devs:rust-dev agent)
- [ ] T215 [US2] Implement `apply_env(body)` in `src/substitution/env.rs`: `std::env::var(format!("TOME_ENV_{}", name))` lookup with prefix preserved, default-value fallback, debug-log on unset-no-default per FR-030–FR-033 (use devs:rust-dev agent)
- [ ] T216 [US2] Wire `apply_env` into `substitution::render()` stage 2 (use devs:rust-dev agent)
- [ ] T217 [US2] Implement `ensure_workspace_data(paths, workspace, catalog, plugin)` with same lazy + override pattern as plugin_data (use devs:rust-dev agent)
- [ ] T218 [US2] Extend `src/workspace/rename.rs` (Phase 4 existing) to relocate `<home>/.tome/workspaces/<old>/plugin-data/` → `<home>/.tome/workspaces/<new>/plugin-data/` if source exists, per FR-025; failure surfaces `WorkspaceDataDirWriteFailed` (exit 25) (use devs:rust-dev agent)
- [ ] T219 [GIT] Commit: `feat(substitution): env passthrough + data-dir creation + workspace rename relocation`
- [ ] T220 [US2] Create `tests/substitution_env.rs` with tests per `contracts/substitution-engine.md` § Stage 2 (set/unset/default; non-namespace passes through; GITHUB_TOKEN-shaped left verbatim) (use devs:rust-dev agent)
- [ ] T221 [US2] Create `tests/substitution_data_dir.rs` with tests: lazy create on first reference, concurrent-safe `create_dir_all`, idempotent on second reference, workspace-rename relocation, failure → exit 25 (use devs:rust-dev agent)
- [ ] T222 [US2] Extend `tests/workspace_rename.rs` (existing) with the new data-dir relocation case (use devs:rust-dev agent)
- [ ] T223 [GIT] Commit: `test(substitution): env passthrough + data-dir creation`
- [ ] T224 [GIT] Push US2.b slice

### Slice US2.c — End-to-end through `get_skill` + reviewer pass + US2 closeout

- [ ] T225 [US2] Extend `src/mcp/tools/get_skill.rs` to invoke `substitution::render()` on the returned body when ANY substitution applies (built-ins + env always; args from US3) per FR-101 (use devs:rust-dev agent)
- [ ] T226 [US2] Create `tests/substitution_pipeline.rs` with tests verifying stage ordering invariant (built-ins → env, no re-scan), and `mcp_prompts_get_runs_builtins_and_env` end-to-end (use devs:rust-dev agent)
- [ ] T227 [GIT] Commit: `feat(mcp): get_skill + prompts/get invoke substitution (builtins + env)`
- [ ] T228 [US2] Dispatch 4 reviewer agents in parallel for US2 with per-scope briefs writing to `/tmp/tome-phase5-us2-*.md`
- [ ] T229 [US2] Consolidate reviewer findings + write disposition; apply blockers + selected majors
- [ ] T230 [GIT] Commit: `fix(phase5/us2): reviewer-pass blockers and selected majors`
- [ ] T231 [US2] Run `/sdd:map incremental` to refresh `.sdd/codebase/` against US2 changes
- [ ] T232 [US2] Review `retro/P4.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T233 [GIT] Commit: `docs: refresh codebase docs + finalize phase 4 retro`

### Phase Completion

- [ ] T234 [GIT] Push final US2 PRs (per-slice)
- [ ] T235 [GIT] Verify all CI checks pass on each US2 sub-PR
- [ ] T236 [GIT] Report US2 PR ready status

---

## Phase 5: US3 — Argument substitution (P3)

**Story goal**: A plugin author ships `commands/migrate-component.md` with `arguments: [component, from, to]` and references `$component`, `$from`, `$to`, `$ARGUMENTS[0]`, `$0`, `$ARGUMENTS` in the body. Invoking the command via the harness slash menu with structured argument inputs renders the body with every reference substituted.

**Independent test**: Author the substitution-bearing command, invoke via `prompts/get` with `{component: "SearchBar", from: "React", to: "Vue"}`, verify all positional and named forms resolve correctly. Invoke a command body lacking any `$ARGUMENTS` reference; verify the append-fallback footer appears.

### Phase Start

- [ ] T250 [GIT] Verify working tree is clean before starting Phase 5
- [ ] T251 Create `specs/005-phase-5-commands-prompts/retro/P5.md` from the standard retro template
- [ ] T252 [GIT] Commit: `docs: initialize phase 5 retro`

### Slice US3.a — Argument substitution stage + four patterns + name binding

- [ ] T253 [US3] Compile `ARG_REGEX` (`\$ARGUMENTS\[(\d+)\]|\$ARGUMENTS|\$(\d+)|\$([a-z_][a-z0-9_]*)`) in `src/substitution/regex.rs::arg_regex()` (OnceLock-cached) (use devs:rust-dev agent)
- [ ] T254 [US3] Implement shell-style quoting parser in `src/substitution/arguments.rs::shell_split(s)`: whitespace separates; single OR double quotes preserve internal whitespace; no escapes; no nested quoting per research §R-10 (use devs:rust-dev agent)
- [ ] T255 [US3] Implement `coerce_arguments(args, declared) -> ResolvedArguments` that materialises positional + named maps per `contracts/substitution-engine.md` § Stage 3 caller coercion table (use devs:rust-dev agent)
- [ ] T256 [US3] Implement `apply_arguments(body, args, declared)` returning `(rendered_body, replacements_performed: bool)`; uses `regex::Regex::replace_all` with a callback resolving each capture group per the four patterns per FR-040 (use devs:rust-dev agent)
- [ ] T257 [US3] Wire `apply_arguments` into `substitution::render()` stage 3, gated on `ctx.args.is_some()` (use devs:rust-dev agent)
- [ ] T258 [GIT] Commit: `feat(substitution): argument stage + four patterns + name binding`
- [ ] T259 [US3] Create `tests/substitution_arguments.rs` with tests per `contracts/substitution-engine.md` § Stage 3: $ARGUMENTS, $ARGUMENTS[N], $N, $name, out-of-range, single-string vs object, shell-split rules, no-re-scan invariant (use devs:rust-dev agent)
- [ ] T260 [GIT] Commit: `test(substitution): argument stage`
- [ ] T261 [GIT] Push US3.a slice

### Slice US3.b — ARGUMENTS append-fallback footer

- [ ] T262 [US3] Implement append-fallback logic in `substitution::render()` stage 4: when caller supplied args AND stage 3 returned `replacements_performed: false`, append `\n\nARGUMENTS: <value>` to body per FR-044 + research §R-13 (use devs:rust-dev agent)
- [ ] T263 [US3] Compute `<value>` per `contracts/substitution-engine.md` § Stage 4: single-string verbatim OR positional values joined by single space (use devs:rust-dev agent)
- [ ] T264 [US3] Handle edge case: body ends with `\n` (no need for separator) vs body ends with non-newline (separator needed) (use devs:rust-dev agent)
- [ ] T265 [GIT] Commit: `feat(substitution): ARGUMENTS append-fallback footer`
- [ ] T266 [US3] Extend `tests/substitution_arguments.rs` with append-fallback tests: triggered when no body references, NOT triggered when any reference matched, footer format matches contract, newline-suffix edge case (use devs:rust-dev agent)
- [ ] T267 [GIT] Commit: `test(substitution): append-fallback footer`
- [ ] T268 [GIT] Push US3.b slice

### Slice US3.c — End-to-end via `prompts/get` + `get_skill` with args + reviewer pass + US3 closeout

- [ ] T269 [US3] Extend `src/mcp/tools/get_skill.rs` arg-handling path to invoke substitution stage 3 when caller passes `args` field (use devs:rust-dev agent)
- [ ] T270 [US3] Verify `prompts/get` arg dispatch from US1.c (T141) correctly maps named/positional to `ArgumentValues` (use devs:rust-dev agent)
- [ ] T271 [US3] Add `prompt_argument_mismatch` error path: when caller supplies more args than declared OR named args not in declaration set, surface MCP error per `contracts/exit-codes-p5.md` (use devs:rust-dev agent)
- [ ] T272 [GIT] Commit: `feat(mcp): prompts/get + get_skill arg substitution end-to-end`
- [ ] T273 [US3] Create `tests/entry_e2e.rs` end-to-end test exercising the full enable → index → search → info → get → prompts pipeline (use devs:rust-dev agent)
- [ ] T274 [GIT] Commit: `test(phase5): end-to-end through enable/index/search/info/get/prompts`
- [ ] T275 [US3] Dispatch 4 reviewer agents in parallel for US3 with per-scope briefs
- [ ] T276 [US3] Consolidate reviewer findings + write disposition; apply blockers + selected majors
- [ ] T277 [GIT] Commit: `fix(phase5/us3): reviewer-pass blockers and selected majors`
- [ ] T278 [US3] Run `/sdd:map incremental` to refresh `.sdd/codebase/` against US3 changes
- [ ] T279 [US3] Review `retro/P5.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T280 [GIT] Commit: `docs: refresh codebase docs + finalize phase 5 retro`

### Phase Completion

- [ ] T281 [GIT] Push final US3 PRs
- [ ] T282 [GIT] Verify all CI checks pass
- [ ] T283 [GIT] Report US3 PR ready status

---

## Phase 6: US4 — Middle-tier discovery + `when_to_use` indexing (P4)

**Story goal**: An agent doing semantic search receives a small ranked list with truncated descriptions, then calls the new middle-tier tool naming one candidate. The response carries full description + `when_to_use` + plugin version + user-invocable flag + absolute path + a one-level resource enumeration capped at 5 children per directory. The when_to_use field also contributes to the embedding so queries semantically related to the disambiguation hint retrieve the entry.

**Independent test**: From an install with a heavy-resource skill, call `get_skill_info` for it; verify response shape per `contracts/mcp-tools-p5.md` § `get_skill_info`. Verify SC-004 (response is order-of-magnitude smaller than `get_skill` body for the same entry). Verify `search_skills` results have descriptions truncated to 150 chars by default.

### Phase Start

- [ ] T300 [GIT] Verify working tree is clean before starting Phase 6
- [ ] T301 Create `specs/005-phase-5-commands-prompts/retro/P6.md` from the standard retro template
- [ ] T302 [GIT] Commit: `docs: initialize phase 6 retro`

### Slice US4.a — `get_skill_info` middle-tier tool

- [ ] T303 [US4] Create `src/mcp/tools/get_skill_info.rs` with `GetSkillInfoInput` (Deserialize + JsonSchema) per data-model.md §4.2 (use devs:rust-dev agent)
- [ ] T304 [US4] Define `SkillInfo` + `ResourceEnumeration` (Serialize + JsonSchema) per data-model.md §4.2 (use devs:rust-dev agent)
- [ ] T305 [US4] Implement `walk_resources(entry_dir, entry_file_basename) -> ResourceEnumeration` per `contracts/mcp-tools-p5.md` § Resource enumeration rules: top-level files (cap 5 + sentinel), subdirs with per-directory cap 5 + sentinel, alphabetical sort, skip the entry file itself (use devs:rust-dev agent)
- [ ] T306 [US4] Implement the tool handler: look up entry by `(catalog, plugin, kind, name)`, populate full description + when_to_use + plugin_version + user_invocable + path + (resources for skill-kind only per FR-083) (use devs:rust-dev agent)
- [ ] T307 [US4] Register the new tool in `src/mcp/server.rs` alongside `search_skills` and `get_skill` via the `#[tool_router]` macro (use devs:rust-dev agent)
- [ ] T308 [US4] Add MCP error responses per `contracts/mcp-tools-p5.md` § Error responses (entry_not_found / invalid_kind / resource_enum_failed) (use devs:rust-dev agent)
- [ ] T309 [GIT] Commit: `feat(mcp): get_skill_info middle-tier tool + resource enumeration`
- [ ] T310 [US4] Create `tests/mcp_get_skill_info.rs` with tests per `contracts/mcp-tools-p5.md` § Tests: skill-kind includes resources, command-kind omits resources, per-directory cap with sentinel, default kind = skill, kind disambiguation (use devs:rust-dev agent)
- [ ] T311 [US4] Create `tests/mcp_get_skill_info_json_shape.rs` byte-stable JSON pin (use devs:rust-dev agent)
- [ ] T312 [GIT] Commit: `test(mcp): get_skill_info + JSON pin`
- [ ] T313 [GIT] Push US4.a slice

### Slice US4.b — `when_to_use` indexing + reindex re-eval

- [ ] T314 [US4] Verify embedding_text composer (already extended in US1.a T114) produces the documented format per `contracts/entry-schema-p5.md` § Embedding text composition (use devs:rust-dev agent)
- [ ] T315 [US4] Run reindex pass against the fixture catalog; confirm rows whose frontmatter has `when_to_use` re-embed because content_hash changed (use devs:rust-dev agent)
- [ ] T316 [US4] Extend `tests/entry_kind_indexing.rs` with `when_to_use_change_invalidates_content_hash` and `query_semantically_matches_when_to_use_text` (use devs:rust-dev agent)
- [ ] T317 [GIT] Commit: `feat(index): when_to_use contributes to embedding_text`
- [ ] T318 [GIT] Push US4.b slice

### Slice US4.c — `search_skills` truncation parameter + reviewer pass + US4 closeout

- [ ] T319 [US4] Extend `SearchSkillsInput` in `src/mcp/tools/search_skills.rs` with `description_max_chars: u32` (default 150) per `contracts/mcp-tools-p5.md` (use devs:rust-dev agent)
- [ ] T320 [US4] Extend `SearchResult` with `kind: EntryKind` field per FR-091 (use devs:rust-dev agent)
- [ ] T321 [US4] Implement truncation: if description char count > `description_max_chars`, slice at boundary and append `…` (U+2026); preserve full description if shorter per FR-092 (use devs:rust-dev agent)
- [ ] T322 [US4] Add `WHERE searchable = 1` filter to the existing search_skills DB query (FR-090) (use devs:rust-dev agent)
- [ ] T323 [US4] Add input validation: `description_max_chars < 0` → `invalid_description_max_chars` MCP error (use devs:rust-dev agent)
- [ ] T324 [GIT] Commit: `feat(mcp): search_skills description truncation + kind in result`
- [ ] T325 [US4] Create `tests/mcp_search_skills_truncation.rs` with tests: default 150 truncation, override via parameter, ellipsis appended, kind in result, disable_model_invocation excluded (use devs:rust-dev agent)
- [ ] T326 [US4] Create `tests/mcp_search_skills_json_shape.rs` byte-stable JSON pin (extend existing if applicable) (use devs:rust-dev agent)
- [ ] T327 [GIT] Commit: `test(mcp): search_skills truncation + kind + searchable filter`
- [ ] T328 [US4] Dispatch 4 reviewer agents in parallel for US4 with per-scope briefs
- [ ] T329 [US4] Consolidate reviewer findings + write disposition; apply blockers + selected majors
- [ ] T330 [GIT] Commit: `fix(phase5/us4): reviewer-pass blockers and selected majors`
- [ ] T331 [US4] Run `/sdd:map incremental` to refresh `.sdd/codebase/` against US4 changes
- [ ] T332 [US4] Review `retro/P6.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T333 [GIT] Commit: `docs: refresh codebase docs + finalize phase 6 retro`

### Phase Completion

- [ ] T334 [GIT] Push final US4 PRs
- [ ] T335 [GIT] Verify all CI checks pass
- [ ] T336 [GIT] Report US4 PR ready status

---

## Phase 7: US5 — Per-entry invocability flags + doctor extensions (P5)

**Story goal**: A plugin author ships entries spanning the matrix of two boolean flags. After indexing, searches exclude `disable-model-invocation: true` entries; prompts surfaces include `user-invocable: true` entries; `tome plugin show` annotates each entry's effective flags and derived prompt name; `tome doctor` reports the prompts surface, collisions, orphan data directories, and per-kind entry counts.

**Independent test**: Plugin ships a 4-entry matrix (default-skill, default-command, model-invocation-disabled, user-invocable-skill). After enable: agent search results contain 3 of 4; harness slash menu contains 2 of 4. `tome plugin show` annotations match. `tome doctor` reports the prompts list correctly grouped.

### Phase Start

- [ ] T350 [GIT] Verify working tree is clean before starting Phase 7
- [ ] T351 Create `specs/005-phase-5-commands-prompts/retro/P7.md` from the standard retro template
- [ ] T352 [GIT] Commit: `docs: initialize phase 7 retro`

### Slice US5.a — End-to-end frontmatter flag honoring through search + prompts surfaces

- [ ] T353 [US5] Verify `searchable` filter is honoured in `search_skills` query (already in T322); confirm test coverage in `tests/mcp_search_skills_truncation.rs::disable_model_invocation_excluded` (use devs:rust-dev agent)
- [ ] T354 [US5] Verify `user_invocable` filter is honoured in `PromptRegistry::build_for_workspace` (already in T127); confirm test coverage in `tests/mcp_prompts.rs::list_excludes_non_invocable` (use devs:rust-dev agent)
- [ ] T355 [US5] Add a 4-entry matrix fixture plugin to `tests/fixtures/sample-plugin-catalog/` covering: default skill, default command, disable-model-invocation skill, user-invocable skill (use devs:rust-dev agent)
- [ ] T356 [US5] Create `tests/entry_e2e.rs` end-to-end test verifying the matrix: search has 3 (excludes disabled), prompts has 2 (default-command + user-invocable-skill), neither surface contains the both-flags-disabled "dormant" entry (use devs:rust-dev agent)
- [ ] T357 [GIT] Commit: `test(phase5/us5): per-entry invocability matrix end-to-end`
- [ ] T358 [GIT] Push US5.a slice

### Slice US5.b — `tome plugin show` annotations + `tome doctor` Phase 5 surface

- [ ] T359 [US5] Extend `src/commands/plugin/show.rs` to render Skills and Commands sections separately, each entry annotated with `searchable=` / `user_invocable=` / `prompt=<derived>` / `[dormant]` per `contracts/catalog-and-plugin-extensions-p5.md` § Human-mode output (use devs:rust-dev agent)
- [ ] T360 [US5] Extend `tome plugin show --json` output shape per `contracts/catalog-and-plugin-extensions-p5.md` § JSON-mode output (use devs:rust-dev agent)
- [ ] T361 [US5] Extend `src/commands/plugin/list.rs` count format to include commands per `contracts/catalog-and-plugin-extensions-p5.md` § `tome plugin list` (use devs:rust-dev agent)
- [ ] T362 [US5] Add `PromptsReport`, `OrphanDataDirReport`, `EntryCountsByKind` types to `src/doctor/report.rs` per `contracts/doctor-extensions-p5.md` § DoctorReport struct extension (use devs:rust-dev agent)
- [ ] T363 [US5] Extend `DoctorReport` with `prompts`, `orphan_data_dirs`, `entry_counts` Option fields (use devs:rust-dev agent)
- [ ] T364 [US5] Implement `doctor::checks::build_prompts_report(workspace, conn)` reusing US1's `PromptRegistry::build_for_workspace` (use devs:rust-dev agent)
- [ ] T365 [US5] Implement `doctor::checks::detect_orphan_data_dirs(paths, conn)` per `contracts/doctor-extensions-p5.md` § Detection algorithm (use devs:rust-dev agent)
- [ ] T366 [US5] Implement `doctor::checks::count_entries_by_kind(workspace, conn)` including `pending_re_embedding` count (use devs:rust-dev agent)
- [ ] T367 [US5] Extend `doctor::assemble_report` to populate the three new sections when in-workspace; emit None when outside-project per Phase 4 convention (use devs:rust-dev agent)
- [ ] T368 [US5] Extend human-mode + JSON-mode rendering in `src/commands/doctor.rs` per `contracts/doctor-extensions-p5.md` § Human-mode rendering (use devs:rust-dev agent)
- [ ] T369 [US5] Enforce read-only invariant (FR-124): doctor must NOT lazy-create data dirs; verify by snapshotting `<home>/.tome/` before/after (use devs:rust-dev agent)
- [ ] T370 [GIT] Commit: `feat(doctor,plugin-show): phase 5 surfaces — prompts, orphans, entry counts, per-entry annotations`
- [ ] T371 [US5] Create `tests/plugin_show_p5.rs` with tests per `contracts/catalog-and-plugin-extensions-p5.md` § Tests (use devs:rust-dev agent)
- [ ] T372 [US5] Create `tests/plugin_show_p5_json_shape.rs` byte-stable JSON pin (use devs:rust-dev agent)
- [ ] T373 [US5] Create `tests/doctor_p5.rs` with tests per `contracts/doctor-extensions-p5.md` § Tests (use devs:rust-dev agent)
- [ ] T374 [US5] Extend `tests/doctor_json.rs` with Phase 5 field serialisation pins (use devs:rust-dev agent)
- [ ] T375 [GIT] Commit: `test(phase5/us5): plugin show + doctor + JSON pins`
- [ ] T376 [GIT] Push US5.b slice

### Slice US5.c — Reviewer pass + US5 closeout

- [ ] T377 [US5] Dispatch 4 reviewer agents in parallel for US5 with per-scope briefs
- [ ] T378 [US5] Consolidate reviewer findings + write disposition; apply blockers + selected majors
- [ ] T379 [GIT] Commit: `fix(phase5/us5): reviewer-pass blockers and selected majors`
- [ ] T380 [US5] Run `/sdd:map incremental` to refresh `.sdd/codebase/` against US5 changes
- [ ] T381 [US5] Review `retro/P7.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T382 [GIT] Commit: `docs: refresh codebase docs + finalize phase 7 retro`

### Phase Completion

- [ ] T383 [GIT] Push final US5 PRs
- [ ] T384 [GIT] Verify all CI checks pass
- [ ] T385 [GIT] Report US5 PR ready status

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Phase-wide reviewer pass surfacing blockers / majors / minors across the whole Phase 5 surface. Disposition routing. 5–6 polish PRs. Final sdd:map + retro + CLAUDE.md + v0.5.0 release prep.

### Phase Start

- [ ] T400 [GIT] Verify working tree is clean before starting Phase 8
- [ ] T401 Create `specs/005-phase-5-commands-prompts/retro/P8.md` from the standard retro template
- [ ] T402 [GIT] Commit: `docs: initialize phase 8 polish retro`

### PR-A — Phase-wide reviewer findings + disposition

- [ ] T403 Dispatch 4 reviewer agents in parallel against the merged Phase 5 surface (contract / Rust-lens / test / security) with per-scope briefs writing to `/tmp/tome-phase5-polish-*.md`
- [ ] T404 Consolidate to `specs/005-phase-5-commands-prompts/review/findings.md` + `disposition.md`
- [ ] T405 [GIT] Commit: `docs(phase5): polish reviewer findings + disposition`
- [ ] T406 [GIT] Push PR-A and open polish PR

### PR-B — Polish blockers

- [ ] T407 Apply all blockers from `disposition.md` § Blockers
- [ ] T408 Run `cargo test` and confirm all gates green
- [ ] T409 [GIT] Commit: `fix(phase5/polish): blockers from reviewer pass`
- [ ] T410 [GIT] Push PR-B

### PR-C — Selected majors (production-code refactors)

- [ ] T411 Apply selected majors from disposition (production-code refactors, helper consolidations, additional safety guards)
- [ ] T412 Run `cargo test` and confirm all gates green
- [ ] T413 [GIT] Commit: `refactor(phase5/polish): selected majors`
- [ ] T414 [GIT] Push PR-C

### PR-D — Test coverage gaps

- [ ] T415 Apply test-coverage gaps from disposition (additional JSON wire-shape pins, exit-code CLI binary tests, security hardening tests)
- [ ] T415a Add `tests/substitution_pipeline.rs::nfr_009_linear_scaling_benchmark` exercising `substitution::render()` on bodies of sizes 1KB, 10KB, 100KB; assert per-byte render cost is bounded by a constant multiple (verifies NFR-009 + NFR-011 in one test; tolerance generous since this is a smoke benchmark, not a competitive perf test) (use devs:rust-dev agent)
- [ ] T415b Add `tests/substitution_pipeline.rs::nfr_004_no_fs_outside_data_dirs` that wraps `substitution::render()` invocation in a tempdir-rooted filesystem snapshot, asserts no new files/dirs appear outside the documented data dir paths (verifies NFR-004) (use devs:rust-dev agent)
- [ ] T415c Add `tests/mcp_prompts.rs::nfr_006_empty_user_invocable_set_still_declares_capability`: bootstrap a workspace with all entries having `user_invocable=false`; assert MCP server declares `prompts` capability AND `prompts/list` returns `[]` (verifies NFR-006) (use devs:rust-dev agent)
- [ ] T415d Add `tests/query.rs::fr_093_skill_only_ranking_unchanged_from_phase4_baseline`: index a fixture plugin containing only skills (no commands directory); run `tome query "test query"`; capture result identities + scores; assert byte-stability against a Phase 4-compatible fixture (verifies FR-093) (use devs:rust-dev agent)
- [ ] T416 [GIT] Commit: `test(phase5/polish): coverage gaps + NFR-004/006/009/011 + FR-093`
- [ ] T417 [GIT] Push PR-D

### PR-E — Security hardening

- [ ] T418 Apply security findings: confirm `${TOME_ENV_*}` namespace strictly enforced; confirm `${GITHUB_TOKEN}` etc. pass through verbatim (extend `tests/scrubbing.rs` or `tests/security_hardening.rs`); confirm doctor read-only invariant; confirm data-dir creation honours symlink refusal (if applicable)
- [ ] T419 [GIT] Commit: `security(phase5/polish): namespace and read-only invariants`
- [ ] T420 [GIT] Push PR-E

### PR-F — Docs + version bump

- [ ] T421 Bump `Cargo.toml` from `0.5.0-dev` to `0.5.0` (use devs:rust-dev agent)
- [ ] T422 Add `[0.5.0]` entry to `CHANGELOG.md` summarising new exit codes (21, 22, 23, 25, 26), new `regex` direct dep, new substitution module, new `get_skill_info` tool, new MCP prompts capability, new doctor surfaces (use devs:rust-dev agent)
- [ ] T423 Update `README.md` Phase 5 status from "planning" to "shipped (v0.5.0)" (use devs:rust-dev agent)
- [ ] T424 Run `cargo build --release` and record final stripped binary size in `RELEASE-BINARY-SIZE.md` (use devs:rust-dev agent)
- [ ] T425 [GIT] Commit: `chore(release): v0.5.0`
- [ ] T426 [GIT] Push PR-F

### PR-G — Closeout

- [ ] T427 Run `/sdd:map incremental` (4 parallel mappers) for final codebase docs refresh
- [ ] T428 Fill `retro/P8.md` with Polish phase learnings: phase-wide reviewer pattern, sub-agent dispatch for high-LOC refactors, JSON wire-shape pin discipline, etc.
- [ ] T429 Extract critical Phase 5 learnings to CLAUDE.md (conservative; only universal patterns)
- [ ] T430 [GIT] Commit: `docs(phase5): closeout — final sdd:map, polish retro, claude.md`
- [ ] T431 [GIT] Push PR-G and open closeout PR
- [ ] T432 [GIT] Verify all CI checks pass on each polish PR
- [ ] T433 [GIT] Report Phase 5 / v0.5.0 ready status

---

## Dependencies

### Critical path

```
Phase 1 (Setup)
    ↓
Phase 2 (Foundational F1-F3) ← BLOCKS all user stories
    ↓
Phase 3 (US1 — Commands as MCP prompts) ← MVP
    ↓
Phase 4 (US2 — Substitution paths/env)
    ↓
Phase 5 (US3 — Argument substitution) ← Final substitution surface
    ↓
Phase 6 (US4 — Middle-tier + when_to_use)  [can run partially-parallel with US3]
    ↓
Phase 7 (US5 — Per-entry flags + doctor)
    ↓
Phase 8 (Polish)
```

### Per-slice dependencies

| Slice | Depends on | Blocks |
|---|---|---|
| US1.a (schema + frontmatter + walks) | F1, F3 | US1.b, US1.c, US3.a, US4.b |
| US1.b (prompts capability + list) | US1.a | US1.c |
| US1.c (prompts/get + sub wiring) | US1.b, F3 (stub OK) | US2.c, US3.c |
| US2.a (built-ins stage) | F3 | US2.c |
| US2.b (env + data-dir + rename) | F3 | US2.c |
| US2.c (e2e + reviewer) | US2.a, US2.b, US1.c | US3.c |
| US3.a (arg stage) | F3 | US3.c |
| US3.b (append-fallback) | US3.a | US3.c |
| US3.c (e2e + reviewer) | US3.a, US3.b, US2.c | US5.a |
| US4.a (get_skill_info) | US1.a | US5.b |
| US4.b (when_to_use indexing) | US1.a | — |
| US4.c (search truncation) | US1.a | — |
| US5.a (flag matrix e2e) | US1, US2, US3, US4 | US5.b |
| US5.b (plugin show + doctor) | US5.a | — |
| Polish | All US closed | — |

### Parallel opportunities

Within US1.a: T103, T104, T105, T106, T107 are sibling parser-side edits and can land in any order. T108, T109 are migration; T110, T111, T112, T113 are pipeline edits and can run in parallel with the migration side.

Within US2.a + US2.b: built-ins stage and env passthrough stage are independent; can run in parallel.

US4.a + US4.b + US4.c are largely independent and can run in parallel; merge order doesn't matter.

US5.b: T362-T367 (doctor checks) and T359-T361 (plugin show) are independent; can run in parallel.

## Implementation strategy

### MVP increment (after Phase 3 / US1)

After US1 closes, the headline Phase 5 deliverable is live: commands ship as MCP prompts. A user installing Tome from this point gets:
- Plugin commands visible in the harness slash menu.
- Substitution layer present but mostly stubbed (built-ins, env, args all no-op until US2/US3).
- Existing `get_skill` and `search_skills` tools work but without Phase 5 enhancements.

This is the minimum viable Phase 5 — every other story adds value but isn't required to demonstrate the central capability.

### Incremental delivery

- After US2 closes: substitution layer is fully functional for path/env references; skill authors can write portable bodies.
- After US3 closes: argument substitution complete; commands with structured args work end-to-end through prompts/get.
- After US4 closes: `get_skill_info` middle-tier tool ships; agent discovery becomes three-tier; `when_to_use` improves retrieval quality.
- After US5 closes: per-entry invocability flags fully honoured; doctor surfaces Phase 5 state.
- After Polish: v0.5.0 ships; reviewer-pass blockers addressed; security hardened; docs updated.

### Risk and pacing

| Risk | Mitigation |
|---|---|
| F3 skeleton-to-production wiring gap (per Phase 4 P6 lesson) | First commit of US2.a / US3.a explicitly grep for "TODO: flip" / "PLACEHOLDER" / stub patterns. Slice 1 of each US is wired into the production code path BEFORE the slice closes. |
| rmcp prompts API uncertainty | T122 reserves a 1-hour exploratory pass with findings recorded in `notes/rmcp-prompts-api.md`. |
| Test injection seam dead-code (per Phase 4 P6 lesson) | Every new `*_OVERRIDE` slot ships with a passing end-to-end test in the same PR that exercises the production code path consulting the slot. |
| Reviewer pass surfaces blockers late | Per-US reviewer pass cadence held; phase-wide Polish pass catches anything per-US missed. |
| CI flake on ubuntu MSRV | Phase 4 retro flagged 2-in-N pattern. Retry once on transient failures; do NOT use `--no-verify`. |

## Format validation

All tasks above follow the strict checklist format:
- ✓ Checkbox `- [ ]` on every line.
- ✓ Sequential ID (T001 through T433+ in blocks of 100).
- ✓ `[P]` parallelism marker where applicable.
- ✓ `[Story]` label on every user-story phase task; absent on Setup, Foundational, Polish, and `[GIT]` tasks.
- ✓ `[GIT]` label on git workflow tasks.
- ✓ File paths present in implementation tasks where applicable.
- ✓ Agent reference (`use devs:rust-dev agent`) on Rust implementation tasks.

## Total task counts

| Phase | Range | Count |
|---|---|---|
| Phase 1: Setup | T001–T009 | 9 |
| Phase 2: Foundational | T010–T041 | 32 |
| Phase 3: US1 | T100–T157 | 58 |
| Phase 4: US2 | T200–T236 | 37 |
| Phase 5: US3 | T250–T283 | 34 |
| Phase 6: US4 | T300–T336 | 37 |
| Phase 7: US5 | T350–T385 | 36 |
| Phase 8: Polish | T400–T433 (+ T415a/b/c/d) | 38 |
| **TOTAL** | T001–T433 | **~281** |

(Numbers approximate; gaps in ID allocation per block.)

## Suggested MVP scope

**Phase 3 (US1)** alone delivers the headline Phase 5 capability: plugin commands accessible as MCP prompts in harness slash menus. The substitution layer ships in stub form (no-op), so URL references in commands won't resolve; this is the cost of an early MVP. Subsequent user stories add the resolution.

For a more meaningful MVP that includes path resolution, ship Phase 3 + Phase 4 (US1 + US2) — this gives commands AND path/env substitution in entry bodies.

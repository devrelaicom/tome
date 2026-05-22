---
description: "Phase 4 implementation tasks — central architecture refactor and cross-harness integration"
---

# Tasks: Phase 4 — Central Architecture Refactor and Cross-Harness Integration

**Input**: Design documents from `/specs/004-phase-4-refactor-harnesses/`
**Prerequisites**: plan.md (✓), spec.md (✓), research.md (✓ 19 R-decisions), data-model.md (✓), contracts/ (✓ 13 files), quickstart.md (✓)
**Created**: 2026-05-22
**Tests**: This project uses TDD. Library-API tests + integration tests are part of each user story's slices.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story. The slice structure matches research §R-13 + plan §Pre-emptive slice plans.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks)
- **[Story]**: Which user story this task belongs to (US1, US2, US3, US4, US5)
- **[GIT]**: Git workflow task at a phase boundary
- All Rust implementation tasks reference the `devs:rust-dev` agent
- All file paths are repository-root-relative

## Branch & feature

- **Feature number**: `004`
- **Feature slug**: `phase-4-refactor-harnesses`
- **Branch**: `004-phase-4-refactor-harnesses` (already created and carries the spec + plan + research + data-model + 13 contracts + quickstart — see `git log`)

## ID allocation

- `T001`–`T099` Setup + Foundational (F1–F10)
- `T100`–`T199` US1 — Bind a project to a workspace
- `T200`–`T249` US2 — Workspace lifecycle
- `T250`–`T299` US3 — Layered settings + composition
- `T300`–`T349` US4 — Summarisation + RULES.md
- `T350`–`T399` US5 — Doctor extensions
- `T400`+ Polish (final phase)

Block-numbering with buffer space — `/sdd:tasks` is allowed to refine; the buffer makes "find me the US3 tasks" trivial.

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Branch hygiene, dependency wiring, deny.toml extensions, structural-test scaffolding.

### Phase Start

- [ ] T001 [GIT] Verify on `004-phase-4-refactor-harnesses` branch and working tree is clean
- [ ] T002 [GIT] Pull and rebase on origin/main; resolve any conflicts
- [ ] T003 [GIT] Confirm `cargo test` is green at HEAD (Phase 3-complete baseline; expect 399 tests across 53 suites)

### Implementation

- [ ] T004 Add `llama-cpp-2` direct dependency to `Cargo.toml` under `[dependencies]` with a minor-pinned version (per research §R-2) (use devs:rust-dev agent)
- [ ] T005 Add `toml_edit` direct dependency to `Cargo.toml` under `[dependencies]` with a minor-pinned version (per research §R-5) (use devs:rust-dev agent)
- [ ] T006 Extend the existing `serde_json` `[dependencies]` entry with `features = ["preserve_order"]` (per research §R-6) (use devs:rust-dev agent)
- [ ] T007 Extend `deny.toml` with licence rows for `llama-cpp-2`, `toml_edit`, and their direct transitives (verify each transitive falls inside the existing allowlist; `cargo deny check` must pass) (use devs:rust-dev agent)
- [ ] T008 Run `cargo build --release` and confirm stripped binary size is ≤ 50 MB; record the baseline (per research §R-4, projection ~28.4 MiB macOS arm64, ~34 MB Linux x86_64) in a comment at the top of `tests/sync_boundary.rs` (use devs:rust-dev agent)
- [ ] T009 Run `cargo test` and confirm the 399 Phase 3-complete baseline is still green after the dep additions (use devs:rust-dev agent)
- [ ] T010 [GIT] Commit: `chore(deps): add llama-cpp-2 + toml_edit; enable serde_json/preserve_order`

### Phase Completion

- [ ] T011 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T012 [GIT] Open or update PR to main with Phase 1 summary
- [ ] T013 [GIT] Verify all CI checks pass (binary-size gate + cargo-deny + clippy + test matrix)
- [ ] T014 [GIT] Report PR ready status

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Land the constitution v1.3.0 amendment, the `Paths` reshape, the new error variants, the atomic-directory helper, the dep audit sweep, the summariser skeleton, the harness skeleton, the settings skeleton, the schema v1→v2 migration registration, and the `WorkspaceName` newtype. Every user story depends on this phase.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

### Phase Start

- [ ] T015 [GIT] Verify working tree is clean before starting Phase 2
- [ ] T016 Create `specs/004-phase-4-refactor-harnesses/retro/P2.md` from the standard retro template
- [ ] T017 [GIT] Commit: `docs(retro): initialise Phase 4 / P2 retro`

### Slice F1 — Constitution v1.3.0 §Paths amendment

- [ ] T018 Edit `CONSTITUTION.md`: rewrite the `## Operational Constraints` §Paths block per research §R-12 (verbatim text); bump `Version` from `1.2.0` to `1.3.0`; bump `Last Amended` to `2026-05-22` (use devs:rust-dev agent)
- [ ] T019 Add a one-paragraph rationale block to the constitution's `## Plan history`-equivalent section (or amendment log) naming the spec section that drove the change (FR-300 through FR-303) (use devs:rust-dev agent)
- [ ] T020 Update README.md banner / Phase status to note that v1.3.0 lands as part of Phase 4 (use devs:rust-dev agent)
- [ ] T021 [GIT] Commit: `docs(constitution): v1.3.0 — rewrite §Paths Operational Constraint for <home>/.tome/ layout`

### Slice F2a — `Paths` reshape (drop XDG-separated fields; introduce `<home>/.tome/` accessors)

- [ ] T022 [P] Refactor `src/paths.rs` per data-model.md §1: rename `Paths` struct to carry only the new fields (`root`, `global_config_file`, `global_settings_file`, `index_db`, `index_lock`, `catalogs_dir`, `models_dir`, `logs_dir`, `mcp_log`, `mcp_log_prev`, `workspaces_dir`); add `paths::home_root() -> Result<PathBuf, TomeError>` resolving via raw env-var inspection (per research §R-1) (use devs:rust-dev agent)
- [ ] T023 [P] Add `Paths::workspace_dir(&WorkspaceName)`, `Paths::workspace_settings_file(&WorkspaceName)`, `Paths::workspace_rules_file(&WorkspaceName)` accessor methods (use devs:rust-dev agent)
- [ ] T024 [P] Add `Paths::project_marker_dir(&Path)`, `Paths::project_marker_config(&Path)`, `Paths::project_marker_rules(&Path)` associated functions (use devs:rust-dev agent)
- [ ] T025 Delete the Phase 3 XDG-separated fields and accessors (`state_dir`, `config_dir`, `data_dir`, `cache_dir`, `config_file_for`, `index_db_for`, `index_lock_for`, `workspace_registry`, `workspace_marker_dir`); the entire field set under data-model.md §1 "Removed in Phase 4" must be gone (use devs:rust-dev agent)
- [ ] T026 Delete `src/workspace/inventory.rs` and remove its module declaration / re-export (per research §R-11) (use devs:rust-dev agent)
- [ ] T027 Mechanical call-site sweep: every consumer of the dropped accessors compiles against the new `Paths` shape. Update `src/commands/`, `src/index/`, `src/embedding/`, `src/catalog/`, `src/doctor/`, `src/mcp/` in lockstep (use devs:rust-dev agent)
- [ ] T028 Add `tests/no_directories_imports.rs` — structural test that `grep`s `src/` for the strings `directories::` and `extern crate directories`; fails if any match found (forward-looking guard per research §R-1) (use devs:rust-dev agent)
- [ ] T028a Add `tests/no_phase3_paths.rs` (FR-304 enforcement) — structural test that `grep`s `src/` for the Phase 3 XDG-separated path identifiers (`config_dir`, `data_dir`, `state_dir`, `cache_dir`, `workspace_registry`, the `workspaces.txt` literal); fails if any survive. Documents in its module-doc that this is the FR-304 "no Phase 3 fallback" regression net (use devs:rust-dev agent)
- [ ] T029 Update `tests/paths_*.rs` (any Phase 3 path-resolution test) to assert the new `<home>/.tome/` layout per contracts/paths-and-layout-p4.md (use devs:rust-dev agent)
- [ ] T030a Read-only DB open refactor — `src/commands/status.rs` opens the central DB via `index::open_read_only` (use devs:rust-dev agent)
- [ ] T030b Read-only DB open refactor — `src/commands/query.rs` opens the central DB via `index::open_read_only` (use devs:rust-dev agent)
- [ ] T030c Read-only DB open refactor — `src/commands/catalog/list.rs` + `show.rs` open the central DB via `index::open_read_only` (use devs:rust-dev agent)
- [ ] T030d Read-only DB open refactor — `src/commands/plugin/list.rs` + `show.rs` open the central DB via `index::open_read_only` (use devs:rust-dev agent)
- [ ] T030e Read-only DB open refactor — `src/commands/doctor.rs` opens the central DB via `index::open_read_only` (use devs:rust-dev agent)
- [ ] T030f Read-only DB open refactor — `src/mcp/preflight.rs` + tools open the central DB via `index::open_read_only` (folds in the P10-deferred refactor per research §R-17 across all six command surfaces; the read-only-open ladder lives in `index::open_read_only`'s contract documented in Phase 3 F5) (use devs:rust-dev agent)
- [ ] T031 [GIT] Commit: `refactor(paths): collapse XDG-separated paths under <home>/.tome/; drop workspace inventory; read-only DB open across read paths`

### Slice F2b — `Scope` reshape (paired with F10's `WorkspaceName`; see F10)

(Note: F2b lands together with F10 because `Scope` now carries `WorkspaceName`. The split into F2a/F2b in plan §Pre-emptive slice plans is for compile-incrementality — F10 builds on F2a and replaces the Phase 3 `Scope::Global | Scope::Workspace(PathBuf)` shape; see T085–T093.)

### Slice F3 — closed-error-set extension (pre-allocate 8 new variants)

- [ ] T032 Add 8 new variants to `TomeError` in `src/error.rs` per data-model.md §14: `WorkspaceNotFound`, `WorkspaceAlreadyExists`, `WorkspaceNameInvalid`, `WorkspaceHasBoundProjects`, `CompositionError { kind: CompositionErrorKind }`, `HarnessNotSupported`, `HarnessClash`, `SummariserFailure { kind: SummariserFailureKind }` (use devs:rust-dev agent)
- [ ] T033 Add `CompositionErrorKind` enum (Cycle, WorkspaceRefOutsideProject, UnknownWorkspace, BadExclusion) per data-model.md §14 (use devs:rust-dev agent)
- [ ] T034 Add `SummariserFailureKind` enum (ModelMissing, ModelChecksumMismatch, BackendInitFailed, OutputUnparsable, OutputEmpty) + `ShortOrLong` discriminator per data-model.md §14 (use devs:rust-dev agent)
- [ ] T035 Extend `TomeError::exit_code()` exhaustive match with codes 13 / 14 / 15 / 16 / 17 / 18 / 19 / 20 per contracts/exit-codes-p4.md (use devs:rust-dev agent)
- [ ] T036 Extend `TomeError::category()` exhaustive match with the eight new category strings per contracts/exit-codes-p4.md (use devs:rust-dev agent)
- [ ] T037 Add the reused-variant match-arms documented in FR-602 (project marker malformed → reuse code 70 `WorkspaceMalformed`; rename precondition missing project dir → reuse 70; per-user state dir unwritable → reuse 7 `Io`); these are not new variants, they widen the semantic scope of existing Phase 3 variants — document the wider scope in the doc-comments on each variant (use devs:rust-dev agent)
- [ ] T038 Extend `tests/exit_codes.rs::build_each_variant` and the exhaustive `_code_for` match-arm to cover the 8 new variants (use devs:rust-dev agent)
- [ ] T039 Extend `tests/error_messages.rs` with one Display assertion per new variant per contracts/exit-codes-p4.md §Display messages (use devs:rust-dev agent)
- [ ] T040 [GIT] Commit: `feat(error): add Phase 4 TomeError variants and exit codes 13–20`

### Slice F4 — atomic-populated-directory helper

- [ ] T041 Create `src/util/` directory with `mod.rs` re-exporting `atomic_dir` (use devs:rust-dev agent)
- [ ] T042 Create `src/util/atomic_dir.rs` implementing `land_directory<F>` and `land_directory_with_replace<F>` per data-model.md §16 and research §R-10 (use devs:rust-dev agent)
- [ ] T043 Promote `lib.rs` to re-export `util::atomic_dir::{land_directory, land_directory_with_replace}` (use devs:rust-dev agent)
- [ ] T044 Refactor `src/workspace/init.rs` (Phase 3) to use the new helper; the existing inline atomic-directory pattern is deleted (use devs:rust-dev agent)
- [ ] T045 Create `tests/atomic_dir.rs` covering: happy path; SIGINT mid-populate (staged temp dir cleaned by `TempDir::drop`); SIGINT after `keep()` and before rename (orphaned staged dir picked up by doctor `--fix`); replace rollback on rename failure; 0700 mode on Unix; documented `.tome.tmp.*` prefix (use devs:rust-dev agent)
- [ ] T046 [GIT] Commit: `refactor(util): promote atomic-populated-directory helper to src/util/atomic_dir.rs`

### Slice F5 — `toml_edit` + `serde_json/preserve_order` sweep audit

- [ ] T047 Audit pass: enable `serde_json/preserve_order` is project-wide (every `serde_json::Value::Object` now uses `IndexMap` not `BTreeMap`). Grep `src/` for any code that relies on alphabetical key ordering; refactor any such code to be order-insensitive (per research §R-6) (use devs:rust-dev agent)
- [ ] T048 Verify `toml_edit` is wired only for harness MCP config read-modify-write paths; Tome-owned TOML files (`settings.toml`, project marker `config.toml`, global `config.toml`, manifests) continue to use the `toml` crate via `serde` (per research §R-5) (use devs:rust-dev agent)
- [ ] T049 Document the strict-vs-lenient boundary inline in `src/harness/mcp_config.rs` (placeholder doc-comment at this slice — file is created in F7 but the audit needs to capture the rule now) (use devs:rust-dev agent)
- [ ] T050 [GIT] Commit: `chore(deps): audit serde_json/preserve_order + toml_edit scope`

### Slice F6 — summariser skeleton

- [ ] T051 Create `src/summarise/mod.rs` exposing the module surface: `Summariser` trait + `backend()` singleton (use devs:rust-dev agent)
- [ ] T052 [P] Create `src/summarise/llama.rs` with `LlamaSummariser` struct + skeleton `Summariser` impl per data-model.md §13 (production wiring lands in US4.a; here the impl returns a `SummariserFailure::BackendInitFailed`-shaped error to enforce that it isn't reachable yet) (use devs:rust-dev agent)
- [ ] T053 [P] Create `src/summarise/stub.rs` with `#[cfg(test)] StubSummariser` per data-model.md §13 + research §R-14 (use devs:rust-dev agent)
- [ ] T054 [P] Create `src/summarise/prompts.rs` with `SHORT_PROMPT` and `LONG_PROMPT` `&'static str` constants per research §R-15 + `SHORT_MAX_CHARS = 800`, `SHORT_TARGET_RANGE = 400..=800`, `LONG_MAX_CHARS = 2400`, `LONG_TARGET_RANGE = 1500..=2500` constants (use devs:rust-dev agent)
- [ ] T055 [P] Create `src/summarise/registry.rs` extending the `MODEL_REGISTRY` with the third entry: `qwen2.5-0.5b-instruct` (URL pinned to Hugging Face, SHA-256 pinned, ~400 MB Q4_K_M) (use devs:rust-dev agent)
- [ ] T056 [P] Create `src/summarise/download.rs` reusing `embedding::download` with the summariser model entry (use devs:rust-dev agent)
- [ ] T057 Add `backend() -> Result<&'static LlamaBackend, TomeError>` using `std::sync::OnceLock<LlamaBackend>` per data-model.md §13 + research §R-2 (use devs:rust-dev agent)
- [ ] T058 Refactor `embedding::download::download_model` to accept an optional `byte_progress: Option<&dyn Fn(u64, u64)>` callback (folds in the TD-010 / P10-deferred byte-progress callback per research §R-17); each existing caller passes `None` to preserve behaviour (use devs:rust-dev agent)
- [ ] T059 Rename the test fabricator `fabricate_installed_model` / `fabricate_all_installed_models` → `fabricate_installed_models` / similar plural rename per research §R-17 (P6-deferred); update every caller (use devs:rust-dev agent)
- [ ] T060 Verify `tests/sync_boundary.rs` passes — `src/summarise/` contains no `tokio::` / `async fn` / `.await` (per research §R-11; no exemption added) (use devs:rust-dev agent)
- [ ] T061 Create `tests/summariser_stub.rs` covering: `StubSummariser::new` + deterministic short/long output + call-count assertions (use devs:rust-dev agent)
- [ ] T062 [GIT] Commit: `feat(summarise): summariser skeleton (Summariser trait, LlamaBackend singleton, StubSummariser, prompts)`

### Slice F7 — harness skeleton

- [ ] T063 Create `src/harness/mod.rs` exposing the module surface: `HarnessModule` trait + `SUPPORTED_HARNESSES` static + `lookup(name) -> Option<&'static dyn HarnessModule>` per data-model.md §10 (use devs:rust-dev agent)
- [ ] T064 Add `RulesFileStrategy` enum (`BlockInExistingFile | StandaloneFile`), `BlockBodyStyle` enum (`AtInclude | Inline`), `McpConfigFormat` enum (`Json | Toml`), `MCP_CONFIG_KEY: &str = "tome"` static (use devs:rust-dev agent)
- [ ] T065 Add `mcp_parent_key(&self) -> &'static str` to the `HarnessModule` trait per data-model.md §10 (reviewer M6 fold-in — required to distinguish `"mcpServers"` from `"mcp_servers"` in TOML harnesses like Codex) (use devs:rust-dev agent)
- [ ] T066 Create `src/harness/rules_file.rs` (empty skeleton — `parse_block`, `write_block`, `remove_block`, `write_standalone`, `remove_standalone` function stubs returning `unimplemented!()`) per contracts/rules-file-integration.md (use devs:rust-dev agent)
- [ ] T067 Create `src/harness/mcp_config.rs` (empty skeleton — `read_entry`, `write_entry`, `remove_entry`, ownership-marker predicate `is_tome_owned` stubs returning `unimplemented!()`) per contracts/mcp-config-integration.md (use devs:rust-dev agent)
- [ ] T068 Create the five harness module files as empty stubs that compile but return `unimplemented!()`: `src/harness/claude_code.rs`, `src/harness/codex.rs`, `src/harness/gemini.rs`, `src/harness/cursor.rs`, `src/harness/opencode.rs` (use devs:rust-dev agent)
- [ ] T069 Wire `SUPPORTED_HARNESSES` to include all five stub impls (in lexicographic order of `name()`) so `tome harness` (bare) lists them once US3.c wires the command (use devs:rust-dev agent)
- [ ] T070 Add `#[doc(hidden)] pub static HARNESS_MODULES_OVERRIDE: std::sync::RwLock<Option<Vec<Box<dyn HarnessModule>>>>` test-injection hook (per CLAUDE.md "Test injection via `#[doc(hidden)] pub static`" pattern + project convention of std-over-parking_lot — std `OnceLock` + `RwLock` covers it). Document the test-only intent in the doc-comment. Pair with a `HarnessModulesGuard` RAII struct (in the consuming test file) whose `Drop` clears the slot per CLAUDE.md "RAII guards for thread-local injection" pattern (use devs:rust-dev agent)
- [ ] T071 [GIT] Commit: `feat(harness): harness skeleton (HarnessModule trait, 5 stub impls, rules_file + mcp_config stubs)`

### Slice F8 — settings skeleton (parser + composition resolver + cycle detection)

- [ ] T072 Create `src/settings/mod.rs` exposing `WorkspaceSettings`, `ProjectMarkerConfig`, `GlobalSettings` types per data-model.md §6/§7/§8 with `#[serde(deny_unknown_fields)]` (use devs:rust-dev agent)
- [ ] T073 [P] Create `src/settings/parser.rs` parsing the three settings shapes (workspace / project marker / global) with strict deserialise (use devs:rust-dev agent)
- [ ] T074 [P] Create `src/settings/composition.rs` with `CompositionRef` enum (Include, Exclude, CurrentWorkspace, NamedWorkspace, Global) + parse-from-string ladder per research §R-9 (use devs:rust-dev agent)
- [ ] T075 [P] Create `src/settings/resolver.rs` implementing `resolve_effective_list(project_marker, bound_workspace, global_settings, central_db) -> Result<EffectiveHarnessList, CompositionError>` per data-model.md §9 + contracts/settings-composition.md; DFS visited-set cycle detection; FR-449 invariant enforced (composition refs resolve to directly-declared lists, NOT effective lists) (use devs:rust-dev agent)
- [ ] T076 Add `EffectiveHarnessList`, `EffectiveHarness`, `ScopeKind` types per data-model.md §9 (use devs:rust-dev agent)
- [ ] T077 Add a `StubScope` test fixture in `src/settings/resolver.rs` (or a sibling `#[cfg(test)] testkit.rs`) that takes hand-rolled scope contents without requiring on-disk files (use devs:rust-dev agent)
- [ ] T078 Add `tests/settings_skeleton.rs` covering: `CompositionRef::parse("[workspace]")` returns `CurrentWorkspace`; `parse("[global]")` → `Global`; `parse("[workspaces.foo]")` → `NamedWorkspace("foo")`; `parse("!bar")` → `Exclude("bar")`; `parse("![global]")` → `BadExclusion` (per FR-448) (use devs:rust-dev agent)
- [ ] T079 [GIT] Commit: `feat(settings): layered settings parser + composition resolver skeleton`

### Slice F9 — schema v1→v2 migration registration

- [ ] T080 Refactor `src/index/schema.rs::bootstrap` to emit schema v2 directly: create `workspaces`, `skills` (without the `enabled` column), `skill_embeddings` (sqlite-vec), `workspace_skills`, `workspace_catalogs`, `workspace_projects` tables + indices per data-model.md §4 (use devs:rust-dev agent)
- [ ] T081 Insert the seeded `global` workspace row inside the bootstrap transaction with `created_at = now`, `last_used_at = now` (FR-323) (use devs:rust-dev agent)
- [ ] T082 Register the first production migration in `src/index/migrations.rs::MIGRATIONS`: a named `fn phase_4_v1_to_v2(tx: &Transaction) -> Result<(), TomeError>` (NOT a closure) per research §R-7 (use devs:rust-dev agent)
- [ ] T083 Implement the v1→v2 migration body: (1) create `workspaces`, `workspace_skills`, `workspace_catalogs`, `workspace_projects` tables + indices; (2) seed the `global` workspace row; (3) rebuild the `skills` table via the SQLite 12-step pattern to drop the `enabled` column (`PRAGMA foreign_keys = OFF` → `CREATE TABLE skills_new` (without `enabled`) → `INSERT INTO skills_new SELECT * FROM skills` (omitting `enabled`) → recreate every non-`enabled` index referenced in `src/index/schema.rs` → `DROP TABLE skills` → `ALTER TABLE skills_new RENAME TO skills` → `PRAGMA foreign_keys = ON`); audit `src/index/schema.rs` for every non-`enabled` index on `skills` before this PR lands (reviewer M-MIG-2 fold-in) (use devs:rust-dev agent)
- [ ] T084 Update `src/index/meta.rs` to record `summariser_name` and `summariser_version` rows during bootstrap + drift detection (per data-model.md §4 `meta` table requirements) (use devs:rust-dev agent)
- [ ] T085 Delete the Phase 3 synthetic `SuggestedFix` injection from `tests/doctor.rs::fix_runs_forward_schema_migration_end_to_end` (per research §R-17 / P7-deferred) — `doctor::build_suggested_fixes` now emits `subsystem: "schema"` naturally when a v1 DB is opened against a Phase 4 binary (use devs:rust-dev agent)
- [ ] T086 Create `tests/migration_v1_to_v2.rs` covering: synthetic v1 fixture (built via the existing `write_index_db_with_schema_version(path, 1)` helper) is migrated end-to-end; post-migration `meta.schema_version = 2`; new tables exist; seeded `global` workspace present; `skills.enabled` column is absent (`SELECT enabled FROM skills` fails with `no such column`); index rebuild preserved every non-`enabled` index; FK check passes (`PRAGMA foreign_key_check`); rollback on injected SQL failure leaves schema at v1 (per contracts/schema-migration-p4.md §Testing strategy) (use devs:rust-dev agent)
- [ ] T087 [GIT] Commit: `feat(migrations): register phase_4_v1_to_v2; bootstrap emits v2 directly`

### Slice F10 — `WorkspaceName` newtype + `Scope` reshape + `workspace_projects` PK

- [ ] T088 Create `src/workspace/name.rs` with the `WorkspaceName` newtype per data-model.md §2; `parse(s: &str) -> Result<Self, TomeError>` validates against FR-347 (charset, length 1..=64, no leading/trailing `-` or `_`, not `.`/`..`/empty); `is_reserved()` returns true for `"global"`; `const GLOBAL: &'static str = "global"` constant (use devs:rust-dev agent)
- [ ] T089 Implement `Serialize` / `Deserialize` for `WorkspaceName` calling `parse()` on deserialise so every TOML / env-var / flag input gets the validation rule (FR-347) (use devs:rust-dev agent)
- [ ] T090 Reshape `src/workspace/scope.rs` per data-model.md §3: `Scope(pub WorkspaceName)`; `ScopeSource { Flag, Env, ProjectMarker, GlobalFallback }`; `ResolvedScope { scope, source, project_root: Option<PathBuf> }`. Delete the Phase 3 `Scope::Global | Scope::Workspace(PathBuf)` shape (use devs:rust-dev agent)
- [ ] T091 Rewrite `src/workspace/resolution.rs::resolve(...) -> Result<ResolvedScope, TomeError>` per data-model.md §3 resolution algorithm: flag > env > marker walk > `global` fallback; central DB membership check on every name; `WorkspaceNotFound` (code 13) on missing; `WorkspaceMalformed` (code 70) on bad marker (use devs:rust-dev agent)
- [ ] T092 Mechanical call-site sweep: every `Scope::Workspace(PathBuf)` and `Scope::Global` consumer is updated to match against the new `Scope(WorkspaceName)` shape (use devs:rust-dev agent)
- [ ] T093 Delete the Phase 3 `--global` top-level flag from `src/cli.rs::GlobalScopeArgs` (FR-345). Keep `--workspace <name>` accepting a `WorkspaceName`. Update help text. The `TOME_WORKSPACE` env var continues to be accepted (use devs:rust-dev agent)
- [ ] T094 Ensure the `workspace_projects` table DDL in `src/index/schema.rs` declares `project_path TEXT PRIMARY KEY NOT NULL` (PK on path alone, NOT a composite PK; enforces FR-322 / FR-342's 1:1 binding at the database layer) (use devs:rust-dev agent)
- [ ] T095 Add `tests/workspace_name.rs` covering: valid names (`a`, `Foo`, `foo-bar_baz`); invalid (empty, `.`, `..`, leading `-`, trailing `_`, 65 chars, char outside `[a-zA-Z0-9_-]`); reserved (`global` rejected by init via `is_reserved()`) (use devs:rust-dev agent)
- [ ] T096 Update `tests/workspace_resolution.rs` to assert the new flag-precedence + `WorkspaceNotFound` mapping per data-model.md §3 (use devs:rust-dev agent)
- [ ] T097 [GIT] Commit: `feat(workspace): WorkspaceName newtype + Scope reshape + workspace_projects PK on project_path`

### Slice F11 — Phase 4 catalog/plugin command refactor (workspace_catalogs + workspace_skills) [B1+B2 reviewer fold-in]

Phase 4's central architecture moves catalog enrolment to the `workspace_catalogs` junction and plugin enablement to the `workspace_skills` junction (FR-360 through FR-367 + FR-380 through FR-385). Every Phase 1/2/3 catalog and plugin command body must rewire onto these junctions; this slice keeps the surface stable while the storage moves underneath. The cheap-re-enable invariant (FR-006 carry-forward + FR-383 retention rule) is preserved. The refcount-under-lock invariant (FR-366 / FR-367) replaces Phase 3's per-scope refcount via opt-in `workspaces.txt`.

- [ ] T098a Rewire `src/commands/catalog/add.rs` per FR-362: operate against the resolved workspace; INSERT a `workspace_catalogs` row for `(workspace_id, catalog_name)`; clone the URL to `<root>/catalogs/<url-hash>/` only if no other workspace already enrols the URL (refcount probe); otherwise reuse the existing clone. Drops the Phase 3 per-scope flow (use devs:rust-dev agent)
- [ ] T098b Rewire `src/commands/catalog/remove.rs` per FR-363 + FR-366: under the advisory lockfile, DELETE the `workspace_catalogs` row for `(workspace_id, catalog_name)`; check refcount (`SELECT count(*) FROM workspace_catalogs WHERE url = ?`); if zero, `remove_dir_all` the on-disk clone. The Phase 2 refusal on enabled plugins in the same workspace continues to apply; `--force` cascades disable. The check + delete + cache cleanup all run inside the same advisory-lock critical section per FR-366 (use devs:rust-dev agent)
- [ ] T098c Rewire `src/commands/catalog/list.rs` per FR-364: report ONLY the resolved workspace's enrolled catalogs; other workspaces' enrolments MUST NOT appear in the output. Joins `workspace_catalogs` (use devs:rust-dev agent)
- [ ] T098d Rewire `src/commands/catalog/update.rs` per FR-365: refresh every catalog that ANY workspace enrols (`SELECT DISTINCT url FROM workspace_catalogs`); reindex pass triggered by an update covers every workspace's enabled plugins for any updated catalog, not just the resolved workspace's (use devs:rust-dev agent)
- [ ] T098e Rewire `src/commands/catalog/show.rs` to read from `workspace_catalogs` (surface unchanged; FR-364 implies show is also workspace-scoped) (use devs:rust-dev agent)
- [ ] T098f Rewire `src/commands/plugin/enable.rs` per FR-380: UPSERT one `skills` row per parsed skill keyed by `(catalog, plugin, skill name)` (workspace-agnostic, re-embedding only on content_hash change); UPSERT one `workspace_skills` row per skill keyed on `(workspace_id, skill_id)`; trigger summary regeneration for the resolved workspace (deferred to US4.b); trigger integration-sync if Tome is operating in a bound project context (use devs:rust-dev agent)
- [ ] T098g Rewire `src/commands/plugin/disable.rs` per FR-381: DELETE the workspace_skills rows for every skill in the plugin within the resolved workspace; the shared `skills` rows are RETAINED (another workspace may still enable them; FR-383); trigger summary regen + integration sync (use devs:rust-dev agent)
- [ ] T098h Rewire `src/commands/plugin/reindex.rs` per FR-382: update each affected `skills` row in place against the new central DB; retain `workspace_skills` rows when present; trigger summary regeneration if any skill's content_hash changed in the resolved workspace's enabled set (use devs:rust-dev agent)
- [ ] T098i Rewire `src/commands/plugin/list.rs` and `src/commands/plugin/show.rs` to read through `workspace_skills` JOIN `skills` scoped to the resolved workspace (use devs:rust-dev agent)
- [ ] T098j Rewire `src/commands/plugin/interactive.rs` (the Phase 2 three-level catalog → plugin → action loop) to honour the resolved workspace's workspace_skills throughout (use devs:rust-dev agent)
- [ ] T098k Implement the cheap-re-enable invariant per FR-006 + FR-383: when `tome plugin enable` is invoked against a plugin whose `(catalog, plugin, skill name)` skills rows already exist with matching content_hash, the embedder closure is NOT invoked — only the `workspace_skills` rows are UPSERTed. Track via `StubEmbedder::call_count() == 0` in the corresponding test (use devs:rust-dev agent)
- [ ] T098l Implement FR-385 forward-progress at the workspace_skills layer (paired with US4.b's summariser-side wire): the workspace_skills row INSERT/DELETE commits in its own transaction BEFORE the summariser is invoked; downstream summariser failure does not roll back the skill-state mutation (use devs:rust-dev agent)
- [ ] T098m Implement FR-325 read-path semantics under the new central DB: readers opened against schema 1 while a writer holds the lock to apply the v1→v2 migration EITHER (a) complete against schema 1 (migration's commit has not landed under WAL) OR (b) fail with `SchemaVersionTooNew` (code 73) once the commit lands. A reader MUST NEVER observe a half-migrated schema mid-statement. Pin with a test in `tests/migration_v1_to_v2.rs` (use devs:rust-dev agent)
- [ ] T098n Verify FR-348 strictness boundary application (per reviewer B3): grep `src/` for every Tome-owned input parse site — global config.toml (Phase 1), workspace settings.toml (Phase 4 / F8), project marker config.toml (Phase 4 / F8), global settings.toml (Phase 4 / F8), model registry manifest (Phase 2). Confirm each carries `#[serde(deny_unknown_fields)]`. Extend `tests/manifest_strictness.rs` (the existing Phase 1 / Phase 2 structural grep test) to cover the three new Phase 4 types in addition to the existing two — folds in B3 + P10 retro's `ModelManifest` strictness grep guard (use devs:rust-dev agent)
- [ ] T098o Add `tests/catalog_workspace_refcount.rs` covering FR-361 / FR-366 / FR-367: two workspaces enrol the same URL → one clone on disk; one removes → clone remains (other workspace still refs); other removes → clone removed; concurrent remove from two workspaces serialised via the advisory lock → no cache leak, no use-after-cleanup (use devs:rust-dev agent)
- [ ] T098p Add `tests/plugin_workspace_skills.rs` covering FR-380 / FR-381 / FR-383: enable in workspace A does not affect workspace B's view; disable in A does not delete the shared `skills` row that B still references; A and B running concurrent enables for the same plugin both succeed; skills row retention assertion (orphan skills rows kept indefinitely) (use devs:rust-dev agent)
- [ ] T098q Add `tests/plugin_cheap_reenable.rs` covering FR-006 carry-forward: re-enabling a plugin with no content-hash change invokes `StubEmbedder::call_count() == 0`; the workspace_skills UPSERT happens; the existing `skills` rows are not re-touched (use devs:rust-dev agent)
- [ ] T098r Add `tests/plugin_summariser_forward_progress.rs` covering FR-385 at the plugin layer (distinct from `tests/summariser_forward_progress.rs` in US4.b — this test asserts the workspace_skills commit ordering): on `plugin enable`, the workspace_skills row is visible in a follow-up read even when the subsequent summariser invocation returns `SummariserFailure` (use devs:rust-dev agent)
- [ ] T098s Add `tests/catalog_update_cross_workspace_reindex.rs` covering FR-365: `catalog update` against a catalog enrolled by workspaces A + B reindexes the enabled plugins in BOTH workspaces (not just the resolved one) (use devs:rust-dev agent)
- [ ] T098t [GIT] Commit: `refactor(catalog,plugin): rewire Phase 4 commands onto workspace_catalogs + workspace_skills junctions`

### End-of-phase

- [ ] T098 Run `/sdd:map incremental` to refresh codebase docs against Foundational changes — expect changes to STACK, ARCHITECTURE, STRUCTURE, ERRORS at minimum (use devs:rust-dev agent)
- [ ] T099 Review `retro/P2.md` and extract critical learnings to CLAUDE.md (conservative — universal patterns only) (use devs:rust-dev agent)

### Phase Completion

- [ ] T100 [GIT] Commit: `docs(codebase): refresh after Phase 4 / P2 Foundational`
- [ ] T101 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T102 [GIT] Update PR with Foundational summary (10 slices F1–F10 landed; ~50 KB of mechanical refactor + new modules)
- [ ] T103 [GIT] Verify all CI checks pass (clippy, deny, tests, binary size)
- [ ] T104 [GIT] Report PR ready status

**Checkpoint**: Foundational complete — constitution amended, paths reshaped, error variants pre-allocated, helper promoted, deps wired, summariser/harness/settings skeletons in place, schema v2 migration registered, `WorkspaceName` shipped. Every user story can now begin.

---

## Phase 3: User Story 1 — Bind a project to a workspace (Priority: P1) 🎯 MVP

**Goal**: One command (`tome workspace use <name>`) from inside a project binds the project to a workspace, drops a rules file into the project's `.tome/` marker, writes the Tome MCP server entry into each configured harness's MCP config, and writes the Tome rules-file block into each configured harness's rules-file target. This is the headline Phase 4 deliverable.

**Independent Test**: From a fresh install with at least one supported harness present, the developer creates a named workspace, enrols a catalog into it, enables a plugin, then runs `tome workspace use my-workspace` in a project directory. The project's `.tome/` marker is created; the configured harness's MCP config gains a Tome entry naming the workspace; the harness's rules-file target gains a Tome block (or standalone file for Cursor). Re-running is a no-op. Rebinding to a different workspace UPSERTs cleanly.

**Slice plan** per research §R-13:
- US1.a — `tome workspace use <name>` core (binding only; harness sync stubbed)
- US1.b — Harness sync inside the bind command (StubHarness end-to-end)
- US1.c — First production harness module (`claude_code`) — bind writes real rules + MCP entries
- US1.d — Cross-product tests + closeout + retro

### Phase Start

- [ ] T110 [GIT] Verify working tree is clean before starting Phase 3 / US1
- [ ] T111 [US1] Create `retro/P3.md` from the standard retro template
- [ ] T112 [GIT] Commit: `docs(retro): initialise Phase 4 / P3 retro`

### Slice US1.a — `tome workspace use <name>` core (binding only; harness sync stubbed)

- [ ] T113 [US1] Add `Command::Workspace(WorkspaceArgs)` variant to `src/cli.rs` (Phase 3's `WorkspaceArgs` already carries `info`; extend with `Use(WorkspaceUseArgs { name: WorkspaceName, force: bool })` per data-model.md §17) (use devs:rust-dev agent)
- [ ] T114 [US1] Create `src/commands/workspace/use_.rs` (filename suffixed `_` because `use` is reserved) wiring `WorkspaceCommand::Use` dispatch (use devs:rust-dev agent)
- [ ] T115 [US1] Create `src/workspace/binding.rs` carrying the binding flow: `bind_project(target_root, name, force, deps) -> Result<BindOutcome, TomeError>` (use devs:rust-dev agent)
- [ ] T116 [US1] Implement the bind algorithm per FR-403: refuse if cwd canonicalises to `<home>` or `/` (per research §R-16); refuse if `<name>` is not in the central registry (code 13); under the advisory lockfile (one critical section for the whole bind), create the project marker directory using `util::atomic_dir::land_directory`; write `<project>/.tome/config.toml` with `workspace = "<name>"`; copy `<root>/workspaces/<name>/RULES.md` to `<project>/.tome/RULES.md`; UPSERT the `workspace_projects` row keyed on `project_path` (FR-322 / FR-342); update `last_used_at` on the workspaces row (use devs:rust-dev agent)
- [ ] T117 [US1] In US1.a only: stub the harness-sync step inside the bind command — call into a placeholder `commands::harness::sync_for_project_root(root, deps)` returning `Ok(SyncOutcome::default())`. Real sync wiring lands in US1.b (use devs:rust-dev agent)
- [ ] T118 [US1] Add `BindOutcome` struct (`workspace`, `project_root`, `created_marker: bool`, `rebind_from: Option<WorkspaceName>`, `sync: Option<SyncOutcome>`) serialisable for `--json` (use devs:rust-dev agent)
- [ ] T119 [US1] Wire the cwd refusal: extract a helper `is_project_root_acceptable(cwd: &Path, home: &Path) -> Result<(), TomeError>` that returns `Usage` (code 2) when the canonical cwd equals canonical `<home>` or `/` (per research §R-16) (use devs:rust-dev agent)
- [ ] T120 [US1] Wire `--force` to bypass the cwd guard (developer override for unusual project roots) — research §R-16 acknowledges the case is rare; the flag is in data-model.md §17 (use devs:rust-dev agent)
- [ ] T121 [US1] Wire idempotence: re-running with the same name is a no-op except that the rules file copy is refreshed from the workspace's central RULES.md (FR-403 second-to-last sentence) (use devs:rust-dev agent)
- [ ] T122 [US1] Wire rebind: re-running with a different name UPSERTs the `workspace_projects` row (the PK on `project_path` makes this atomic per FR-342); `BindOutcome.rebind_from` carries the prior workspace name; the project rules file is replaced with the new workspace's content (use devs:rust-dev agent)
- [ ] T123 [US1] Update `last_used_at` on the resolved workspace inside the bind transaction (FR-411 — write-path command updates timestamp) (use devs:rust-dev agent)
- [ ] T124 [US1] Add `tests/workspace_use_binding.rs` covering: cwd is home → exit 2; cwd is `/` → exit 2; nonexistent workspace → exit 13; happy-path bind creates marker + DB row; idempotent re-bind refreshes rules copy without changing DB row; rebind to a different workspace updates DB + replaces marker contents; concurrent bind from two terminals via `std::sync::Barrier::new(2)` (last commit wins; PK guarantees no two rows). Library-API tests via the `bind_project` entry point; CLI binary tests cover only the cwd refusal + exit codes (use devs:rust-dev agent)
- [ ] T125 [US1] Verify FR-411 `last_used_at` update via a test in `tests/workspace_use_binding.rs::last_used_at_advances_on_bind` (use devs:rust-dev agent)
- [ ] T126 [GIT] Commit: `feat(workspace): tome workspace use <name> — core binding flow`

### Slice US1.b — harness sync inside the bind command (StubHarness end-to-end)

- [ ] T127 [US1] Create `src/harness/sync.rs` implementing `sync_for_project_root(root: &Path, deps: &SyncDeps) -> Result<SyncOutcome, TomeError>` per contracts/sync-algorithm.md; uses `settings::resolver::resolve_effective_list` to compute the effective list `L`; for each harness in `L`, calls the harness module's contract methods (use devs:rust-dev agent)
- [ ] T128 [US1] Wire the two-phase concurrency model per contracts/sync-algorithm.md: Phase A (DB read of effective list + workspace identity) is brief and runs under the advisory lock; Phase B (filesystem I/O against harness rules files + MCP config files) runs unlocked — long file I/O cannot block other writers (use devs:rust-dev agent)
- [ ] T129 [US1] Inside `harness::sync`, dispatch each harness through the still-stub `HarnessModule` trait methods. The five harness modules are wired in F7; this slice exercises the dispatch path against `unimplemented!()` panics for all five. A `#[cfg(test)] StubHarness` fixture replaces the five with one deterministic stub via the `HARNESS_MODULES_OVERRIDE` injection point from T070 (use devs:rust-dev agent)
- [ ] T130 [US1] Create `src/harness/StubHarness` (test-only) implementing `HarnessModule` deterministically: name `"stub"`, rules target `<project>/STUB_RULES.md`, MCP config `<project>/stub.mcp.json`, strategy `BlockInExistingFile`, body style `Inline`, format `Json`, parent key `"mcpServers"` (use devs:rust-dev agent)
- [ ] T131 [US1] Implement `harness::rules_file` body for `BlockInExistingFile` + `Inline`: `parse_block(content)`, `write_block(content, body)`, `remove_block(content)` per contracts/rules-file-integration.md and FR-480; markers are exactly `<!-- tome:begin -->` / `<!-- tome:end -->` each on its own line followed by exactly one `\n` (use devs:rust-dev agent)
- [ ] T132 [US1] Implement `harness::rules_file` body for `BlockInExistingFile` + `AtInclude`: same `write_block` shape but body is `@<relative-path-from-rules-file-to-project-marker-rules-file>` via `pathdiff::diff_paths` (if needed; otherwise std::path::Path::strip_prefix) (use devs:rust-dev agent)
- [ ] T133 [US1] Implement `harness::rules_file` body for `StandaloneFile`: `write_standalone(path, body)`, `remove_standalone(path)` per FR-481; no markers; entire file Tome-owned (use devs:rust-dev agent)
- [ ] T134 [US1] Implement `harness::rules_file` symlink refusal: if the target path is itself a symlink, refuse with `TomeError::Io` quoting the path (security hardening carry-over from Phase 3 PR-F) (use devs:rust-dev agent)
- [ ] T135 [US1] Implement multi-harness shared-rules-file dedup per FR-482: if more than one harness in the effective list resolves to the same rules-file path, write one Tome block at that path (use devs:rust-dev agent)
- [ ] T136 [US1] Implement multi-harness shared-MCP-config dedup per FR-482's MCP-config equivalent: if more than one harness's `mcp_config_path` resolves to the same path, write one `"tome"` entry (use devs:rust-dev agent)
- [ ] T137 [US1] Implement `harness::mcp_config` JSON branch using `serde_json` with `preserve_order` per contracts/mcp-config-integration.md: `read_entry(file, parent_key)`, `write_entry(file, parent_key, key, entry)`, `remove_entry(file, parent_key, key)`, `is_tome_owned(entry)`; preserve `env` on rewrite per FR-503; preserve every developer-authored entry, key, comment, and order (use devs:rust-dev agent)
- [ ] T138 [US1] Implement `harness::mcp_config` TOML branch using `toml_edit::Document` per contracts/mcp-config-integration.md: same operations with TOML semantics; preserve comments, key order, and inline-vs-standard-table choice; cover the Codex CLI `mcp_servers` table convention (use devs:rust-dev agent)
- [ ] T139 [US1] Implement the ownership marker per FR-501: an entry is Tome-owned iff `command == "tome"` AND `args[0] == "mcp"`; any other entry with the key `"tome"` is user-owned and refuses rewrite without `--force` (use devs:rust-dev agent)
- [ ] T140 [US1] Implement parent-directory creation per FR-505 using `util::atomic_dir` discipline — when the MCP config file's parent directory does not exist, create it before writing (use devs:rust-dev agent)
- [ ] T141 [US1] Implement the sync FR-543 cleanup pass: walk every supported harness's rules-file target for the current project; if a target file contains a Tome block (or matches the Cursor standalone path) but the harness is NOT in the effective list, remove the block (or delete the standalone file); preserve surrounding content (use devs:rust-dev agent)
- [ ] T142 [US1] Implement the sync FR-545 cleanup pass: walk every supported harness's MCP config file; if it contains a Tome-owned `"tome"` entry but the harness is NOT in the effective list, remove the entry; user-owned entries are left untouched (use devs:rust-dev agent)
- [ ] T143 [US1] Implement the sync change summary per FR-547: `SyncOutcome { added: Vec<SyncChange>, removed: Vec<SyncChange>, leave_alones: usize, decisions: Vec<HarnessDecision> }` with the structured-form decisions array (use devs:rust-dev agent)
- [ ] T144 [US1] Wire the harness-clash error (FR-502 / code 19): when the bind / sync command would overwrite a user-owned `"tome"` entry, refuse with `HarnessClash { path, command, first_arg }`; `--force` rewrites; FR-403 forward-progress rule — the binding INSERT remains committed even on harness-clash (developer can re-run sync after addressing the clash) (use devs:rust-dev agent)
- [ ] T145 [US1] Add `tests/harness_sync_stub.rs` covering the dispatch path against `StubHarness` (uses `HARNESS_MODULES_OVERRIDE`): bind writes the stub rules block + stub MCP entry; rebind rewrites both; harness-clash surfaces exit 19 + binding remains committed; `--force` overrides the clash (use devs:rust-dev agent)
- [ ] T146 [US1] Add `tests/mcp_config_create.rs` covering MCP entry creation: empty file → scaffold with single Tome entry; existing file with other entries → insert Tome at canonical position; TOML config with comments → comments preserved (uses `StubHarness` TOML variant for unit isolation) (use devs:rust-dev agent)
- [ ] T147 [US1] Add `tests/mcp_config_update.rs` covering MCP entry update: workspace rebind rewrites `args`; `env` field preserved per FR-503 (use devs:rust-dev agent)
- [ ] T148 [US1] Add `tests/mcp_config_clash.rs` covering FR-501 / FR-502: user-owned entry (different command or different first arg) refuses with exit 19; `--force` rewrites (use devs:rust-dev agent)
- [ ] T149 [US1] Add `tests/mcp_config_remove.rs` covering FR-545: removing a harness from the effective list deletes the Tome entry; user-owned `"tome"` entries are left untouched (use devs:rust-dev agent)
- [ ] T150 [US1] Add `tests/mcp_config_preserve_order.rs` covering serde_json `preserve_order`: three-entry config with Tome in the middle — insertion preserves order; rewriting Tome preserves surrounding entry order (use devs:rust-dev agent)
- [ ] T151 [US1] Add `tests/rules_file_block_in_existing.rs` covering block insertion / update / removal; AtInclude + Inline body styles; multi-harness shared file (FR-482); surrounding content preservation (FR-484); symlink refusal (T134); marker tolerance for trailing whitespace (FR-480 regex) (use devs:rust-dev agent)
- [ ] T152 [US1] Add `tests/rules_file_standalone.rs` covering Cursor's standalone-file strategy: creation, removal, no-marker handling (use devs:rust-dev agent)
- [ ] T152a [US1] Add `tests/sync_algorithm.rs` (per contracts/sync-algorithm.md) walking each numbered step of FR-540 → FR-547 against a `StubHarness` matrix: (FR-540) effective list computation; (FR-541) per-harness contract consultation; (FR-542) rules-file ensure-current; (FR-543) cleanup of rules-file targets no longer in the effective list (multi-harness scenario: X in, Y removed, Z unrelated); (FR-544) MCP entry ensure-current including stale-workspace-arg update; (FR-545) MCP entry cleanup for harnesses not in effective list; (FR-546) filesystem-state-as-source-of-truth (no sidecar file); (FR-547) `SyncOutcome` summary structure (use devs:rust-dev agent)
- [ ] T153 [GIT] Commit: `feat(harness): sync algorithm + rules_file + mcp_config implementations (StubHarness exercised end-to-end)`

### Slice US1.c — first production harness module (`claude_code`)

- [ ] T154 [US1] Implement `src/harness/claude_code.rs` per research §R-8: `name() -> "claude-code"`; `detect(home)` checks `home/.claude/` existence; `rules_file_target(project)` returns `AGENTS.md` if present, else `CLAUDE.md` if present, else `.claude/CLAUDE.md` (precedence ladder); `rules_file_strategy() -> BlockInExistingFile`; `block_body_style() -> AtInclude`; `mcp_config_path(project, home) -> project/.claude/settings.json`; `mcp_config_format() -> Json`; `mcp_parent_key() -> "mcpServers"` (use devs:rust-dev agent)
- [ ] T155 [US1] Reviewer-deferred verification: before this slice's PR opens, verify each path/key in T154 against the current Claude Code documentation (reviewer M11-equivalent: record in the PR body "verified against [Claude Code docs URL] dated YYYY-MM-DD; no conflict found"); same discipline is repeated for codex, gemini, cursor, opencode in US3.c (use devs:rust-dev agent)
- [ ] T156 [US1] Wire `SUPPORTED_HARNESSES` to include `claude_code` as a real impl; the other four remain stubs returning `unimplemented!()` until US3.c (use devs:rust-dev agent)
- [ ] T157 [US1] Add `tests/harness_module_claude_code.rs` covering: `detect()` honours `home/.claude/` existence; `rules_file_target` precedence (AGENTS.md > CLAUDE.md > .claude/CLAUDE.md); `mcp_config_path` returns `<project>/.claude/settings.json` (use devs:rust-dev agent)
- [ ] T158 [US1] End-to-end test: `tests/workspace_use_claude_code_e2e.rs` — `tome workspace use my-workspace` inside a fixture project bound to a workspace with one enabled plugin writes the Tome block (AtInclude form) into the project's `AGENTS.md` AND writes the Tome MCP entry into `<project>/.claude/settings.json`. Uses real (Claude Code) harness module; uses `StubEmbedder` for plugin enable; uses `StubSummariser` (F6) for the workspace's cached summaries (use devs:rust-dev agent)
- [ ] T159 [GIT] Commit: `feat(harness): claude-code harness module + end-to-end bind test`

### Slice US1.d — cross-product tests + harness-clash + closeout

- [ ] T160 [US1] Add `tests/workspace_use_cross_product.rs` covering the bind-pre-state combinations: (a) no marker exists; (b) marker exists pointing to the same workspace (idempotent); (c) marker exists pointing to a different workspace (rebind UPSERTs); (d) DB has a stale `workspace_projects` row for this path (deleted marker after binding) — re-bind heals (use devs:rust-dev agent)
- [ ] T161 [US1] Add `tests/workspace_use_forward_progress.rs` covering FR-403 forward-progress: the binding INSERT/UPSERT remains committed even when the harness-sync step fails; on harness-clash (code 19), the project's `.tome/config.toml` exists and names the new workspace; on filesystem write failure (code 7), same (use devs:rust-dev agent)
- [ ] T162 [US1] Add `tests/workspace_use_atomicity.rs` covering atomic-populated-directory landing: SIGINT before `keep()` cleans the staged temp dir; SIGINT after `keep()` and before `rename` leaves a documented `.tome.tmp.*` orphan (cleanup deferred to US5's doctor `--fix`) (use devs:rust-dev agent)
- [ ] T163 [US1] Add `tests/workspace_use_concurrent.rs` exercising two parallel bind commands from two threads via `std::sync::Barrier::new(2)`; both writes go through the advisory lock; the `workspace_projects` UPSERT is idempotent; last commit wins (use devs:rust-dev agent)
- [ ] T164 [US1] Add `tests/exit_codes.rs` row coverage for code 13 (bind to missing workspace), code 19 (harness-clash without `--force`), code 2 (cwd is `<home>`) reached via the CLI binary (use devs:rust-dev agent)
- [ ] T165 [US1] Dispatch four reviewers in parallel against the US1 source (per research §R-18 — run the multi-agent review at every user-story close, not only phase close): contract audit (against contracts/workspace-commands.md, harness-modules.md, rules-file-integration.md, mcp-config-integration.md, sync-algorithm.md), Rust-lens code review, test audit, security audit. Collate findings into `review/us1-findings.md` and triage in `review/us1-disposition.md` (use devs:code-reviewer agent + Explore agent for context)
- [ ] T166 [US1] Apply any blocker-class findings from T165; sliced per finding; conventional commit per fix (use devs:rust-dev agent)
- [ ] T167 [US1] Apply any major-class findings from T165 (use devs:rust-dev agent)
- [ ] T168 [GIT] Commit: `fix: US1 reviewer-flagged fixups`

### End-of-phase

- [ ] T169 [US1] Run `/sdd:map incremental` to refresh codebase docs against US1 changes (use devs:rust-dev agent)
- [ ] T170 [US1] Review `retro/P3.md` and extract critical learnings to CLAUDE.md (conservative)

### Phase Completion

- [ ] T171 [GIT] Commit: `docs(codebase): refresh after Phase 4 / P3 US1`
- [ ] T172 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T173 [GIT] Update PR with US1 summary
- [ ] T174 [GIT] Verify all CI checks pass
- [ ] T175 [GIT] Report PR ready status

**Checkpoint**: `tome workspace use <name>` ships end-to-end against Claude Code. SC-104 + SC-110 + SC-112 + (partially) SC-114 are reachable.

---

## Phase 4: User Story 2 — Manage workspace lifecycle (Priority: P2)

**Goal**: A complete workspace-lifecycle command surface: `init`, `list`, `info`, `rename`, `remove`, `sync`, `regen-summary`. Lifecycle is independent of any particular harness — these commands operate purely on the central registry + workspace directories.

**Independent Test**: From an install with at least two workspaces and at least one bound project, every lifecycle command runs to completion: `list` shows the workspaces with bound-project + enabled-plugin counts; `info` reports per-workspace details; `rename` updates the central directory + DB row + every bound project's marker; `remove --force` cascades; `sync` propagates the workspace's rules content; `regen-summary` regenerates and re-syncs.

**Slice plan** per research §R-13:
- US2.a — `init` + `list` + `info` + `rename` + `regen-summary` (uses StubSummariser from F6)
- US2.b — `remove` with cascade ordering + reserved-name check + bound-project rejection + override-flag cascade
- US2.c — `sync` (per-workspace + all-workspaces) + closeout

### Phase Start

- [ ] T200 [GIT] Verify working tree is clean before starting Phase 4 / US2
- [ ] T201 [US2] Create `retro/P4.md` from the standard retro template
- [ ] T202 [GIT] Commit: `docs(retro): initialise Phase 4 / P4 retro`

### Slice US2.a — init + list + info + rename + regen-summary

- [ ] T203 [US2] Extend `src/cli.rs` `WorkspaceCommand` enum with `Init(WorkspaceInitArgs)`, `List(WorkspaceListArgs)`, `Rename(WorkspaceRenameArgs)`, `RegenSummary(WorkspaceRegenSummaryArgs)` variants per data-model.md §17 (use devs:rust-dev agent)
- [ ] T204 [US2] Extend `commands::workspace::info::assemble` (Phase 3 silent-compute entry point) to include the Phase 4 fields: enrolled catalogs (from `workspace_catalogs`), enabled plugins (from `workspace_skills` grouped by plugin), bound projects (from `workspace_projects`), cached summary lengths (from `[summaries]` section of settings.toml). Add a `<name>` argument so `info` works against any workspace, not just the resolved one (FR-402) (use devs:rust-dev agent)
- [ ] T205 [US2] Create `src/commands/workspace/init.rs` per FR-400: validate name via `WorkspaceName::parse` (refuses `global`); refuse if workspace already exists (code 14); use `util::atomic_dir::land_directory` to land the workspace directory at `<root>/workspaces/<name>/`; write `settings.toml` with `name = "<name>"` (and optional `[catalogs]` inherited from global if `--inherit-global`); write empty `RULES.md`; INSERT `workspaces` row with `created_at = now`, `last_used_at = now`; rolls everything back on failure (use devs:rust-dev agent)
- [ ] T206 [US2] Implement `--inherit-global` per FR-400: copies global's `workspace_catalogs` rows into the new workspace's settings.toml `[[catalogs]]` array AND inserts a row per catalog into `workspace_catalogs` for the new workspace; if global has no enrolled catalogs, the flag is a documented no-op (use devs:rust-dev agent)
- [ ] T207 [US2] Create `src/commands/workspace/list.rs` per FR-401: read `workspaces` table; for each workspace, count catalogs (`SELECT count(*) FROM workspace_catalogs WHERE workspace_id = ?`), enabled plugins (`SELECT count(DISTINCT (catalog, plugin)) FROM workspace_skills JOIN skills WHERE workspace_skills.workspace_id = ?`), indexed skills (`SELECT count(*) FROM workspace_skills WHERE workspace_id = ?`), bound projects (`SELECT count(*) FROM workspace_projects WHERE workspace_id = ?`), `last_used_at`. Tabular human form using `presentation::tables`; structured form is a `Vec<WorkspaceListEntry>`. Both byte-stable (use devs:rust-dev agent)
- [ ] T208 [US2] Create `src/commands/workspace/rename.rs` per FR-404: validate `<new>` via `WorkspaceName::parse`; refuse if `<new>` exists; refuse renaming `global` (use `WorkspaceName::is_reserved`); under the advisory lockfile, walk every bound project's marker (lexicographic by `project_path`), update each project's `.tome/config.toml` to the new name via the per-file atomic-write discipline; if any bound project's directory is missing → exit 70 with no state changed (transaction rolls back); rename the central workspace directory atomically via `std::fs::rename`; UPDATE `workspaces.name` (use devs:rust-dev agent)
- [ ] T209 [US2] Create `src/commands/workspace/regen_summary.rs` per FR-407: load enabled plugins for the resolved workspace; invoke `Summariser::summarise` (returns `LlamaSummariser` in production, `StubSummariser` in tests); on success, write both summaries (short + long) into the workspace's `[summaries]` section of settings.toml with `generated_at` RFC 3339 timestamp; rewrite `<root>/workspaces/<name>/RULES.md` body from the long summary; call `workspace::sync_to_bound_projects(name)` in the same invocation (use devs:rust-dev agent)
- [ ] T210 [US2] Wire the summariser failure path per FR-424: missing model, checksum mismatch, backend init failure, or empty/unparsable output all return `TomeError::SummariserFailure { kind }` exiting 20; prior cached summaries (if any) remain unchanged; the command exits with 20 (use devs:rust-dev agent)
- [ ] T211 [US2] Wire the length-window warnings per FR-425: a short summary > `SHORT_MAX_CHARS` or long > `LONG_MAX_CHARS` emits a `tracing::warn!` naming the workspace and observed length; the cached value is still written (use devs:rust-dev agent)
- [ ] T212 [US2] Add `tests/workspace_init.rs` covering: name validation (invalid → exit 15); already-exists → exit 14; reserved-name `global` rejected by init (note: `global` itself is seeded on bootstrap, not re-init-able) → exit 15; happy-path creates directory + DB row + empty RULES.md; `--inherit-global` copies catalogs from global; partial-write failure rolls back via `land_directory` (use devs:rust-dev agent)
- [ ] T213 [US2] Add `tests/workspace_list.rs` covering: empty install (one row: `global`); two workspaces with different catalog/plugin counts; tabular shape; JSON wire-stability via byte-comparison snapshot (use devs:rust-dev agent)
- [ ] T214 [US2] Add `tests/workspace_info.rs` (extend Phase 3's `tests/workspace_info.rs`) for Phase 4 fields: enrolled catalogs list, enabled plugins list, bound projects list, summary cache state; `info <name>` works against any workspace; missing workspace → exit 13 (use devs:rust-dev agent)
- [ ] T215 [US2] Add `tests/workspace_rename.rs` covering: `<new>` invalid → exit 15; `<new>` already exists → exit 14; rename `global` → exit 15 (reserved); zero bound projects → rename succeeds; >0 bound projects → all marker configs updated atomically; one missing project dir → exit 70 with no state changed (use devs:rust-dev agent)
- [ ] T216 [US2] Add `tests/workspace_regen_summary.rs` covering: `StubSummariser` is invoked when summaries are required; summaries are written to settings.toml under `[summaries]`; RULES.md is rewritten from the long summary; bound projects are re-synced; missing summariser model → exit 20 (covered via `SummariserFailure::ModelMissing` injection point) (use devs:rust-dev agent)
- [ ] T217 [US2] Add `tests/workspace_global_protected_rename.rs` asserting that `tome workspace rename global anything` refuses with exit 15 (reserved); `tome workspace rename anything global` also refuses (target collides with reserved) (use devs:rust-dev agent)
- [ ] T218 [GIT] Commit: `feat(workspace): init + list + info + rename + regen-summary`

### Slice US2.b — remove with cascade

- [ ] T219 [US2] Extend `src/cli.rs` `WorkspaceCommand` with `Remove(WorkspaceRemoveArgs { name, force })` (use devs:rust-dev agent)
- [ ] T220 [US2] Create `src/commands/workspace/remove.rs` per FR-405: refuse if name is `global` (reserved) — code 15 (FR-405 final sentence); refuse without `--force` if any project is bound — code 16 (`WorkspaceHasBoundProjects`) carrying the name + count + list of bound project paths; with `--force`, execute the cascade in the explicit numbered order under the advisory lockfile (use devs:rust-dev agent)
- [ ] T221 [US2] Implement the FR-405 cascade step (1): tear down integration in every bound project — for each bound project path, compute the effective harness list (via the resolver from F8), run `harness::sync_for_project_root` with the workspace's bind effectively withdrawn (the `workspace_projects` row is still present at this step but the `bound_workspace` is conceptually "about to be removed"); remove the Tome rules-file block (or standalone file) per harness; remove the Tome MCP entry per harness; if a bound project's directory is missing on disk, log a debug line per Edge Cases and continue (use devs:rust-dev agent)
- [ ] T222 [US2] Implement cascade step (2): remove each bound project's marker directory (the `<project>/.tome/` directory itself) via `std::fs::remove_dir_all` (use devs:rust-dev agent)
- [ ] T223 [US2] Implement cascade step (3): inside one DB transaction, DELETE workspace_skills rows, then workspace_catalogs rows, then workspace_projects rows, then the workspaces row — in that order. Commit (use devs:rust-dev agent)
- [ ] T224 [US2] Implement cascade step (4): remove the workspace's central directory `<root>/workspaces/<name>/` via `std::fs::remove_dir_all` (use devs:rust-dev agent)
- [ ] T225 [US2] Implement cascade step (5): refcount-clean any catalog clone no longer referenced by any workspace per FR-366 / FR-367 — for each catalog URL referenced by the removed workspace, query `workspace_catalogs` for remaining references; if zero, `remove_dir_all` the cache at `<root>/catalogs/<url-hash>/`. Runs under the same advisory lock as step (3) so the refcount check + cache delete are atomic relative to other writers (use devs:rust-dev agent)
- [ ] T226 [US2] Implement failure semantics: failure at steps (1) or (2) → DB unchanged (no rows deleted yet); failure at step (4) → orphaned central directory, detectable by doctor and cleanable by re-running `workspace remove` (idempotent past step 3); failure at step (5) → orphaned cache directory, same recovery (use devs:rust-dev agent)
- [ ] T227 [US2] Add the `WorkspaceRemoveOutcome` struct (`removed`, `bound_projects_torn_down`, `catalog_caches_cleaned: Vec<String>`, `orphaned_paths: Vec<PathBuf>`) for `--json` (use devs:rust-dev agent)
- [ ] T228 [US2] Add `tests/workspace_remove.rs` covering: `remove global` refuses → exit 15 (`is_reserved`); `remove non-global` with zero bound projects + zero catalog references → happy path; with bound projects without `--force` → exit 16; with `--force` → cascade succeeds (all five steps); concurrent `remove` with another `workspace use` for the same workspace via `Barrier::new(2)` — only one wins, the other observes the workspace already gone with exit 13 (use devs:rust-dev agent)
- [ ] T229 [US2] Add `tests/workspace_remove_cascade.rs` covering: cascade step (1) tears down a Claude Code MCP entry in a bound project; cascade step (5) refcount-cleans a catalog cache when no other workspace references the URL; concurrent `catalog add` for the same URL serialised through the advisory lock per FR-366 (use devs:rust-dev agent)
- [ ] T230 [US2] Add `tests/workspace_remove_orphan_recovery.rs` covering: simulated failure at step (4) leaves an orphaned `<root>/workspaces/<name>/`; re-running `workspace remove --force <name>` is idempotent and cleans the orphan (use devs:rust-dev agent)
- [ ] T231 [GIT] Commit: `feat(workspace): remove with cascade ordering and refcount cleanup`

### Slice US2.c — sync (per-workspace + all-workspaces)

- [ ] T232 [US2] Extend `src/cli.rs` `WorkspaceCommand` with `Sync(WorkspaceSyncArgs { name: Option<WorkspaceName> })` (use devs:rust-dev agent)
- [ ] T233 [US2] Create `src/commands/workspace/sync.rs` per FR-406: with a name argument, copies `<root>/workspaces/<name>/RULES.md` to every bound project's marker rules file copy; with no argument, syncs every workspace. Idempotent (no write if the source file's bytes equal the destination's bytes — check `==` before `fs::write`); MUST NOT regenerate summaries (use devs:rust-dev agent)
- [ ] T234 [US2] Wire the `WorkspaceSyncOutcome { synced_projects: Vec<PathBuf>, unchanged: Vec<PathBuf>, missing_project_dirs: Vec<PathBuf> }` for `--json` (use devs:rust-dev agent)
- [ ] T235 [US2] Add `tests/workspace_sync.rs` covering: with no arg, all workspaces synced; with a name, only that workspace synced; missing bound project directories logged + skipped (Edge Cases); idempotent re-run produces zero writes (verified by `stat(2)` mtime unchanged on already-matching files) (use devs:rust-dev agent)

### End-of-phase closeout

- [ ] T236 [US2] Dispatch the four-reviewer parallel pass against US2 source (contract audit, Rust-lens, test audit, security audit) per research §R-18; collate to `review/us2-findings.md` + `review/us2-disposition.md` (use devs:code-reviewer agent)
- [ ] T237 [US2] Apply any US2 blocker-class findings (use devs:rust-dev agent)
- [ ] T238 [US2] Apply any US2 major-class findings (use devs:rust-dev agent)
- [ ] T239 [GIT] Commit: `fix: US2 reviewer-flagged fixups`
- [ ] T240 [US2] Run `/sdd:map incremental` to refresh codebase docs against US2 changes (use devs:rust-dev agent)
- [ ] T241 [US2] Review `retro/P4.md` and extract critical learnings to CLAUDE.md (conservative)

### Phase Completion

- [ ] T242 [GIT] Commit: `docs(codebase): refresh after Phase 4 / P4 US2`
- [ ] T243 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T244 [GIT] Update PR with US2 summary
- [ ] T245 [GIT] Verify all CI checks pass
- [ ] T246 [GIT] Report PR ready status

**Checkpoint**: All eight workspace lifecycle commands ship. SC-103 + SC-105 + SC-117 reachable.

---

## Phase 5: User Story 3 — Layered settings + composition (Priority: P2)

**Goal**: Each scope (project, workspace, global) may declare a `harnesses` array. The effective list for any project is computed by walking the scopes in priority order with composition references (`[workspace]`, `[workspaces.<name>]`, `[global]`, `<name>`, `!<name>`) supported. A `tome harness` command surface exposes inspection + scope-level editing.

**Independent Test**: With harness lists declared at multiple scopes, `tome harness list` reports the effective list annotated by source scope. A composition cycle is detected and refused with exit 17. `[workspace]` outside project scope refuses with exit 17. `tome harness use <name> --scope workspace` appends a harness to the workspace's settings file and runs sync if the effective list changed.

**Slice plan** per research §R-13:
- US3.a — settings parser + composition resolver + cycle detection (pure compute, all library API)
- US3.b — `[workspace]` valid-only-in-project enforcement + `!`-prefix validation + harness-not-supported check
- US3.c — `tome harness` command surface (list, use, remove, info, sync, bare); scope annotation; closeout

### Phase Start

- [ ] T250 [GIT] Verify working tree is clean before starting Phase 5 / US3
- [ ] T251 [US3] Create `retro/P5.md` from the standard retro template
- [ ] T252 [GIT] Commit: `docs(retro): initialise Phase 4 / P5 retro`

### Slice US3.a — settings parser + composition resolver + cycle detection

- [ ] T253 [US3] Complete `src/settings/resolver.rs::resolve_effective_list` (skeleton landed in F8): implement the DFS visit algorithm tracking `(ScopeKind, ScopeKey)` visited tuples; cycle detection emits `CompositionErrorKind::Cycle { path }` naming every scope in the loop chain (FR-445) (use devs:rust-dev agent)
- [ ] T254 [US3] Implement priority-walk-with-stop-at-first-declarer per FR-441: walk project → workspace → global; stop at the first scope that has a `harnesses` key (even if the key's value is `[]`); further scopes are only consulted via explicit composition references (use devs:rust-dev agent)
- [ ] T255 [US3] Implement composition expansion per FR-444: union of every reachable scope's directly-declared names; `!`-prefixed names subtracted from the union after expansion; order of entries within an array does not affect the result; output order honours first-included-from for stable `tome harness list` output (use devs:rust-dev agent)
- [ ] T256 [US3] Implement FR-449 — composition references resolve to the referenced scope's **directly-declared** harness list (verbatim from the settings file), NOT its computed effective list. Pin this with an explicit assertion in `tests/settings_composition_resolves_to_as_written.rs` (use devs:rust-dev agent)
- [ ] T257 [US3] Implement scope annotation per FR-441 reporting: `EffectiveHarness::source_chain: Vec<ScopeKind>` records the chain of references that brought the name into the list (Project → Workspace → Global) (use devs:rust-dev agent)
- [ ] T258 [US3] Implement excluded-names reporting per FR-441: `EffectiveHarnessList::excluded: Vec<String>` carries names subtracted by `!`-prefixes (for `tome harness list` reporting) (use devs:rust-dev agent)
- [ ] T259 [US3] Add `tests/settings_priority.rs` covering: global declares list, no project/workspace → effective from global; workspace declares list, no project → effective from workspace (global NOT consulted unless workspace's list references global); project declares list, workspace + global ignored unless project's list references them; empty `[]` at any scope terminates walk with empty effective list (FR-442) (use devs:rust-dev agent)
- [ ] T260 [US3] Add `tests/settings_composition.rs` covering: all five forms (plain include, `!`-exclude, `[workspace]`, `[workspaces.<name>]`, `[global]`); multi-level composition; explicit add + explicit remove combinations; per FR-444 the order of entries does not affect the result (use devs:rust-dev agent)
- [ ] T261 [US3] Add `tests/settings_cycle_detection.rs` covering: workspace A includes B which includes A → cycle error naming both ends (FR-445); cycle through a path of length one (workspace's settings references itself); cycle through project → workspace → workspace (use devs:rust-dev agent)
- [ ] T262 [US3] Add `tests/settings_composition_resolves_to_as_written.rs` (FR-449 invariant): a project's `["[workspace]", "!cursor"]` whose bound workspace has NO `harnesses` declaration resolves to the empty set (with `cursor` then subtracted from nothing). Does NOT fall through to global (use devs:rust-dev agent)
- [ ] T263 [GIT] Commit: `feat(settings): composition resolver + cycle detection + scope annotation`

### Slice US3.b — `[workspace]` valid-only-in-project + `!`-prefix validation + harness-not-supported

- [ ] T264 [US3] Enforce FR-446: `[workspace]` in a workspace's or global settings file → `CompositionErrorKind::WorkspaceRefOutsideProject` (exit 17) (use devs:rust-dev agent)
- [ ] T265 [US3] Enforce FR-446 from-project-without-marker: `[workspace]` in a project's settings file when the project's marker does not name a workspace (binding broken) → same error (use devs:rust-dev agent)
- [ ] T266 [US3] Enforce FR-447: composition reference to a non-existent workspace → `CompositionErrorKind::UnknownWorkspace(name)` (exit 13 per the reused-variant table FR-602; mapped to `WorkspaceNotFound`) (use devs:rust-dev agent)
- [ ] T267 [US3] Enforce FR-448: `!`-prefixed entry that contains brackets (e.g. `"![global]"`, `"![workspaces.foo]"`) → `CompositionErrorKind::BadExclusion` (exit 17); pin the rule that exclusions accept plain harness names only (use devs:rust-dev agent)
- [ ] T268 [US3] Enforce FR-450: TOML array elements parsed as strings; treating bracketed forms as TOML table headers is an implementation error — add a `tests/settings_array_types.rs` test that uses a settings file with `harnesses = [[workspaces.foo]]` (table header) — must fail to parse with a documented serde error message naming the line (use devs:rust-dev agent)
- [ ] T269 [US3] Enforce FR-460: a harness name in any settings file that is not one of the five supported (`claude-code`, `codex`, `gemini`, `cursor`, `opencode`) → `TomeError::HarnessNotSupported { name }` (exit 18). The check happens at the end of composition resolution against the `SUPPORTED_HARNESSES` static (use devs:rust-dev agent)
- [ ] T270 [US3] Add `tests/settings_workspace_ref_outside_project.rs` covering FR-446 (use devs:rust-dev agent)
- [ ] T271 [US3] Add `tests/settings_bad_exclusion.rs` covering FR-448 (use devs:rust-dev agent)
- [ ] T272 [US3] Add `tests/settings_harness_not_supported.rs` covering FR-460 across all three scope file types (use devs:rust-dev agent)
- [ ] T273 [GIT] Commit: `feat(settings): composition validation rules (workspace-ref, bad-exclusion, unsupported)`

### Slice US3.c — `tome harness` command surface

- [ ] T274 [US3] Implement the remaining four harness modules per research §R-8, each ~50 lines, in parallel: (a) `src/harness/codex.rs` (rules-file: AGENTS.md, MCP: `~/.codex/config.toml` TOML, parent key `mcp_servers`, body style AtInclude); (b) `src/harness/gemini.rs` (rules-file precedence: AGENTS.md > GEMINI.md > .gemini/GEMINI.md, MCP: `~/.gemini/settings.json` JSON, parent key `mcpServers`, body style AtInclude); (c) `src/harness/cursor.rs` (RulesFileStrategy::StandaloneFile at `<project>/.cursor/rules/TOME_SKILLS.md`, MCP: `<project>/.cursor/mcp.json` JSON, parent key `mcpServers`); (d) `src/harness/opencode.rs` (rules-file: AGENTS.md, MCP: `<project>/opencode.json` JSON, parent key `mcpServers`, body style Inline) (use devs:rust-dev agent)
- [ ] T275 [US3] Reviewer-deferred verification (per US1.c precedent + reviewer M11-equivalent): record in the PR body for T274 "verified against [Codex CLI docs URL], [Gemini CLI docs URL], [Cursor docs URL], [OpenCode docs URL] dated 2026-05-22; no conflict found" (use devs:rust-dev agent)
- [ ] T276 [US3] Add `Command::Harness(HarnessArgs)` to `src/cli.rs` with nested `HarnessCommand { List, Use, Remove, Info, Sync }` enum + bare-`tome harness` dispatch (use devs:rust-dev agent)
- [ ] T277 [US3] Create `src/commands/harness/mod.rs` dispatcher (mirrors `commands/plugin/mod.rs` shape); creates `src/commands/harness/list.rs`, `use_.rs`, `remove.rs`, `info.rs`, `sync.rs` (use devs:rust-dev agent)
- [ ] T278 [US3] Implement bare `tome harness` per FR-520: tabular form, columns = name, detected (yes/no via `HarnessModule::detect`), rules-file target (if project resolved), MCP config target. JSON form available (use devs:rust-dev agent)
- [ ] T279 [US3] Implement `tome harness list [<workspace>]` per FR-521: no arg → effective list for current project with `source_chain` annotations + excluded names section; with arg → directly-declared `harnesses` array verbatim (the "as-written" view) (use devs:rust-dev agent)
- [ ] T280 [US3] Implement `tome harness use <name> [--scope project|workspace|global]` per FR-522: default scope = project; append to the chosen scope's `harnesses` key (create key if absent); recompute the effective list for the current project; if the effective list changed, run `harness::sync_for_project_root`; if unchanged, print informational notice naming the scope edited (use devs:rust-dev agent)
- [ ] T281 [US3] Implement `tome harness remove <name> [--scope ...]` per FR-523: mirror `use` logic; if removal changes effective list, run sync (which removes the harness's Tome entry per FR-545 + FR-483); if unchanged, informational notice (use devs:rust-dev agent)
- [ ] T282 [US3] Implement `tome harness info <name>` per FR-524: one-line description; `detect()` result + paths if detected; rules-file target for current project; MCP config target; current filesystem integration state (rules block present? MCP entry present? Tome-owned?); which scopes reference this harness, including composition annotation (use devs:rust-dev agent)
- [ ] T283 [US3] Implement `tome harness sync` per FR-525: compute effective list; reconcile filesystem state via `harness::sync_for_project_root` (the shared library entry from US1.b); MUST be byte-for-byte idempotent — if no file content would change, NO file is rewritten (mtime does NOT advance); stdout reports "no changes" in human form (empty `changes` array in JSON) (use devs:rust-dev agent)
- [ ] T284 [US3] Wire `--scope` parsing as a clap enum (`project`, `workspace`, `global`) with `project` as default; explicit `--scope project` refuses outside any project marker with exit 2 + helpful message (use devs:rust-dev agent)
- [ ] T285 [US3] Wire writes to settings.toml via `toml_edit::Document` so developer-authored comments are preserved (FR-349) (use devs:rust-dev agent)
- [ ] T286 [US3] Add `tests/harness_bare.rs` covering FR-520 tabular output (use devs:rust-dev agent)
- [ ] T287 [US3] Add `tests/harness_list_effective.rs` covering FR-521 no-arg: source_chain annotations + excluded names section (use devs:rust-dev agent)
- [ ] T288 [US3] Add `tests/harness_list_as_written.rs` covering FR-521 with-arg: directly-declared list verbatim, no expansion (use devs:rust-dev agent)
- [ ] T289 [US3] Add `tests/harness_use_scope.rs` covering FR-522 across all three scopes; the file written is the correct settings file; if list unchanged, sync NOT invoked (use devs:rust-dev agent)
- [ ] T290 [US3] Add `tests/harness_remove_scope.rs` covering FR-523 mirror semantics (use devs:rust-dev agent)
- [ ] T291 [US3] Add `tests/harness_info.rs` covering FR-524 (use devs:rust-dev agent)
- [ ] T292 [US3] Add `tests/sync_idempotence.rs` covering FR-525 byte-for-byte: second sync invocation produces zero writes; mtime unchanged on every harness's rules file + MCP config file; verified via `stat(2)` comparisons before/after (use devs:rust-dev agent)
- [ ] T293 [US3] Add `tests/harness_modules.rs` covering each of the five harness modules: `detect()`, `rules_file_target()`, `mcp_config_path()`, `block_body_style()`, `mcp_parent_key()` return the documented values per research §R-8 (use devs:rust-dev agent)

### End-of-phase closeout

- [ ] T294 [US3] Dispatch the four-reviewer parallel pass against US3 source (use devs:code-reviewer agent)
- [ ] T295 [US3] Apply US3 blocker + major findings (use devs:rust-dev agent)
- [ ] T296 [GIT] Commit: `fix: US3 reviewer-flagged fixups`
- [ ] T297 [US3] Run `/sdd:map incremental` to refresh codebase docs against US3 changes (use devs:rust-dev agent)
- [ ] T298 [US3] Review `retro/P5.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T299 [GIT] Commit: `docs(codebase): refresh after Phase 4 / P5 US3`

### Phase Completion

- [ ] T300 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T301 [GIT] Update PR with US3 summary
- [ ] T302 [GIT] Verify all CI checks pass
- [ ] T303 [GIT] Report PR ready status

**Checkpoint**: Layered settings + composition + 5-harness command surface ship. SC-107 + SC-108 + SC-109 + SC-110 + SC-111 + SC-112 + SC-113 reachable.

---

## Phase 6: User Story 4 — Summarisation + RULES.md (Priority: P2)

**Goal**: Each workspace describes its enabled-plugin set in plain English through two cached summaries (short + long). The short summary is interpolated into the MCP search tool's description; the long summary becomes the body of the workspace's RULES.md. Both regenerate whenever the workspace's enabled-plugin state changes. The summariser is a bundled local LLM (Qwen2.5-0.5B-Instruct via `llama-cpp-2`), lazily loaded.

**Independent Test**: With the summariser model downloaded, enabling a plugin in a workspace produces a short summary (~400-800 chars) and a long summary (~1500-2500 chars) cached in the workspace settings file. The MCP search tool's description includes the short summary. Disabling regenerates both. An explicit `regen-summary` command regenerates on demand. Summariser failures (model missing, output empty) exit with code 20 and the prior skill-state mutation is retained (forward-progress per FR-385).

**Slice plan** per research §R-13:
- US4.a — production `LlamaSummariser` + prompts module + length-window enforcement
- US4.b — trigger wiring (enable/disable/reindex/catalog-update + FR-385 forward-progress) + MCP server cached-short-summary readout
- US4.c — `regen-summary` end-to-end + closeout

### Phase Start

- [ ] T310 [GIT] Verify working tree is clean before starting Phase 6 / US4
- [ ] T311 [US4] Create `retro/P6.md` from the standard retro template
- [ ] T312 [GIT] Commit: `docs(retro): initialise Phase 4 / P6 retro`

### Slice US4.a — production `LlamaSummariser`

- [ ] T313 [US4] Implement `src/summarise/llama.rs::LlamaSummariser::new(paths: &Paths) -> Result<Self, TomeError>`: load model from `<root>/models/qwen2.5-0.5b-instruct/model.gguf`; verify SHA-256 checksum against the pinned value in `MODEL_REGISTRY`; on mismatch return `SummariserFailure { kind: ModelChecksumMismatch }`; on missing file return `ModelMissing` (use devs:rust-dev agent)
- [ ] T314 [US4] Implement `LlamaSummariser::summarise(input)` per research §R-2: borrow the `LlamaBackend` singleton via `summarise::backend()?`; load `LlamaModel` from the model path; create `LlamaContext`; run `SHORT_PROMPT` (substituting the plugin descriptions into `{descriptions}`); run `LONG_PROMPT` (substituting the short summary's topics into `{topics}`); drop model + context (backend stays alive) (use devs:rust-dev agent)
- [ ] T315 [US4] Wire inference sampling parameters per contracts/summariser.md: `temperature=0.3`, `top_p=0.9`, `repeat_penalty=1.1` (deterministic-leaning) (use devs:rust-dev agent)
- [ ] T316 [US4] Enforce length windows per FR-425 + research §R-15: short summary < 1 char or unparsable → `SummariserFailure { kind: OutputEmpty(Short) | OutputUnparsable(Short) }` (exit 20); same for long; short or long EXCEEDING the documented max → `tracing::warn!` naming the workspace and observed length, but the value is still returned (non-fatal warning per FR-425) (use devs:rust-dev agent)
- [ ] T317 [US4] Wire the LlamaSummariser into `commands::workspace::regen_summary` (replacing the F6 stub return-Err) so production calls hit the real summariser; tests continue to use `StubSummariser` via constructor injection (use devs:rust-dev agent)
- [ ] T318 [US4] Audit and wire `tome models` extension to list the summariser as the third entry (FR-420) — `tome models list` shows three rows; `tome models download qwen2.5-0.5b-instruct` downloads the summariser model; `tome models remove qwen2.5-0.5b-instruct` removes it (use devs:rust-dev agent)
- [ ] T319 [US4] Wire the byte-progress callback (added to `embedding::download::download_model` in T058) on the summariser download path — the indicatif spinner displays progress for the ~400 MB Qwen download (use devs:rust-dev agent)
- [ ] T320 [US4] Reviewer-deferred fold-in (M8): document the `LlamaBackend::init()` race semantics in `src/summarise/mod.rs::backend()`'s doc-comment — `OnceLock::get_or_try_init` ensures init runs exactly once; concurrent calls block on the inner mutex until init completes; if init returns `BackendAlreadyInitializedError` it's a hard panic in a single-process invariant violation (use devs:rust-dev agent)
- [ ] T321 [US4] Reviewer-deferred fold-in (M1): record the pinned `llama-cpp-2` version in a `Cargo.lock` comment for traceability; add a CHANGELOG note about the pin (use devs:rust-dev agent)
- [ ] T322 [US4] Add `tests/summariser_real.rs` — env-gated (`TOME_TEST_REAL_MODELS=1`) integration test that downloads the real Qwen2.5-0.5B-Instruct model, invokes `LlamaSummariser::summarise` against a fixture input (two plugins, five skills total), and asserts that both outputs are non-empty and within their length windows. CI-skipped by default per research §R-14 (use devs:rust-dev agent)
- [ ] T323 [GIT] Commit: `feat(summarise): production LlamaSummariser + length-window enforcement + tome models extension`

### Slice US4.b — trigger wiring + MCP server cached-short-summary readout

- [ ] T324 [US4] Wire summary regeneration to `tome plugin enable` per FR-380: after the workspace_skills UPSERT commits, invoke `Summariser::summarise` against the workspace's current enabled-plugin set; on success, write `[summaries]` to the workspace's settings.toml + rewrite `<root>/workspaces/<name>/RULES.md` + invoke `workspace::sync_to_bound_projects(name)` (use devs:rust-dev agent)
- [ ] T325 [US4] Wire summary regeneration to `tome plugin disable` per FR-381 — same pattern as `enable` (use devs:rust-dev agent)
- [ ] T326 [US4] Wire conditional regeneration to `tome plugin reindex` per FR-382: only regenerate if any skill's content_hash changed in the resolved workspace's enabled set; if no content hashes changed, skip the summariser invocation (cached summaries reused per FR-423) (use devs:rust-dev agent)
- [ ] T327 [US4] Wire conditional regeneration to `tome catalog update` per FR-365 + FR-423: for each workspace whose enabled plugins include any updated catalog's plugins, regenerate that workspace's summaries (per-workspace trigger, not just the resolved workspace) (use devs:rust-dev agent)
- [ ] T328 [US4] Implement FR-385 forward-progress: the skill-state mutation (workspace_skills row insert/delete) commits BEFORE the summariser is invoked; if the summariser subsequently fails, the surrounding command exits with code 20 (`SummariserFailure`); the prior workspace_skills rows are NOT rolled back; the workspace's cached summaries are NOT written; the doctor command reports the workspace's cached summary as stale (use devs:rust-dev agent)
- [ ] T329 [US4] Update MCP server tool description per FR-425: `src/mcp/tools/search_skills.rs` reads the resolved workspace's `[summaries].short` from settings.toml at startup; interpolates it into a fixed scaffold that names the search tool and explains when to call it; the total description length is best-effort against a documented agent-host budget — if exceeded, emit a tracing warning but never refuse to start. If the cached summary is absent (no successful summarisation pass yet), use the fixed scaffold alone (use devs:rust-dev agent)
- [ ] T330 [US4] MCP server MUST NOT invoke the summariser in-process (sync-boundary discipline per FR-425 + NFR-103). The cached short summary read is a one-shot file read at MCP startup; subsequent regenerations from CLI invocations write to the same file but the running MCP server keeps its in-memory description (re-startup picks up the new one) (use devs:rust-dev agent)
- [ ] T331 [US4] Reviewer-deferred fold-in (P8 / m-WKS-4): cap MCP `Input` length on `search_skills` (per research §R-17, fold the P8-deferred input-length cap into US5; here in US4 add the corresponding test fixture pinning that the description length warning fires correctly under exceed conditions, even if the cap itself lands in US5) (use devs:rust-dev agent)
- [ ] T332 [US4] Add `tests/summariser_triggers.rs` covering each trigger from FR-423: enable invokes the stub exactly once; disable invokes once; reindex with changed hash invokes once; reindex with unchanged hash invokes zero times; catalog update invokes per-workspace; regen-summary invokes once (use devs:rust-dev agent)
- [ ] T333 [US4] Add `tests/summariser_forward_progress.rs` covering FR-385: `StubSummariser::with_force_fail()` returns `SummariserFailure::OutputEmpty`; on `plugin enable` the workspace_skills rows are committed; the command exits 20; the workspace's `[summaries]` section is NOT updated; the doctor command reports the summariser subsystem as broken (use devs:rust-dev agent)
- [ ] T334 [US4] Add `tests/summariser_cache.rs` covering cache hit/miss: when settings.toml has `[summaries]` and no triggers fire, the cached values are reused (the summariser is NOT invoked — `StubSummariser::call_count() == 0`); when a trigger fires, the cache is overwritten; `generated_at` updates to the new timestamp (use devs:rust-dev agent)
- [ ] T335 [US4] Add `tests/mcp_tool_description.rs` covering FR-425: MCP server's `search_skills` tool description includes the workspace's cached short summary when present; falls back to scaffold-only when absent; a too-long description emits a tracing warning but the server still starts (use devs:rust-dev agent)
- [ ] T336 [GIT] Commit: `feat(summarise): trigger wiring + MCP cached-short-summary readout + forward-progress`

### Slice US4.c — regen-summary end-to-end + closeout

- [ ] T337 [US4] Verify `tome workspace regen-summary <name>` end-to-end through the production LlamaSummariser (existing test from US2.a — T216 — extended for the real path under the env-gate from T322) (use devs:rust-dev agent)
- [ ] T338 [US4] Add `tests/exit_codes.rs` row for exit 20 reachable via CLI: `tome workspace regen-summary <name>` with the summariser model missing → exit 20 (use devs:rust-dev agent)

### End-of-phase closeout

- [ ] T339 [US4] Dispatch the four-reviewer parallel pass against US4 source (use devs:code-reviewer agent)
- [ ] T340 [US4] Apply US4 blocker + major findings (use devs:rust-dev agent)
- [ ] T341 [GIT] Commit: `fix: US4 reviewer-flagged fixups`
- [ ] T342 [US4] Run `/sdd:map incremental` to refresh codebase docs against US4 changes (use devs:rust-dev agent)
- [ ] T343 [US4] Review `retro/P6.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T344 [GIT] Commit: `docs(codebase): refresh after Phase 4 / P6 US4`

### Phase Completion

- [ ] T345 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T346 [GIT] Update PR with US4 summary
- [ ] T347 [GIT] Verify all CI checks pass
- [ ] T348 [GIT] Report PR ready status

**Checkpoint**: Summariser ships end-to-end. SC-106 reachable. MCP tool description embeds the workspace's short summary.

---

## Phase 7: User Story 5 — Doctor extensions (Priority: P3)

**Goal**: `tome doctor` reports — and `--fix` repairs — every Phase 4 subsystem: project binding well-formedness, project-local rules-file-copy currency, per-harness rules-file integration state, per-harness MCP config integration state, and the summariser subsystem. The `subsystem` field is promoted from `String` to a typed enum (P6 retro: promote at >6 arms; Phase 4 reaches ~11).

**Independent Test**: From a bound project with two harnesses correctly integrated, `tome doctor` reports every subsystem healthy. Deleting the project's RULES.md by hand makes doctor flag the rules-copy as drifted; `--fix` re-copies. Replacing a harness's `"tome"` MCP entry's `command` field makes doctor flag the entry as user-owned and refuses to auto-fix even with `--fix`; `--fix --force` rewrites. From outside any project, doctor reports the global workspace and no binding subsystem.

**Slice plan** per research §R-13:
- US5.a — doctor reports the new subsystems; `Subsystem` enum promotion
- US5.b — `--fix` handlers + closeout

### Phase Start

- [ ] T360 [GIT] Verify working tree is clean before starting Phase 7 / US5
- [ ] T361 [US5] Create `retro/P7.md` from the standard retro template
- [ ] T362 [GIT] Commit: `docs(retro): initialise Phase 4 / P7 retro`

### Slice US5.a — doctor reports new subsystems + `Subsystem` enum promotion

- [ ] T363 [US5] Promote `subsystem: String` to a typed `Subsystem` enum in `src/doctor/report.rs` per data-model.md §15: variants `Embedder, Reranker, Index, Drift, Catalog(String), Schema, Summariser, Binding, BindingRulesCopy, HarnessRules(String), HarnessMcp(String)` (use devs:rust-dev agent)
- [ ] T364 [US5] Implement custom `Serialize` + `Deserialize` for `Subsystem` emitting/parsing the wire-shape strings per data-model.md §15: `Embedder → "embedder"`, `Catalog(name) → "catalog:<name>"`, `Summariser → "summariser"`, `BindingRulesCopy → "binding-rules-copy"`, `HarnessRules(name) → "harness-rules:<name>"`, etc. The Phase 3 byte-shape is preserved exactly for every Phase 3 variant. Add a unit test that locks the round-trip (use devs:rust-dev agent)
- [ ] T365 [US5] Refactor every dispatch site that previously matched on `&str` to match on the typed `Subsystem` enum: `doctor::fixes::apply_one`, `doctor::build_suggested_fixes`, every test in `tests/doctor*.rs` (use devs:rust-dev agent)
- [ ] T366 [US5] Create `src/doctor/binding.rs` with `check_binding(scope: &ResolvedScope, paths: &Paths, central_db: &Connection) -> ProjectBindingState`: returns `None` when scope source is not `ProjectMarker`; when it is, checks the marker's config is well-formed (parses against `ProjectMarkerConfig` strict deserialise); reads `<project>/.tome/RULES.md` and compares bytes against `<root>/workspaces/<name>/RULES.md` to compute `RulesCopyState { Match, Missing, Drift }` (use devs:rust-dev agent)
- [ ] T367 [US5] Create `src/doctor/harness_integration.rs` with `check_harness_integration(scope, paths, effective_list, home) -> (Vec<(String, SubsystemHealth)>, Vec<(String, SubsystemHealth)>)`: per harness in the effective list, check (a) the rules-file target contains a Tome block (BlockInExistingFile) or the standalone file exists (StandaloneFile) — return `Ok` or `Drift` per content match; (b) the MCP config file contains a Tome-owned entry pointing at the resolved workspace — return `Ok`, `Drift` (stale workspace arg), `Broken` (entry missing), or `UserOwned` (entry exists but command/first_arg don't match Tome's shape) (use devs:rust-dev agent)
- [ ] T368 [US5] Wire the new subsystems into `doctor::assemble_report` per data-model.md §15: `summariser: SubsystemHealth` (mirror of embedder/reranker); `project_binding: Option<ProjectBindingState>`; `effective_harness_list: Option<EffectiveHarnessList>`; `harness_rules: Vec<(String, SubsystemHealth)>`; `harness_mcp: Vec<(String, SubsystemHealth)>`; `detected_uninstalled_harnesses: Vec<String>` (FR-560 informational: supported harnesses present on machine but NOT in effective list) (use devs:rust-dev agent)
- [ ] T369 [US5] Implement summariser check: cheap-probe the `<root>/models/qwen2.5-0.5b-instruct/manifest.json` existence + SHA-256 of `model.gguf` against the pinned registry checksum when `--verify` is set (parallel to existing embedder/reranker checks); promote `summariser_name` / `summariser_version` drift detection in `index::meta::detect_drift` (use devs:rust-dev agent)
- [ ] T370 [US5] Extend `doctor::classify` to include Phase 4 subsystems per FR-561: summariser missing/corrupt → Unhealthy; binding broken (marker names missing workspace) → Unhealthy; BindingRulesCopy missing or drift → Degraded; HarnessRules drift → Degraded; HarnessMcp broken (entry missing in effective-list harness) → Degraded; HarnessMcp user-owned conflict → Degraded; empty effective list → all harness subsystems classified `NotApplicable` and don't affect overall classification (FR-561) (use devs:rust-dev agent)
- [ ] T371 [US5] Implement FR-564: from outside any project marker, doctor resolves to the `global` workspace; `project_binding` is `None`; harness subsystems report the global effective list (per the layered lookup that stops at global from outside the project's perspective) (use devs:rust-dev agent)
- [ ] T372 [US5] Implement FR-560 informational note for detected-but-not-configured harnesses: per-machine detection (via `HarnessModule::detect`) of every supported harness; harnesses present but not in effective list reported as `detected_uninstalled_harnesses` (informational; classification unaffected) (use devs:rust-dev agent)
- [ ] T373 [US5] Fold in the P8-deferred MCP `Input` length caps (per research §R-17): add a max-length validator in `src/mcp/tools/search_skills.rs` rejecting queries > 4096 chars with a dedicated error envelope; add `tests/mcp_input_length_caps.rs` (use devs:rust-dev agent)
- [ ] T374 [US5] Add `tests/doctor_p4.rs` covering each new subsystem: binding healthy / binding marker malformed (exit 70) / binding names missing workspace; BindingRulesCopy match / missing / drift; HarnessRules per-harness match / drift / removed-by-hand; HarnessMcp per-harness match / drift (stale workspace arg) / user-owned conflict / missing entry; summariser missing / present / drift on model version; doctor outside any project resolves `global` (use devs:rust-dev agent)
- [ ] T375 [US5] Add `tests/doctor_subsystem_serialize.rs` locking the `Subsystem` enum round-trip — every variant serialises to the documented string; deserialise from string round-trips back to the variant (use devs:rust-dev agent)
- [ ] T376 [US5] Add `tests/doctor_detected_uninstalled.rs` covering FR-560 informational note: a fixture home dir with `.gemini/` present + effective list `[claude-code]` reports `detected_uninstalled_harnesses: ["gemini"]` without affecting overall classification (use devs:rust-dev agent)
- [ ] T376a [US5] Add `tests/doctor_read_only_by_default.rs` covering FR-563: `tome doctor` (no `--fix`) reads the entire project tree + central DB without mutating; mtime on every file under `<root>/` and `<project>/.tome/` is unchanged before and after the invocation; verified via a `walkdir` pass + `metadata().modified()` comparison (use devs:rust-dev agent)
- [ ] T376b [US5] Per FR-461 / reviewer M3 fold-in: extend `tests/harness_modules.rs` (the existing T293) into an explicit matrix — for each of the five harness modules, assert every method on the trait (`name`, `description`, `detect`, `rules_file_target`, `rules_file_strategy`, `block_body_style`, `mcp_config_path`, `mcp_config_format`, `mcp_parent_key`) returns the documented value per research §R-8. Five harnesses × nine methods = a 45-row matrix; encoded as a per-method test loop driven by a `&[(&dyn HarnessModule, ExpectedValues)]` slice (use devs:rust-dev agent)
- [ ] T377 [GIT] Commit: `feat(doctor): Phase 4 subsystems + Subsystem enum promotion + detected-uninstalled informational`

### Slice US5.b — `--fix` handlers + override semantics

- [ ] T378 [US5] Extend `doctor::fixes::apply_one` dispatch ladder for the new `Subsystem` variants per FR-562: `Summariser` → re-download via `summariser::download::download_summariser_model`; `BindingRulesCopy` → re-copy from `<root>/workspaces/<name>/RULES.md`; `HarnessRules(name)` → re-run harness sync for that harness only (via a new `harness::sync_for_harness(name, project_root, deps)` library entry); `HarnessMcp(name)` → re-run harness sync for that harness; `Schema` → forward-migrate via `index::migrations::apply_pending` (folds in the F9 schema migration for cases where a v1 DB is somehow present — research §R-17) (use devs:rust-dev agent)
- [ ] T379 [US5] Implement the FR-562 NOT-auto-fixable cases: `Binding` broken when the marker names a missing workspace → `auto_fixable: false`; suggested fix message explains developer choice ("rebind or recreate the named workspace"); the `--fix` flag does NOT silently rebind (use devs:rust-dev agent)
- [ ] T380 [US5] Implement the user-owned MCP conflict case: `HarnessMcp(name)` with `UserOwned` health → `auto_fixable: false`; suggested fix names the conflict + suggests `tome harness sync --force` as the explicit-override path; `--fix` alone does NOT rewrite; `--fix --force` does (use devs:rust-dev agent)
- [ ] T381 [US5] Add the `--force` flag to `tome doctor`; pairing `--fix --force` enables the user-owned-MCP override per FR-562 (use devs:rust-dev agent)
- [ ] T382 [US5] Implement the orphaned `.tome.tmp.*` cleanup on `tome doctor --fix` per FR-410 second paragraph: scan `<root>/workspaces/` and every project marker's parent for `.tome.tmp.*` directories older than 1 hour (heuristic); remove them (folds in the P3 retro cleanup gap noted in CONCERNS.md TD-016) (use devs:rust-dev agent)
- [ ] T383 [US5] Implement exit codes for the new `--fix` paths: exit 0 if all subsystems healthy after fix; exit 1 if any subsystem remains Degraded/Broken after fix; exit 75 (`DoctorFixNotSafe`) if at least one subsystem remains unfixable (Binding broken; user-owned MCP without `--force`) (use devs:rust-dev agent)
- [ ] T384 [US5] Add `tests/doctor_fix_p4.rs` covering each repair class: missing summariser model → `--fix` re-downloads (CI-skipped path); missing BindingRulesCopy → `--fix` re-copies; drifted HarnessRules → `--fix` re-runs sync; missing HarnessMcp → `--fix` re-runs sync; user-owned HarnessMcp → `--fix` exits 75 without rewrite; `--fix --force` rewrites (use devs:rust-dev agent)
- [ ] T385 [US5] Add `tests/doctor_orphan_tmp_cleanup.rs` covering FR-410 cleanup: a sibling `.tome.tmp.abc123/` next to `<root>/workspaces/foo/` is removed on `doctor --fix` when older than 1 hour; not removed when fresher (use devs:rust-dev agent)
- [ ] T386 [US5] Add `tests/doctor_outside_project.rs` covering FR-564 + `--workspace global`: doctor from outside any project reports `global` with `project_binding == None`; harness subsystems report the global effective list (use devs:rust-dev agent)

### End-of-phase closeout

- [ ] T387 [US5] Dispatch the four-reviewer parallel pass against US5 source (use devs:code-reviewer agent)
- [ ] T388 [US5] Apply US5 blocker + major findings (use devs:rust-dev agent)
- [ ] T389 [GIT] Commit: `fix: US5 reviewer-flagged fixups`
- [ ] T390 [US5] Run `/sdd:map incremental` to refresh codebase docs against US5 changes (use devs:rust-dev agent)
- [ ] T391 [US5] Review `retro/P7.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T392 [GIT] Commit: `docs(codebase): refresh after Phase 4 / P7 US5`

### Phase Completion

- [ ] T393 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T394 [GIT] Update PR with US5 summary
- [ ] T395 [GIT] Verify all CI checks pass
- [ ] T396 [GIT] Report PR ready status

**Checkpoint**: All five user stories ship. Phase 4 feature work complete. SC-114 reachable.

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Four-reviewer parallel pass at the phase boundary (per research §R-18 / P10 retro), apply blockers + majors, deferred coverage items, documentation updates, CHANGELOG entry, README banner update, and v0.4.0 release prep. Mirrors Phase 3's PR-A → PR-H pattern.

### Phase Start

- [ ] T400 [GIT] Verify working tree is clean before starting Phase 8 / Polish
- [ ] T401 Create `retro/P8.md` from the standard retro template
- [ ] T402 [GIT] Commit: `docs(retro): initialise Phase 4 / P8 polish retro`

### PR-A — Multi-agent review (already run per-US, now phase-wide)

- [ ] T403 Dispatch four reviewers in parallel against the full Phase 4 source: contract audit (`review-compact` against `contracts/`), Rust-lens code review (`devs:code-reviewer`), test audit (custom prompt covering coverage gaps and test-quality), security audit (covering credential scrubbing, symlink refusal, file modes, atomic-write discipline). Collate findings into `review/findings.md` and triage in `review/disposition.md` (use Explore + devs:code-reviewer agents)
- [ ] T404 Triage findings per the P10 mapping table: blockers → dedicated PR; majors → dedicated PR; minors → folded; nits → wontfix unless trivial-while-nearby
- [ ] T405 [GIT] Commit: `docs(review): Phase 4 review findings + disposition`

### PR-B — Apply blocker-class fixups

- [ ] T406 Apply any blocker-class findings from T403. Slice as needed; conventional commit per fix (use devs:rust-dev agent)
- [ ] T407 [GIT] Commit: `fix: Phase 4 blocker-class reviewer findings`

### PR-C — Apply major-class fixups

- [ ] T408 Apply any major-class findings from T403 (use devs:rust-dev agent)
- [ ] T409 [GIT] Commit: `fix: Phase 4 major-class reviewer findings`

### PR-D — Deferred coverage items

- [ ] T410 Fold in remaining P10-deferred items from research §R-17 not already absorbed by earlier slices: any test gap (M-MCP-3, M-MCP-11, m-WKS-*) — open per-test PRs as needed; verify each carries explicit traceability to its retro line (use devs:rust-dev agent)
- [ ] T411 Add `tests/exit_codes_e2e.rs` rows for any new exit codes (13–20) not already exercised at the CLI binary level (extend Phase 3's `exit_codes_e2e.rs` coverage matrix) (use devs:rust-dev agent)
- [ ] T412 Add `tests/scrubbing.rs` rows for summariser model download URLs (per NFR-105) and harness MCP config paths in error chains (use devs:rust-dev agent)
- [ ] T413 Extend `tests/manifest_strictness.rs` to cover the new strict types: WorkspaceSettings, ProjectMarkerConfig, GlobalSettings, summariser ModelManifest (use devs:rust-dev agent)
- [ ] T414 Run `cargo build --release` and record the final stripped binary size as a comment update at the top of `tests/sync_boundary.rs` (anticipated ~28.4 MiB on macOS arm64, ~34 MB on Linux x86_64 per research §R-4; verify against the 50 MB cap) (use devs:rust-dev agent)
- [ ] T415 [GIT] Commit: `test: close Phase 4 deferred coverage items`

### PR-E — Security hardening

- [ ] T416 Verify 0600 mode on every Phase 4 Tome-owned file write per FR-305 + NFR-104: workspace settings.toml, project marker config.toml, global settings.toml; add a structural test if any path is missed (use devs:rust-dev agent)
- [ ] T417 Verify symlink refusal extends to project markers and harness rules-file targets per Phase 3 PR-F discipline (T134 covered the rules-file path; this audit confirms project marker writes also refuse symlinks) (use devs:rust-dev agent)
- [ ] T418 Verify the credential-scrubber extends per NFR-105: summariser model URLs, harness MCP config paths in error messages, workspace paths in MCP log lines (Phase 3 P8 PR-F's scrubbing for workspace paths in MCP logs is now exercised on the central log path) (use devs:rust-dev agent)
- [ ] T419 [GIT] Commit: `fix(security): Phase 4 hardening audit (file modes, symlink refusal, credential scrubbing)`

### PR-F — Documentation

- [ ] T420 Update `README.md` with a Phase 4 section covering `tome workspace use`, the harness commands, the central architecture, the summariser, and the v1.3.0 §Paths amendment (use devs:rust-dev agent)
- [ ] T421 Update `CHANGELOG.md` with the Phase 4 entry naming: all new commands (8 workspace + 6 harness); 8 new exit codes (13–20); reused-variant table (FR-602); 2 new direct deps (`llama-cpp-2`, `toml_edit`); `serde_json` feature addition (`preserve_order`); the central-architecture refactor (single root, single DB, named workspaces, project binding pointers); the constitution v1.3.0 §Paths amendment (use devs:rust-dev agent)
- [ ] T422 Verify every Phase 4 command's `--help` text is accurate against the contracts; update inline doc strings on the `#[arg(...)]` and `#[command(...)]` attributes as needed (use devs:rust-dev agent)
- [ ] T423 Bump `Cargo.toml` version to `0.4.0` (use devs:rust-dev agent)
- [ ] T424 [GIT] Commit: `docs: README + CHANGELOG + help text for v0.4.0`

### PR-G — Final mapping + retro extraction

- [ ] T425 Run `/sdd:map incremental` to refresh codebase docs against the full Phase 4 surface — 4 mappers in parallel (use devs:rust-dev agent)
- [ ] T426 Review `retro/P8.md` and extract critical learnings to CLAUDE.md (conservative — universal patterns only) (use devs:rust-dev agent)
- [ ] T427 [GIT] Commit: `docs(codebase): final refresh for Phase 4`

### PR-H — Ready-for-merge gate

- [ ] T428 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T429 [GIT] Update PR with Polish summary + manual SC-120 validation note (Phase 4 CI matrix: macOS, Linux, WSL2-on-WSL-filesystem; the WSL2 entry may need a one-off manual confirmation run in the closeout PR body if not in the standard PR CI matrix)
- [ ] T430 [GIT] Verify all CI checks pass — binary size step asserts ≤ 50 MB; `cargo-deny` passes with new dep rows; clippy + tests green on macOS + Linux
- [ ] T431 [GIT] Report PR ready status (output: `**PR #<N> READY FOR MERGE. AWAITING LGTM**` + PR URL)

---

## Dependencies & Execution Order

### Phase dependencies

- **Phase 1 (Setup)** — no dependencies; can start immediately.
- **Phase 2 (Foundational, F1–F10)** — depends on Phase 1; BLOCKS every user story. F1 (constitution amendment) must land first; F2a (Paths reshape) before any code touching paths; F3 (error variants) pre-allocates so user-story slices don't need to add variants; F4 (atomic_dir helper) before US1/US2 use it; F6/F7/F8 (skeletons) before user stories implement them; F9 (schema migration) before any code that depends on schema v2 being canonical; F10 (`WorkspaceName`) before any command that takes a workspace name.
- **Phase 3 (US1 — bind a project)** — depends on Foundational (F2a Paths, F3 errors, F4 atomic_dir, F7 harness skeleton, F8 settings skeleton, F10 WorkspaceName). Slices US1.a → US1.b → US1.c → US1.d are strictly sequential.
- **Phase 4 (US2 — workspace lifecycle)** — depends on Foundational (F4 atomic_dir, F6 summariser stub, F9 schema, F10 WorkspaceName). Independent of US1 but tests against US1's bind flow.
- **Phase 5 (US3 — layered settings + harness commands)** — depends on Foundational (F7 harness skeleton, F8 settings skeleton). The four remaining production harness modules (codex, gemini, cursor, opencode) land in US3.c. US1 strongly recommended first (the `claude_code` module lands there).
- **Phase 6 (US4 — summarisation)** — depends on Foundational (F6 summariser skeleton). The production `LlamaSummariser` replaces the F6 stub in non-test code in US4.a.
- **Phase 7 (US5 — doctor extensions)** — depends on US1 + US2 + US3 + US4 (covers every Phase 4 subsystem). The `Subsystem` enum promotion in US5.a depends on every subsystem name being final.
- **Phase 8 (Polish)** — depends on every preceding phase.

### User story independence (parallel team strategy)

After Foundational completes:
- Developer A: US1 (the headline; biggest single body of work; ends with one harness wired)
- Developer B: US2 in parallel (workspace lifecycle is largely independent of bind)
- Developer C: US3.a + US3.b in parallel (settings + composition is pure compute)
- Developer D: US4 (summariser; depends only on F6 skeleton)
- Developer E: US5 starts after US1+US2+US3+US4 settle (covers every subsystem)

### Parallel opportunities (within a slice)

- F2a tasks T022, T023, T024 — different methods on `Paths`, all parallelisable [P].
- F3 tasks T032–T036 — different variants on `TomeError`, sequential because each adds a row to an exhaustive `match`.
- F6 tasks T052–T056 — different files inside `src/summarise/`, all parallelisable [P].
- F7 tasks T068 (five stub harness files) — all parallelisable [P].
- F8 tasks T073, T074, T075 — different files in `src/settings/`, all parallelisable [P].
- US3.c task T274 — five harness module impls in parallel.
- US5.a tasks T366, T367 — different files, parallelisable [P].

### Within each user story

- Models / types before services (F3 errors → F6/F7/F8 modules → US slices that consume them).
- Library API before CLI wrapper (US1.b harness::sync_for_project_root before US1.a CLI command finalisation; US3.a resolver before US3.c command surface).
- Library tests before CLI binary tests (StubSummariser tests before LlamaSummariser tests; StubHarness tests before claude_code tests).
- Per slice, contract first → implementation → tests (matches Phase 3 discipline).

---

## Parallel Example: F7 harness skeleton stubs

```bash
# Launch the five harness module stubs in parallel — different files, no cross-dependencies:
Task: "Create src/harness/claude_code.rs as an empty stub returning unimplemented!()"
Task: "Create src/harness/codex.rs as an empty stub returning unimplemented!()"
Task: "Create src/harness/gemini.rs as an empty stub returning unimplemented!()"
Task: "Create src/harness/cursor.rs as an empty stub returning unimplemented!()"
Task: "Create src/harness/opencode.rs as an empty stub returning unimplemented!()"
```

## Parallel Example: US3.c production harness modules

```bash
# Once F7's skeleton + US1.c's claude_code impl land, the remaining four impls are parallelisable:
Task: "Implement src/harness/codex.rs per research §R-8 row"
Task: "Implement src/harness/gemini.rs per research §R-8 row"
Task: "Implement src/harness/cursor.rs per research §R-8 row"
Task: "Implement src/harness/opencode.rs per research §R-8 row"
```

---

## Implementation Strategy

### MVP first (US1 only)

1. Complete Phase 1 (Setup).
2. Complete Phase 2 (Foundational F1–F10 — CRITICAL, blocks every user story).
3. Complete Phase 3 (US1 — `tome workspace use <name>` with Claude Code wired).
4. **STOP AND VALIDATE**: Bind a real project to a workspace, launch Claude Code, confirm `search_skills` resolves against the workspace's enabled plugin. This is SC-104 + SC-110.
5. If green, US1 ships as v0.3.5 (interim release) — MVP delivered; the rest of the phase rolls out incrementally.

### Incremental delivery

1. Setup + Foundational → branch is on a working trunk, no user-visible change yet (refactor only).
2. US1 (bind) → MVP. Ship.
3. US2 (workspace lifecycle) → workspaces creatable, inspectable, removable.
4. US3 (layered settings + remaining 4 harnesses) → harnesses are first-class.
5. US4 (summarisation) → MCP tool descriptions become workspace-aware.
6. US5 (doctor extensions) → operability.
7. Polish → release v0.4.0.

Each phase concludes with a PR, CI green, and the standard "ready for merge" gate.

### Parallel team strategy

After Foundational completes:
- Developer A: US1 (highest-value, most contained; ends with Claude Code wired).
- Developer B: US2 (workspace lifecycle; largely independent).
- Developer C: US3 (settings + composition + 4 remaining harnesses).
- Developer D: US4 (summarisation; depends only on F6 skeleton).
- Developer E: US5 (doctor extensions; sequenced after the other four).

---

## Notes

- **[P] tasks** = different files, no dependencies on incomplete tasks. Commit at slice end, not per-task, except where a `[GIT]` task explicitly marks a commit point.
- **[Story]** label maps the task to the user story it implements; carry it through commit messages where useful for traceability.
- **[GIT]** tasks bracket each phase. They are mandatory.
- Each user story is independently completable and testable against the contracts.
- The four-reviewer parallel pass runs at every user-story close per research §R-18 (P10 retro recommendation) — applied as the closeout commits of each US phase, not just the final Polish phase.
- The constitution v1.3.0 amendment (T018–T021) is the very first Foundational commit; every subsequent slice depends on the new §Paths Operational Constraint being in effect.
- Binary-size cap (50 MB per NFR-101) is enforced by the existing CI step; T008 records the Phase 1 baseline; T414 records the Phase 4 final.
- Sync-boundary discipline (NFR-103) — no `tokio` / `async fn` / `.await` outside `src/mcp/` — is enforced by `tests/sync_boundary.rs` (Phase 3); no exemption needed for `src/summarise/` because `llama-cpp-2` is sync.
- The forward-looking `tests/no_directories_imports.rs` (T028) is a guard against future re-introduction of the `directories` crate, not a regression net for a Phase 4 removal (per research §R-1's framing correction — `directories` was never a Tome dependency).
- Phase 3 deferred items (per research §R-17) are folded into Phase 4 slices: read-only DB refactor (F2a / T030); MCP Input length caps (US5 / T373); `fabricate_models` rename (F6 / T059); `subsystem` enum promotion (US5.a / T363); drop synthetic `SuggestedFix` injection (F9 / T085); byte-progress callback (F6 / T058).
- Out of scope for Phase 4 (per research §R-17 second list): `tome workspace prune`; `Paths.config_file` rename (resolved by the F2a wholesale Paths reshape); manual SC-001/SC-002 against real BGE models; T093/T094/T095 MCP integration tests.
- Manual SC-104 (real bind + harness end-to-end) cannot run fully in CI; it ships as a documented manual gate in the PR body for US1's closeout PR (T173).

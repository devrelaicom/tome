---
description: "Phase 3 implementation tasks — MCP server, workspaces, and doctor"
---

# Tasks: Phase 3 — MCP Server, Workspaces, and Doctor

**Input**: Design documents from `/specs/003-phase-3-mcp-workspaces/`
**Prerequisites**: plan.md (✓), spec.md (✓), research.md (✓), data-model.md (✓), contracts/ (✓ 10 files), quickstart.md (✓)
**Tests**: This project uses TDD. Library-API tests + integration tests are part of each user story's slices.

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story. The slice structure per user story matches research §R-15.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- **[GIT]**: Git workflow task at a phase boundary
- All Rust implementation tasks reference `devs:rust-dev` agent
- All file paths are repository-root-relative

## Branch & feature

- **Feature number**: `003`
- **Feature slug**: `phase-3-mcp-workspaces`
- **Branch**: `003-phase-3-mcp-workspaces`

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Branch hygiene, dependency wiring, profile updates, structural-test scaffolding.

### Phase Start

- [X] T001 [GIT] Verify on main branch and working tree is clean
- [X] T002 [GIT] Pull latest changes from origin/main
- [X] T003 [GIT] Create feature branch: `003-phase-3-mcp-workspaces`

### Implementation

- [X] T004 Add `rmcp` dependency to `Cargo.toml` under `[dependencies]` (use devs:rust-dev agent)
- [X] T005 Add `tokio` dependency to `Cargo.toml` with features `["rt", "macros", "io-std", "sync", "signal", "time"]` and `default-features = false` (use devs:rust-dev agent)
- [X] T006 Extend `deny.toml` with licence rows for `rmcp`, `tokio`, and their direct transitives (use devs:rust-dev agent) — `cargo deny check` passes with no new exceptions; all transitive licences fall under the existing allowlist.
- [X] T007 Add `tests/sync_boundary.rs` — structural test that greps `src/` for `async fn`, `.await`, `tokio::`, and `tokio_` outside `src/mcp/`; fails if any match found (use devs:rust-dev agent)
- [X] T008 Run `cargo build --release` and confirm stripped binary size is < 50 MB; record the value in a comment at the top of `tests/sync_boundary.rs` (use devs:rust-dev agent) — baseline 20.91 MiB (21,922,336 bytes) on macOS arm64.
- [X] T009 Update `.gitignore` to include `specs/003-phase-3-mcp-workspaces/scratch/` if any local scratch work is needed (use devs:rust-dev agent) — generalised to `specs/*/scratch/`.
- [X] T010 [GIT] Commit: chore(deps): add rmcp + tokio scoped to src/mcp/

### Phase Completion

- [X] T011 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [X] T012 [GIT] Create PR to main with Phase 1 summary — PR #43.
- [X] T013 [GIT] Verify all CI checks pass — 6/6 green after CI fix at commit `945db1f` (Swatinem/rust-cache `cache-bin: false`).
- [X] T014 [GIT] Report PR ready status — squash-merged as commit `65291e9` on main.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Build the cross-cutting `Scope` infrastructure, the new error variants, the read-only DB open refactor, the `query::run_with_deps` library entry point, the `state_dir` path resolver, the populated `apply_pending` migrator, and the workspace-resolution algorithm. Every user story depends on this phase.

**⚠️ CRITICAL**: No user story work can begin until this phase is complete.

### Phase Start

- [X] T015 [GIT] Verify working tree is clean before starting Phase 2
- [X] T016 Create `specs/003-phase-3-mcp-workspaces/retro/P2.md` from the standard retro template
- [X] T017 [GIT] Commit: docs(retro): initialise Phase 3 / P2 retro

### Slice F1 — `Scope` type + paths refactor

- [X] T018 Create `src/workspace/mod.rs` exposing the module surface (use devs:rust-dev agent)
- [X] T019 [P] Create `src/workspace/scope.rs` with `Scope`, `ScopeSource`, `ResolvedScope` per data-model.md §1 (use devs:rust-dev agent)
- [X] T020 [P] Add `state_dir`, `mcp_log`, `mcp_log_prev`, `workspace_registry` fields to `Paths` in `src/paths.rs` and resolve via `directories::ProjectDirs::state_dir()` with XDG fallback per research §R-6 (use devs:rust-dev agent) — used the existing raw-env-var + HOME-fallback pattern instead of adding `directories` as a single-call dependency.
- [ ] T021 [P] Rename `Paths.config_file` → `Paths.global_config_file`, `Paths.index_db` → `Paths.global_index_db`, `Paths.index_lock` → `Paths.global_index_lock` in `src/paths.rs` (mechanical rename, all call sites updated in slice F4) (use devs:rust-dev agent) — **DEFERRED to slice F4** so the working tree stays compiling across slice boundaries; F4 will rename + sweep call sites in one commit.
- [X] T022 [P] Add `Paths::config_file(&Scope)`, `Paths::index_db(&Scope)`, `Paths::index_lock(&Scope)`, `Paths::workspace_marker_dir(&Path)` accessor methods in `src/paths.rs` (use devs:rust-dev agent) — landed as `config_file_for`, `index_db_for`, `index_lock_for`, `workspace_marker_dir` (the `_for` suffix avoids the field-name collision until F4 renames the fields and drops the suffix).
- [X] T023 Add `lib.rs` re-export for `workspace::{Scope, ScopeSource, ResolvedScope}` (use devs:rust-dev agent)
- [X] T024 Add `tests/paths_phase3.rs` covering `state_dir` resolution under set / unset `XDG_STATE_HOME`, and `Paths::index_db(&Scope::Workspace(path))` returning `path/.tome/index.db` (use devs:rust-dev agent)
- [X] T025 [GIT] Commit: feat(workspace): introduce Scope type and per-scope Paths accessors

### Slice F2 — closed-error-set extension

- [X] T026 Add eight new variants to `TomeError` in `src/error.rs` per contracts/exit-codes-p3.md: `McpStartupFailed`, `McpProtocolIo`, `WorkspaceMalformed`, `WorkspaceNotFound`, `WorkspaceConflict`, `SchemaVersionTooNew`, `SchemaMigrationFailed`, `DoctorFixNotSafe` (use devs:rust-dev agent)
- [X] T027 Extend `TomeError::exit_code()` exhaustive match with codes 60 / 61 / 70 / 71 / 72 / 73 / 74 / 75 (use devs:rust-dev agent)
- [X] T028 Extend `TomeError::category()` exhaustive match with the eight new category strings per contracts/exit-codes-p3.md (use devs:rust-dev agent)
- [X] T029 Extend `tests/exit_codes.rs::build_each_variant` and the exhaustive `_code_for` arm to cover the eight new variants (use devs:rust-dev agent)
- [X] T030 Extend `tests/error_messages.rs` with one Display assertion per new variant per contracts/exit-codes-p3.md §Display messages (use devs:rust-dev agent)
- [X] T031 [GIT] Commit: feat(error): add Phase 3 TomeError variants and exit codes

### Slice F3 — workspace resolution

- [X] T032 [P] Create `src/workspace/resolution.rs` implementing `resolve(args: &GlobalScopeArgs) -> Result<ResolvedScope, TomeError>` per contracts/workspace-resolution.md (use devs:rust-dev agent)
- [X] T033 [P] Add `GlobalScopeArgs { workspace: Option<PathBuf>, global: bool }` to `src/cli.rs` with `global = true` + `conflicts_with = "global"`; wire as global flags accepted on every command (use devs:rust-dev agent) — `conflicts_with` deliberately NOT used; clap's usage-error exit code (2) doesn't match the contract's required exit 72 (`WorkspaceConflict`). The resolver detects both-set and returns the dedicated error.
- [X] T034 [P] Create `src/workspace/inventory.rs` reading the opt-in `${state_dir}/workspaces.txt` registry; returns `Vec<PathBuf>` (use devs:rust-dev agent)
- [X] T035 Wire `workspace::resolution::resolve` into `src/main.rs` immediately after `Cli::parse()` (and after the pre-parse `--version` hook); pass the `ResolvedScope` into every command's `run()` (signature update across `src/commands/*` is mechanical and lives in slice F4) (use devs:rust-dev agent) — resolution runs in main.rs; the `ResolvedScope` is computed and held in a `let _ = ` placeholder until F4 threads it through.
- [X] T036 Add debug logging line per contracts/workspace-resolution.md §Debug logging in `src/workspace/resolution.rs` (use devs:rust-dev agent)
- [X] T037 Create `tests/workspace_resolution.rs` covering: CWD walk first-hit-wins, env var override, `--workspace` flag override, `--global` flag override, mutually-exclusive flags return exit 72, env-var-points-nowhere returns exit 71, malformed `.tome/config.toml` returns exit 70, nested-workspace-wins (use devs:rust-dev agent) — landed 11 cases; **malformed-config exit 70 deferred to F4's per-command tests** because resolution itself doesn't load config (it's loaded on first command access, where exit 70 emerges).
- [X] T038 [GIT] Commit: feat(workspace): resolution algorithm + global CLI flags

### Slice F4 — every command takes Scope (mechanical refactor)

- [ ] T039 Update every command's `run()` signature in `src/commands/{catalog,plugin,models,query,reindex,status}/**.rs` to take `&ResolvedScope` (use devs:rust-dev agent)
- [ ] T040 Update `src/config.rs` to expose `load_for_scope(paths: &Paths, scope: &Scope)` and `save_for_scope(paths: &Paths, scope: &Scope, config: &Config)` (use devs:rust-dev agent)
- [ ] T041 Update `src/catalog/store.rs` to honour `Scope` on every load / save call site (use devs:rust-dev agent)
- [ ] T042 Update `src/index/db.rs::open` and `src/index/lock.rs::acquire_lock` to take per-scope paths (use devs:rust-dev agent)
- [ ] T043 Verify every existing test in `tests/` still passes against the refactored signatures (no behaviour change — only signature plumbing; the workspace-specific tests come in US3) (use devs:rust-dev agent)
- [ ] T044 [GIT] Commit: refactor(commands): plumb ResolvedScope through every command surface

### Slice F5 — read-only DB open refactor (folded P10 deferral)

- [ ] T045 Add `index::open_read_only(paths: &Paths, scope: &Scope) -> Result<Connection, TomeError>` using `OpenFlags::SQLITE_OPEN_READ_ONLY | SQLITE_OPEN_NO_MUTEX` in `src/index/db.rs` (use devs:rust-dev agent)
- [ ] T046 Update read sites — `commands::plugin::open_index_for_read`, `commands::query::run`, `commands::plugin::list::run`, `commands::plugin::show::run`, `commands::status::assemble_report` — to use `open_read_only` (use devs:rust-dev agent)
- [ ] T047 Add a unit test in `tests/index_lock.rs` confirming a read-only handle does not block a writer holding the lock and does not race with it (use devs:rust-dev agent)
- [ ] T048 [GIT] Commit: refactor(index): plumb read-only open across read paths

### Slice F6 — `query::run_with_deps` library entry point (folded P10 deferral)

- [ ] T049 Add `pub fn run_with_deps(args: QueryArgs, deps: QueryDeps, mode: Mode) -> Result<QueryOutcome, TomeError>` in `src/commands/query.rs` accepting injected `Embedder` and `Reranker` traits, mirroring `reindex::run_with_deps` shape (use devs:rust-dev agent)
- [ ] T050 Refactor `commands::query::run` to call `run_with_deps` after constructing real `FastembedEmbedder` + `FastembedReranker` (use devs:rust-dev agent)
- [ ] T051 Extend `tests/query.rs` to exercise the library API directly via `run_with_deps` + `StubEmbedder` (use devs:rust-dev agent)
- [ ] T052 [GIT] Commit: refactor(query): expose run_with_deps for library testing

### Slice F7 — populate `apply_pending` migration framework

- [ ] T053 Populate `apply_pending(conn: &mut Connection, current: u32, target: u32) -> Result<u32, TomeError>` in `src/index/migrations.rs` per contracts/schema-migration.md §Algorithm (use devs:rust-dev agent)
- [ ] T054 Define `Migration { from, to, name, apply }` struct and `const MIGRATIONS: &[Migration] = &[]` (empty) in `src/index/migrations.rs` (use devs:rust-dev agent)
- [ ] T055 Add `#[cfg(test)] thread_local!` `MIGRATIONS_OVERRIDE` injection point per contracts/schema-migration.md §Registration (use devs:rust-dev agent)
- [ ] T056 Wire `apply_pending` into `index::open` after the schema version is read (use devs:rust-dev agent)
- [ ] T057 Add tracing info events at migration start, commit, and failure per contracts/schema-migration.md §Logging (use devs:rust-dev agent)
- [ ] T058 [GIT] Commit: feat(index): populate apply_pending migration framework

### Slice F8 — MCP file log appender plumbing

- [ ] T059 [P] Create `src/mcp/mod.rs` exposing `pub fn run(scope: &ResolvedScope, paths: &Paths) -> Result<(), TomeError>` as a sync entry point (use devs:rust-dev agent)
- [ ] T060 [P] Create `src/mcp/runtime.rs::build_runtime() -> Result<Runtime, TomeError>` constructing a single-threaded `tokio::runtime::Runtime` with the feature set from research §R-2 (use devs:rust-dev agent)
- [ ] T061 [P] Create `src/mcp/log.rs` implementing a JSON-lines file appender at `paths.mcp_log`, with size-based rotation at startup per contracts/log-format.md §Rotation policy (use devs:rust-dev agent)
- [ ] T062 [P] Add a `tracing-subscriber` registry construction helper in `src/mcp/log.rs` that wires the file appender + a stderr `fatal!`-only appender; honours `TOME_LOG` / `RUST_LOG` (use devs:rust-dev agent)
- [ ] T063 Add `src/mcp/preflight.rs::run(scope: &ResolvedScope, paths: &Paths) -> Result<EmbedderHandle, TomeError>` implementing the FR-110 pre-flight checks; returns the loaded embedder (use devs:rust-dev agent)
- [ ] T064 [GIT] Commit: feat(mcp): file logging and runtime/preflight scaffolding (sync boundary respected)

### End-of-phase

- [ ] T065 Run `/sdd:map incremental` to refresh codebase docs against Phase 2 Foundational changes
- [ ] T066 Review `retro/P2.md` and extract critical learnings to CLAUDE.md (conservative — pattern-level only)
- [ ] T067 [GIT] Commit: docs(codebase): refresh after Phase 3 Foundational

### Phase Completion

- [ ] T068 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T069 [GIT] Update PR with Phase 2 summary
- [ ] T070 [GIT] Verify all CI checks pass
- [ ] T071 [GIT] Report PR ready status

**Checkpoint**: Foundation ready — user story implementation can now begin in priority order.

---

## Phase 3: User Story 1 — MCP server (Priority: P1) 🎯 MVP

**Goal**: Ship `tome mcp` — a stdio MCP server backed by the Phase 2 index, exposing `search_skills` and `get_skill` tools. The MVP increment of Phase 3: registering Tome in any compliant MCP harness produces working tool calls.

**Independent Test**: From a working Phase 2 install, register `tome mcp` in Claude Code's `.claude.json`. Inside an agent session, the agent calls `search_skills` with a natural-language query and gets ranked matches; calls `get_skill` with one of the returned triples and gets the body + resource paths.

**Slice plan** per research §R-15:
- US1.a — Server scaffolding + tool registration
- US1.b — `search_skills` handler
- US1.c — `get_skill` handler
- US1.d — Integration tests + closeout

### Phase Start

- [ ] T072 [GIT] Verify working tree is clean before starting Phase 3 / US1
- [ ] T073 [US1] Create `specs/003-phase-3-mcp-workspaces/retro/P3.md` from the standard retro template
- [ ] T074 [GIT] Commit: docs(retro): initialise P3 retro

### Slice US1.a — server scaffolding

- [ ] T075 [US1] Create `src/mcp/server.rs` defining the `rmcp::ServerHandler` impl with the `initialize` / `list_tools` / `call_tool` boilerplate, holding a `McpState { embedder: Arc<dyn Embedder>, reranker: OnceCell<Arc<dyn Reranker>>, scope: ResolvedScope, paths: Paths }` (use devs:rust-dev agent)
- [ ] T076 [US1] Wire `src/mcp/mod.rs::run` to: build runtime → install MCP tracing subscriber → run preflight → construct `McpState` → call `rmcp::serve_server(state, stdio())` via `runtime.block_on(...)` → translate errors per contracts/mcp-server.md §"Startup pre-flight failure modes" (use devs:rust-dev agent)
- [ ] T077 [US1] Add `Command::Mcp(McpArgs)` variant in `src/cli.rs` and dispatch in `src/main.rs` to `commands::mcp::run`; thin CLI wrapper at `src/commands/mcp.rs` (use devs:rust-dev agent)
- [ ] T078 [US1] Create `src/mcp/tools/mod.rs` with `pub fn register(handler: &mut ToolRouter, state: Arc<McpState>)` stub that registers both tools (handler bodies in subsequent slices) (use devs:rust-dev agent)
- [ ] T079 [US1] Add `tests/mcp_lifecycle.rs` covering: startup ok, startup with missing index → exit 35, startup with embedder identity mismatch → exit 41, startup with missing embedder file → exit 30, schema-too-new → exit 73, `--workspace` + `--global` → exit 72 (use devs:rust-dev agent)
- [ ] T080 [GIT] Commit: feat(mcp): server scaffolding + lifecycle exit-code surface

### Slice US1.b — `search_skills` handler

- [ ] T081 [US1] Create `src/mcp/tools/search_skills.rs` with `SearchSkillsInput` + `SearchSkillsOutput` types per data-model.md §7 and contracts/mcp-tools.md (use devs:rust-dev agent)
- [ ] T082 [US1] Implement the handler: validate `plugin` requires `catalog`; resolve filter against the workspace's config (return `unknown_catalog` / `unknown_plugin` per contracts/mcp-tools.md); embed query; lazy-load reranker on first call; KNN with `top_k × 4` candidates; rerank; trim to `top_k`; return (use devs:rust-dev agent)
- [ ] T083 [US1] Reuse `commands::query::run_with_deps` (from F6) where the pipeline overlaps; do NOT duplicate the KNN+rerank logic (use devs:rust-dev agent)
- [ ] T084 [US1] Register the tool with the description from contracts/mcp-tools.md §search_skills, gated under `#[tool(description = "...")]` (or equivalent) (use devs:rust-dev agent)
- [ ] T085 [US1] Add log events at info level per contracts/log-format.md §Event taxonomy for `search_skills` (use devs:rust-dev agent)
- [ ] T086 [GIT] Commit: feat(mcp): search_skills tool handler

### Slice US1.c — `get_skill` handler

- [ ] T087 [US1] Create `src/mcp/tools/get_skill.rs` with `GetSkillInput` + `GetSkillOutput` types (use devs:rust-dev agent)
- [ ] T088 [US1] Implement the handler: resolve `(catalog, plugin, name)` against the enabled-skills index; return `unknown_catalog` / `unknown_plugin` / `unknown_skill` per contracts/mcp-tools.md; read SKILL.md; strip frontmatter using `plugin::frontmatter::strip`; walk the skill directory non-recursively collecting siblings; return the triple `{ content, path, resources }` (use devs:rust-dev agent)
- [ ] T089 [US1] Register the tool with the description from contracts/mcp-tools.md §get_skill (use devs:rust-dev agent)
- [ ] T090 [US1] Add log events per contracts/log-format.md (use devs:rust-dev agent)
- [ ] T091 [GIT] Commit: feat(mcp): get_skill tool handler

### Slice US1.d — integration tests + closeout

- [ ] T092 [US1] Create `tests/mcp_server.rs` covering: tool list contains exactly two tools, search_skills returns ranked hits against a fixture index, search_skills filters apply pre-rerank, search_skills unknown_catalog error, get_skill happy path, get_skill unknown_skill error, get_skill skill_file_missing error, descriptions do NOT contain plugin / skill substrings from fixtures (use devs:rust-dev agent)
- [ ] T093 [US1] Create `tests/mcp_protocol_purity.rs` — spawn the binary, drive a minimal MCP handshake over real pipes, capture stdout, assert every byte parses as MCP protocol (use devs:rust-dev agent)
- [ ] T094 [US1] Create `tests/mcp_latency.rs` (release-mode-only) — assert `search_skills` p50 < 300 ms / p99 < 600 ms against a ~100-skill fixture index (use devs:rust-dev agent)
- [ ] T095 [US1] Add SIGINT graceful-shutdown coverage in `tests/mcp_lifecycle.rs` per FR-112 (5 s timeout for in-flight calls) (use devs:rust-dev agent)
- [ ] T096 [GIT] Commit: test(mcp): server + protocol-purity + latency suites

### End-of-phase

- [ ] T097 [US1] Run `/sdd:map incremental` to refresh codebase docs against Phase 3 / US1 changes
- [ ] T098 [US1] Review `retro/P3.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T099 [GIT] Commit: docs(codebase): refresh after Phase 3 / US1

### Phase Completion

- [ ] T100 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T101 [GIT] Update PR with US1 summary
- [ ] T102 [GIT] Verify all CI checks pass
- [ ] T103 [GIT] Report PR ready status

**Checkpoint**: US1 is fully functional and independently testable. Stop and validate against a real harness before continuing to US2.

---

## Phase 4: User Story 2 — Workspace creation + autodetect (Priority: P2)

**Goal**: Ship `tome workspace init` and `tome workspace info`. Workspaces exist; the developer can create one and inspect the resolved scope.

**Independent Test**: From an existing Phase 2 install, `tome workspace init` in a fresh directory creates `.tome/`. `tome workspace info` from inside that directory or any subdirectory reports the workspace. From outside, reports global. `--inherit-global` seeds the catalog list without seeding enablement.

**Slice plan** per research §R-15:
- US2.a — resolution algorithm (already done in Foundational F3) + `tome workspace info`
- US2.b — `tome workspace init`
- US2.c — closeout

### Phase Start

- [ ] T104 [GIT] Verify working tree is clean before starting Phase 4 / US2
- [ ] T105 [US2] Create `retro/P4.md` from the standard retro template
- [ ] T106 [GIT] Commit: docs(retro): initialise P4 retro

### Slice US2.a — `tome workspace info`

- [ ] T107 [P] [US2] Create `src/commands/workspace/mod.rs` with the `WorkspaceCommand` dispatcher per data-model.md §11 (use devs:rust-dev agent)
- [ ] T108 [P] [US2] Create `src/commands/workspace/info.rs::run(scope: &ResolvedScope, paths: &Paths, mode: Mode) -> Result<(), TomeError>` per contracts/workspace-info.md (use devs:rust-dev agent)
- [ ] T109 [US2] Define `WorkspaceInfo` struct in `src/workspace/mod.rs` per data-model.md §4 (or factor out) (use devs:rust-dev agent)
- [ ] T110 [US2] Wire `Command::Workspace(WorkspaceCommand::Info)` in `src/cli.rs` and `src/main.rs` (use devs:rust-dev agent)
- [ ] T111 [US2] Create `tests/workspace_info.rs` covering: global scope reports global state; workspace scope reports workspace state; not-yet-bootstrapped DB reported informationally; `--json` output is byte-stable (use devs:rust-dev agent)
- [ ] T112 [GIT] Commit: feat(workspace): tome workspace info command

### Slice US2.b — `tome workspace init`

- [ ] T113 [US2] Create `src/workspace/init.rs::init(path: &Path, inherit_global: bool, force: bool, paths: &Paths) -> Result<InitOutcome, TomeError>` per contracts/workspace-init.md (use devs:rust-dev agent)
- [ ] T114 [US2] Create `src/commands/workspace/init.rs::run(args: WorkspaceInitArgs, paths: &Paths, mode: Mode) -> Result<(), TomeError>` (use devs:rust-dev agent)
- [ ] T115 [US2] Wire `Command::Workspace(WorkspaceCommand::Init)` in `src/cli.rs` (use devs:rust-dev agent)
- [ ] T116 [US2] Implement atomic `.tome/` creation per contracts/workspace-init.md §Atomicity using `tempfile::TempDir::persist` (use devs:rust-dev agent)
- [ ] T117 [US2] Implement `--inherit-global` to copy the global config's `[catalogs]` block into the new workspace config; do NOT copy enablement state (enablement lives in the index DB, not config) (use devs:rust-dev agent)
- [ ] T118 [US2] Implement the opt-in workspace registry append in `src/workspace/inventory.rs::append_if_registry_exists(path)` invoked by init (use devs:rust-dev agent)
- [ ] T119 [US2] Create `tests/workspace_init.rs` covering: happy path; `--inherit-global` seeds catalogs without enablement; pre-existing `.tome/` without `--force` returns exit 4; `--force` replaces atomically; non-existent `<path>` returns exit 7; concurrent init contention is rejected (use devs:rust-dev agent)
- [ ] T120 [GIT] Commit: feat(workspace): tome workspace init command

### End-of-phase

- [ ] T121 [US2] Run `/sdd:map incremental` to refresh codebase docs against Phase 4 / US2 changes
- [ ] T122 [US2] Review `retro/P4.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T123 [GIT] Commit: docs(codebase): refresh after Phase 4 / US2

### Phase Completion

- [ ] T124 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T125 [GIT] Update PR with US2 summary
- [ ] T126 [GIT] Verify all CI checks pass
- [ ] T127 [GIT] Report PR ready status

**Checkpoint**: Workspaces exist as a creatable, inspectable concept. US1 + US2 both work; US1 still operates on the resolved scope.

---

## Phase 5: User Story 3 — Every existing command honours workspace (Priority: P2)

**Goal**: Catalog management, plugin enable/disable, query, reindex, status — all honour the resolved `Scope`. Inside a workspace, they mutate workspace state. With `--global`, they mutate global state. Reference-counted catalog cleanup across scopes.

**Independent Test**: From a workspace with one catalog added, enabling a plugin doesn't affect the global install. From the global scope with `--global` from inside the workspace, mutations land globally. Removing the last reference to a catalog URL across all known scopes triggers the on-disk clone cleanup.

**Slice plan** per research §R-15:
- US3.a — per-command semantic refactor (each command writes to the resolved scope's config/DB)
- US3.b — reference-counted catalog clone cleanup
- US3.c — cross-product integration tests + closeout

### Phase Start

- [ ] T128 [GIT] Verify working tree is clean before starting Phase 5 / US3
- [ ] T129 [US3] Create `retro/P5.md` from the standard retro template
- [ ] T130 [GIT] Commit: docs(retro): initialise P5 retro

### Slice US3.a — per-command semantic refactor

- [ ] T131 [P] [US3] Update `src/commands/catalog/add.rs::run` to write to `paths.config_file(&scope)` rather than the global config file (use devs:rust-dev agent)
- [ ] T132 [P] [US3] Update `src/commands/catalog/remove.rs::run` to read / mutate the resolved scope's config (use devs:rust-dev agent)
- [ ] T133 [P] [US3] Update `src/commands/catalog/list.rs::run` to list the resolved scope's catalogs (use devs:rust-dev agent)
- [ ] T134 [P] [US3] Update `src/commands/catalog/show.rs::run` to read the resolved scope's config (use devs:rust-dev agent)
- [ ] T135 [P] [US3] Update `src/commands/catalog/update.rs::run` to refresh the resolved scope's catalogs and reindex the resolved scope's enabled plugins (use devs:rust-dev agent)
- [ ] T136 [P] [US3] Update `src/commands/plugin/{enable,disable,list,show,interactive}.rs` to operate against the resolved scope's index DB (use devs:rust-dev agent)
- [ ] T137 [P] [US3] Update `src/commands/query.rs` to operate against the resolved scope's index DB (use devs:rust-dev agent)
- [ ] T138 [P] [US3] Update `src/commands/reindex.rs` to operate against the resolved scope's index DB (use devs:rust-dev agent)
- [ ] T139 [P] [US3] Update `src/commands/status.rs` to report the resolved scope's state per contracts/workspace-info.md and Phase 2 status contract (use devs:rust-dev agent)
- [ ] T140 [P] [US3] Update `src/plugin/lifecycle.rs` enable / disable / reindex_plugin / cascade_disable_for_catalog / auto_disable_orphan to accept the resolved scope's `Paths`-derived files (use devs:rust-dev agent)
- [ ] T141 [US3] Bootstrap-on-first-write: ensure `commands::plugin::enable::run` creates `<workspace>/.tome/index.db` (and its parent) before opening if absent (use devs:rust-dev agent)
- [ ] T142 [US3] Bootstrap-on-first-write: ensure `commands::catalog::add::run` creates `<workspace>/.tome/config.toml` if absent (workspace exists but config was deleted) (use devs:rust-dev agent)
- [ ] T143 [GIT] Commit: refactor(commands): every command honours resolved Scope

### Slice US3.b — reference-counted catalog clone cleanup

- [ ] T144 [US3] Implement `catalog::store::reference_count(url: &str, paths: &Paths) -> Vec<Scope>` per contracts/catalog-extensions-p3.md §Reference-counting algorithm; reads the global config plus every workspace path in the inventory registry (use devs:rust-dev agent)
- [ ] T145 [US3] Update `src/commands/catalog/remove.rs::run` to call `reference_count` after writing the new config; if the result is empty, `fs::remove_dir_all(cache_path)` best-effort (use devs:rust-dev agent)
- [ ] T146 [US3] Update `src/plugin/lifecycle.rs::cascade_disable_for_catalog` to honour the same reference-count check before returning (the cascade is followed by a remove call that runs the cleanup; document the ordering) (use devs:rust-dev agent)
- [ ] T147 [US3] Add a doc comment on `reference_count` describing the TOCTOU profile per contracts/catalog-extensions-p3.md §Concurrency (use devs:rust-dev agent)
- [ ] T148 [GIT] Commit: feat(catalog): reference-counted catalog clone cleanup across scopes

### Slice US3.c — cross-product integration tests

- [ ] T149 [US3] Create `tests/workspace_commands.rs` covering the cross-product: from inside a workspace without overrides, `catalog add` / `catalog remove` / `plugin enable` / `plugin disable` / `query` / `reindex` / `status` each affect only the workspace; with `--global` they affect only the global state (use devs:rust-dev agent)
- [ ] T150 [US3] Create `tests/catalog_cache_refcount.rs` covering: adding the same URL in two scopes produces one on-disk clone; removing from one scope leaves the clone; removing from both scopes (and global) removes the clone; concurrent-remove benign race (use devs:rust-dev agent)
- [ ] T151 [US3] Extend `tests/catalog_remove_cascade.rs` to verify cascade-with-reference-count: catalog removed in scope A cascades only A's plugins, leaves scope B's plugins for the same URL untouched, and does NOT remove the clone if B still references it (use devs:rust-dev agent)
- [ ] T152 [GIT] Commit: test(workspace): per-command scope honouring + catalog refcount

### End-of-phase

- [ ] T153 [US3] Run `/sdd:map incremental` to refresh codebase docs against Phase 5 / US3 changes
- [ ] T154 [US3] Review `retro/P5.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T155 [GIT] Commit: docs(codebase): refresh after Phase 5 / US3

### Phase Completion

- [ ] T156 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T157 [GIT] Update PR with US3 summary
- [ ] T158 [GIT] Verify all CI checks pass
- [ ] T159 [GIT] Report PR ready status

**Checkpoint**: Workspaces are first-class. Every Phase 1/2 command honours the resolved scope. US1/2/3 work end-to-end.

---

## Phase 6: User Story 4 — `tome doctor` (Priority: P3)

**Goal**: Ship `tome doctor [--fix] [--json]`. Reports every subsystem (workspace context, models, index, drift, catalog caches, harnesses), classifies overall health, lists suggested fixes, and (with `--fix`) performs the three safe automatic repairs.

**Independent Test**: Healthy install → exits 0, all green. Remove a model → reports missing, exits 1, suggested fix lists `tome models download`. With `--fix`, the model is re-downloaded and the report re-runs as healthy. Same for a broken catalog cache (re-clone) and an older-schema DB (forward migration).

**Slice plan** per research §R-15:
- US4.a — `assemble_report` library + per-subsystem check fns + harness detection
- US4.b — `--fix` repair implementations + integration tests
- US4.c — `--json` form + human form polish + closeout

### Phase Start

- [ ] T160 [GIT] Verify working tree is clean before starting Phase 6 / US4
- [ ] T161 [US4] Create `retro/P6.md` from the standard retro template
- [ ] T162 [GIT] Commit: docs(retro): initialise P6 retro

### Slice US4.a — report assembly + checks

- [ ] T163 [P] [US4] Create `src/doctor/mod.rs` exposing the module surface and re-exports (use devs:rust-dev agent)
- [ ] T164 [P] [US4] Create `src/doctor/report.rs` defining `DoctorReport`, `CatalogCacheHealth`, `CatalogCacheState`, `HarnessPresence`, `DoctorClassification`, `SuggestedFix` per data-model.md §5 (use devs:rust-dev agent)
- [ ] T165 [P] [US4] Create `src/doctor/checks.rs` with one function per subsystem: `check_models`, `check_index`, `check_drift`, `check_catalogs`, `check_harnesses`, each returning a finding plus a suggested-fix list (use devs:rust-dev agent)
- [ ] T166 [P] [US4] Create `src/doctor/harness_detect.rs` per research §R-7 — probes for `~/.claude/`, `~/.codex/`, `~/.cursor/`, `~/.gemini/`, `~/.opencode/`, `~/.continue/` (use devs:rust-dev agent)
- [ ] T167 [US4] Implement `pub fn assemble_report(scope: &ResolvedScope, paths: &Paths, verify: bool) -> Result<DoctorReport, TomeError>` in `src/doctor/mod.rs` per the library-bypass pattern from Phase 8 (mirrors `commands::status::assemble_report`) (use devs:rust-dev agent)
- [ ] T168 [US4] Implement classification: `Unhealthy` if embedder missing/corrupt OR index integrity fail OR embedder drift OR schema-too-new; `Degraded` if reranker missing/corrupt OR reranker drift OR catalog cache broken OR orphan clone; else `Ok` (use devs:rust-dev agent)
- [ ] T169 [US4] Add unit tests in `src/doctor/checks.rs` for each per-subsystem check function against mutated fixture state (use devs:rust-dev agent)
- [ ] T170 [GIT] Commit: feat(doctor): report assembly + per-subsystem checks (library API)

### Slice US4.b — `--fix` + CLI surface

- [ ] T171 [US4] Create `src/doctor/fixes.rs::apply(report: &mut DoctorReport, paths: &Paths) -> Result<(), TomeError>` performing the three safe repair classes per contracts/doctor.md §`--fix` semantics (use devs:rust-dev agent)
- [ ] T172 [US4] Implement re-download repair via `embedding::download::download_model` (use devs:rust-dev agent)
- [ ] T173 [US4] Implement re-clone repair via `catalog::git::Git::clone` against the recorded URL and pinned ref (use devs:rust-dev agent)
- [ ] T174 [US4] Implement forward-migration repair via `index::migrations::apply_pending` under the resolved scope's advisory lock (use devs:rust-dev agent)
- [ ] T175 [US4] Implement `--fix` re-classification — after each repair, re-run the affected `check_*` function and update the in-place report (use devs:rust-dev agent)
- [ ] T176 [US4] Implement the `DoctorFixNotSafe` exit-75 return path when `--fix` was passed but the report ends with un-fixable issues remaining (use devs:rust-dev agent)
- [ ] T177 [US4] Create `src/commands/doctor.rs::run(args: DoctorArgs, scope: &ResolvedScope, paths: &Paths, mode: Mode) -> Result<(), TomeError>` (use devs:rust-dev agent)
- [ ] T178 [US4] Wire `Command::Doctor(DoctorArgs)` in `src/cli.rs` and dispatch in `src/main.rs` (use devs:rust-dev agent)
- [ ] T179 [US4] Implement human form rendering in `src/doctor/mod.rs::emit_human` per contracts/doctor.md §Output (human) (use devs:rust-dev agent)
- [ ] T180 [US4] Implement `--json` form rendering per contracts/doctor.md §Output (`--json`) (use devs:rust-dev agent)
- [ ] T181 [GIT] Commit: feat(doctor): --fix repairs + CLI surface

### Slice US4.c — integration tests + closeout

- [ ] T182 [US4] Create `tests/doctor.rs` covering: healthy report exits 0; every failure class (missing model, corrupt model, missing catalog cache, broken catalog cache, schema-older, schema-newer, embedder drift, reranker drift, orphan clone) is detected and classified; `--fix` repairs the three safe classes and re-runs as healthy; `--fix` with un-fixable issues returns exit 75; harness detection finds well-known directories; `--global` from inside a workspace reports global state (use devs:rust-dev agent)
- [ ] T183 [US4] Create `tests/doctor_json.rs` covering the JSON envelope shape end-to-end per data-model.md §5 (use devs:rust-dev agent)
- [ ] T184 [GIT] Commit: test(doctor): subsystem coverage + --fix repair classes

### End-of-phase

- [ ] T185 [US4] Run `/sdd:map incremental` to refresh codebase docs against Phase 6 / US4 changes
- [ ] T186 [US4] Review `retro/P6.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T187 [GIT] Commit: docs(codebase): refresh after Phase 6 / US4

### Phase Completion

- [ ] T188 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T189 [GIT] Update PR with US4 summary
- [ ] T190 [GIT] Verify all CI checks pass
- [ ] T191 [GIT] Report PR ready status

**Checkpoint**: Doctor reports every subsystem and repairs the safe classes. US1/2/3/4 all work end-to-end.

---

## Phase 7: User Story 5 — Forward schema migrations (Priority: P3)

**Goal**: Exercise the schema-migration framework (already populated in Phase 2 Foundational) against synthetic older-version and newer-version fixture databases. No real migration ships — Phase 4+ adds the first.

**Independent Test**: A `tests/fixtures/older-schema.db` opened by current Tome is forward-migrated and the schema-version row is updated. A `tests/fixtures/newer-schema.db` is refused with exit 73. Injected mid-sequence failure leaves the last-good intermediate version intact.

**Slice plan** per research §R-15:
- US5.a — fixtures + e2e tests
- US5.b — closeout

### Phase Start

- [ ] T192 [GIT] Verify working tree is clean before starting Phase 7 / US5
- [ ] T193 [US5] Create `retro/P7.md` from the standard retro template
- [ ] T194 [GIT] Commit: docs(retro): initialise P7 retro

### Slice US5.a — fixtures + tests

- [ ] T195 [US5] Create the fixture builder `tests/common/mod.rs::write_index_db_with_schema_version(path, version)` (use devs:rust-dev agent)
- [ ] T196 [US5] Generate `tests/fixtures/older-schema.db` recording `meta.schema_version = 0` (use devs:rust-dev agent)
- [ ] T197 [US5] Generate `tests/fixtures/newer-schema.db` recording `meta.schema_version = 99` (use devs:rust-dev agent)
- [ ] T198 [US5] Create `tests/schema_migration_e2e.rs` covering the four cases from contracts/schema-migration.md §Testing strategy: forward migration succeeds; multi-step migration succeeds; mid-sequence failure leaves last-good intermediate; newer-on-disk refused with exit 73 (use devs:rust-dev agent)
- [ ] T199 [US5] Extend `tests/atomicity.rs` with a forward-migration interrupt case — SIGINT mid-transaction leaves the schema-version row unchanged (use devs:rust-dev agent)
- [ ] T200 [US5] Verify `tome doctor --fix` runs the forward migration end-to-end against a fixture; assert the post-fix re-classification shows healthy (use devs:rust-dev agent)
- [ ] T201 [GIT] Commit: test(schema): forward-migration e2e + atomicity + doctor --fix interaction

### End-of-phase

- [ ] T202 [US5] Run `/sdd:map incremental` to refresh codebase docs against Phase 7 / US5 changes
- [ ] T203 [US5] Review `retro/P7.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T204 [GIT] Commit: docs(codebase): refresh after Phase 7 / US5

### Phase Completion

- [ ] T205 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T206 [GIT] Update PR with US5 summary
- [ ] T207 [GIT] Verify all CI checks pass
- [ ] T208 [GIT] Report PR ready status

**Checkpoint**: All five user stories shipped. Phase 3 feature work complete.

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Multi-agent review pass (per P10 retro: "Run the multi-agent review at every phase boundary"), reviewer-flagged fixups, deferred coverage items, documentation, and CHANGELOG entries.

### Phase Start

- [ ] T209 [GIT] Verify working tree is clean before starting Phase 8 / Polish
- [ ] T210 Create `retro/P8.md` from the standard retro template
- [ ] T211 [GIT] Commit: docs(retro): initialise P8 polish retro

### Multi-agent review

- [ ] T212 Dispatch four reviewers in parallel against the Phase 3 source: contract audit (`review-compact` against contracts/), Rust-lens code review (`devs:code-reviewer`), test audit (custom prompt covering coverage gaps and test-quality), security audit (`midnight-cq:cq-reviewer` equivalent). Collate findings into `review/findings.md` and triage in `review/disposition.md`
- [ ] T213 Triage findings per the P10 mapping table: blockers → dedicated PR; majors → dedicated PR; minors → folded into this phase; nits → wontfix unless trivial-while-nearby
- [ ] T214 [GIT] Commit: docs(review): Phase 3 review findings + disposition

### Reviewer-flagged fixups (placeholders — final list depends on T212-T213)

- [ ] T215 Apply any blocker-class fixups identified in T212. Slice as needed; conventional commit per fix (use devs:rust-dev agent)
- [ ] T216 Apply any major-class fixups identified in T212 (use devs:rust-dev agent)
- [ ] T217 [GIT] Commit: fix: reviewer-flagged Phase 3 fixups (squash multiple commits if appropriate per the slice)

### Deferred coverage items

- [ ] T218 Extend `tests/error_messages.rs` with Display assertions for the remaining Phase 2 `TomeError` variants flagged in P10 retro (10 variants) (use devs:rust-dev agent)
- [ ] T219 Add `ModelManifest` strictness grep guard to `tests/manifest_strictness.rs` covering `src/embedding/registry.rs` (P10 deferred item) (use devs:rust-dev agent)
- [ ] T220 Add CLI-binary JSON-envelope schema test for `tome catalog update --json` per P10 deferred item (use devs:rust-dev agent)
- [ ] T221 [GIT] Commit: test: close P10 deferred coverage items

### Documentation

- [ ] T222 Update `README.md` with a Phase 3 section covering `tome mcp`, workspaces, and `tome doctor` (use devs:rust-dev agent)
- [ ] T223 Update `CHANGELOG.md` with the Phase 3 entry naming all new commands, the eight new exit codes, and the two new dependencies (use devs:rust-dev agent)
- [ ] T224 Verify every Phase 3 command's `--help` text is accurate against the contracts; update inline doc strings on the `#[arg(...)]` and `#[command(...)]` attributes as needed (use devs:rust-dev agent)
- [ ] T225 Bump `Cargo.toml` version to `0.3.0` (use devs:rust-dev agent)
- [ ] T226 [GIT] Commit: docs: README + CHANGELOG + help text for v0.3.0

### Final mapping

- [ ] T227 Run `/sdd:map incremental` to refresh codebase docs against the full Phase 3 surface
- [ ] T228 Review `retro/P8.md` and extract critical learnings to CLAUDE.md (conservative)
- [ ] T229 [GIT] Commit: docs(codebase): final refresh for Phase 3

### Phase Completion

- [ ] T230 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T231 [GIT] Update PR with Polish summary + manual SC-101 validation note (SC-101 requires a real harness; document the run in the PR body)
- [ ] T232 [GIT] Verify all CI checks pass
- [ ] T233 [GIT] Report PR ready status

---

## Dependencies & Execution Order

### Phase dependencies

- **Phase 1 (Setup)** — no dependencies; can start immediately.
- **Phase 2 (Foundational)** — depends on Phase 1; BLOCKS every user story.
- **Phase 3 (US1)** — depends on Foundational (esp. F1 Scope, F2 errors, F5 read-only DB, F6 query library, F8 MCP logging plumbing).
- **Phase 4 (US2)** — depends on Foundational (esp. F1 Scope, F3 resolution).
- **Phase 5 (US3)** — depends on Foundational (F1 Scope, F4 mechanical refactor); US2 strongly recommended first (workspaces must exist for US3 to be meaningful).
- **Phase 6 (US4)** — depends on Foundational + benefits from US3 (doctor enumerates per-scope state).
- **Phase 7 (US5)** — depends on Foundational (F7 migration framework); independent of US1-US4.
- **Phase 8 (Polish)** — depends on every preceding phase.

### User story independence

US1 (MCP) is the headline value; ship it first and stop / validate against a real harness. US2 + US3 together make workspaces real. US4 is operability. US5 is preventive plumbing.

If team capacity permits parallel work after Foundational:
- One developer on US1 (MCP) — biggest single body of work.
- One developer on US2 + US3 in sequence (workspace).
- One developer on US4 (doctor) — depends on US3 for full coverage but US4.a can begin against global-only state.
- One developer on US5 (migration tests) — fully independent.

### Parallel opportunities (within a slice)

- Foundational F1 tasks T019, T020, T021, T022 — different files, all parallelisable [P].
- Foundational F8 tasks T059, T060, T061, T062 — same.
- US1.a tools-mod scaffolding is sequential because both tools register on the same router.
- US3.a tasks T131-T140 — different files, all parallelisable [P]. This is the largest parallel batch in the plan.
- US4.a tasks T163-T166 — different files, all parallelisable [P].

### Within each user story

- Models / types before services.
- Library API before CLI wrapper.
- Library tests before CLI binary tests.
- Per slice, contract first → implementation → tests.

---

## Parallel Example: US3.a refactor

```bash
# Launch the per-command refactor in parallel — different files, no cross-dependencies:
Task: "Update src/commands/catalog/add.rs to write to Paths::config_file(&scope)"
Task: "Update src/commands/catalog/remove.rs to read/mutate the resolved scope's config"
Task: "Update src/commands/catalog/list.rs to list the resolved scope's catalogs"
Task: "Update src/commands/plugin/enable.rs to operate against the resolved scope's index DB"
Task: "Update src/commands/plugin/disable.rs to operate against the resolved scope's index DB"
Task: "Update src/commands/query.rs to operate against the resolved scope's index DB"
Task: "Update src/commands/reindex.rs to operate against the resolved scope's index DB"
Task: "Update src/commands/status.rs to report the resolved scope's state"
```

---

## Implementation Strategy

### MVP first (US1 only)

1. Complete Phase 1 (Setup).
2. Complete Phase 2 (Foundational — CRITICAL, blocks every user story).
3. Complete Phase 3 (US1 — `tome mcp` with both tools).
4. **STOP AND VALIDATE**: Register `tome mcp` in Claude Code or Codex and confirm `search_skills` + `get_skill` are invocable from an agent session. This is SC-101.
5. If green, US1 ships as v0.2.5 (interim release) — MVP delivered, workspaces stays Phase 4+.

### Incremental delivery

1. Setup + Foundational → branch is on a working trunk, no user-visible change yet.
2. US1 (MCP) → MVP. Ship.
3. US2 (workspace init/info) → workspaces creatable, inspectable. No automatic semantic change to existing commands.
4. US3 (every command honours scope) → workspaces are first-class.
5. US4 (doctor) → operability.
6. US5 (migration framework exercised) → preventive plumbing.
7. Polish → release.

Each phase concludes with a PR, CI green, and the standard "ready for merge" gate.

### Parallel team strategy

After Foundational completes:
- Developer A: US1 (highest-value, most contained).
- Developer B: US2 then US3 (workspace track).
- Developer C: US4 (doctor track; depends on US3 for coverage of workspace-scoped reports but US4.a can start against global only).
- Developer D: US5 (independent; can start any time after Foundational F7).

---

## Notes

- **[P] tasks** = different files, no dependencies. Commit at slice end, not per-task.
- **[Story]** label maps task to the user story it implements; carry it through commit messages where useful for traceability.
- **[GIT]** tasks bracket each phase. They are mandatory.
- Each user story is independently completable and testable against the contracts.
- Phase 8 multi-agent review honours the P10 retro's "run the review at every phase boundary, not just the end" recommendation — applied here as the final pass before v0.3.0 ships.
- Avoid: vague tasks, same-file conflicts on parallel tracks, cross-story dependencies that break US1's MVP independence.
- The constitution's sync-only invariant is enforced by `tests/sync_boundary.rs` (T007). Any PR that violates the invariant fails CI before review.
- Binary-size cap (50 MB) is enforced by the existing CI step; T008 records the Phase 3 baseline.
- Manual SC-101 (MCP works in a real harness) cannot run in CI; it ships as a documented manual gate in the PR body for the closeout PR (T231).

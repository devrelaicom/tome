---
description: "Phase 2 — plugin enable/disable and local skill index"
---

# Tasks: Phase 2 — Plugin Enable/Disable and Local Skill Index

**Input**: Design documents from `specs/002-phase-2-plugins-index/`
**Prerequisites**: plan.md (required), spec.md (required), research.md, data-model.md, contracts/, quickstart.md — all present.

**Tests**: Phase 1's integration-test discipline is carried forward. Every shipped CLI command gets an integration test against real fixtures (real `TempDir`-backed SQLite, real Git fixtures, stub embedder). Test tasks are included alongside implementation tasks below; the constitution requires it (principle VIII) and the closed error-set principle requires per-exit-code coverage.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies on incomplete tasks).
- **[Story]**: `[US1]` … `[US7]` for user-story phases; absent for Setup, Foundational, and Polish.
- **[GIT]**: Git-workflow tasks (phase boundaries, commits).

## Path Conventions

Single project. `src/`, `tests/`, `vendor/`, `migrations/`, `.github/workflows/` at repository root.

## Branch

Working branch is `002-phase-2-plugins-index` (already created by `/sdd:specify`). The first-phase git task verifies we're on it and clean, rather than the "verify on main" default.

---

## Phase 1: Setup — Shared Infrastructure

**Purpose**: extend the Phase 1 build, deps, and CI to support Phase 2 without touching behaviour yet.

### Phase Start

- [x] T001 [GIT] Verify on branch `002-phase-2-plugins-index` and working tree is clean (`git branch --show-current && git status --porcelain`)
- [x] T002 [GIT] Pull origin and rebase if needed (`git fetch origin && git rebase origin/main` — abort and report on conflict)

### Setup

- [x] T003 Add Phase 2 direct dependencies to `Cargo.toml`: `rusqlite = { version = "0.31", features = ["bundled"] }`, `fastembed = "4"`, `ort = { version = "2", default-features = false, features = ["copy-dylibs"] }`, `indicatif = "0.17"`, `comfy-table = "7"`, `owo-colors = "4"`, `inquire = "0.7"`, `reqwest = { version = "0.12", default-features = false, features = ["blocking", "rustls-tls"] }` (use devs:rust-dev agent)
- [x] T004 Tune release profile in `Cargo.toml`: `lto = "thin"`, `codegen-units = 1`, `panic = "abort"`, `strip = "symbols"`, `opt-level = 3` (use devs:rust-dev agent)
- [x] T005 [P] Create `vendor/sqlite-vec/` directory and vendor pinned `sqlite-vec.c`, `sqlite-vec.h`, and `LICENSE` from the upstream release commit identified in research §R1
- [x] T006 Create `build.rs` to compile `vendor/sqlite-vec/sqlite-vec.c` against the rusqlite-bundled SQLite headers and expose it for static link (use devs:rust-dev agent)
- [x] T007 [P] Update `deny.toml` to enumerate every new transitive licence per plan.md §Operational Constraints; ensure no GPL-family appears under `cargo deny check`
- [x] T008 [P] Extend `_typos.toml` with Phase 2 vocabulary (rusqlite, sqlite-vec, fastembed, onnx, embedder, reranker, bge) so typos lint stays green
- [x] T009 Extend `.github/workflows/ci.yml` with a binary-size assertion step on Linux: `cargo build --release --locked` then fail when `target/release/tome` exceeds 10 MB (use devs:rust-dev agent for YAML)
- [x] T010 Run `cargo build --locked` and confirm the new deps compile cleanly; run `cargo deny check` and confirm green
- [x] T011 [GIT] Commit: chore(deps): add Phase 2 dependencies, vendor sqlite-vec, tune release profile

### Phase 1 Completion

- [ ] T012 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T013 [GIT] Create or update PR to `main` with Phase 1 setup summary (deps + build infra; no behaviour yet)
- [ ] T014 [GIT] Verify all CI checks pass on the setup commit
- [ ] T015 [GIT] Report PR ready status

---

## Phase 2: Foundational — Blocking Prerequisites

**Purpose**: build the cross-cutting machinery every user story depends on — error variants, paths, presentation primitives, plugin metadata parsers, index database core, embedding pipeline core (including the test stub).

**Note**: No user-facing CLI changes ship in this phase. The new modules compile and have unit tests, but no `tome plugin …` etc. commands are wired into `cli.rs` yet (that happens in US1).

### Phase Start

- [x] T016 [GIT] Verify working tree is clean before starting Phase 2 (`git status --porcelain`)
- [x] T017 Create `specs/002-phase-2-plugins-index/retro/P2.md` from the standard retro template

### Error surface

- [x] T018 Extend `src/error.rs` `TomeError` enum with the 18 Phase 2 variants enumerated in `contracts/exit-codes.md`; update `impl ExitCode for TomeError` to map each to its dedicated exit code (use devs:rust-dev agent)
- [x] T019 [P] Extend `tests/exit_codes.rs` with one case per new variant: construct each `TomeError`, assert its exit-code value matches `contracts/exit-codes.md` (use devs:rust-dev agent)
- [x] T020 [GIT] Commit: feat(error): enumerate Phase 2 error variants and exit codes

### Paths

- [x] T021 Extend `src/paths.rs` with `index_db_path()`, `index_lock_path()`, `models_dir()`, and `model_path(name)` — all under `${XDG_DATA_HOME}/tome/` per spec FR-021 (use devs:rust-dev agent)
- [x] T022 [P] Extend `tests/path_validation.rs` (or new `tests/paths_phase2.rs`) with table-driven cases for the new path resolvers (use devs:rust-dev agent)
- [x] T023 [GIT] Commit: feat(paths): add index database, lock, and models directory paths

### Presentation primitives

- [x] T024 Create `src/presentation/mod.rs` declaring `pub mod tables; pub mod progress; pub mod colour; pub mod prompt;` (use devs:rust-dev agent)
- [x] T025 [P] Implement `src/presentation/tables.rs` — `comfy-table` helpers with `NO_COLOR` / non-TTY plain-text fallback (use devs:rust-dev agent)
- [x] T026 [P] Implement `src/presentation/progress.rs` — `indicatif` wrappers that auto-suppress when stderr is not a TTY (FR-042/FR-043/FR-046) (use devs:rust-dev agent)
- [x] T027 [P] Implement `src/presentation/colour.rs` — `owo-colors` integration, `NO_COLOR` env + `--no-color` flag, auto-disable on non-TTY stdout (FR-045/FR-046) (use devs:rust-dev agent)
- [x] T028 [P] Implement `src/presentation/prompt.rs` — `inquire::Select/MultiSelect/Confirm` wrappers; entry assertion that stdout AND stdin are both terminals, otherwise return `TomeError::NotATerminal` (FR-051) (use devs:rust-dev agent)
- [x] T029 Wire `presentation` module into `src/lib.rs` re-exports (use devs:rust-dev agent)
- [x] T030 [GIT] Commit: feat(presentation): add comfy-table / indicatif / owo-colors / inquire wrappers

### Plugin metadata parsers (third-party; lenient)

- [X] T031 Create `src/plugin/mod.rs` declaring submodules and `pub use` re-exports for `PluginId`, `PluginRecord`, `PluginManifest`, `SkillFrontmatter`, `ComponentCounts` (use devs:rust-dev agent)
- [X] T032 [P] Implement `src/plugin/identity.rs` — `PluginId` struct + `FromStr` / `Display`; reject empty parts, embedded slashes, `..`, absolute paths (use devs:rust-dev agent)
- [X] T033 [P] Implement `src/plugin/manifest.rs` — lenient `plugin.json` parse via `serde_json` (no `deny_unknown_fields`); name required, others optional with sensible defaults (use devs:rust-dev agent)
- [X] T034 [P] Add `serde_yaml` (or `serde_yml`) to `Cargo.toml` and implement `src/plugin/frontmatter.rs` — lenient YAML frontmatter parse with FR-011 / FR-012 fallbacks (directory name; first 500 chars of body); distinguish header-delimiter parse failure (returns `SkillFrontmatterParseError`) from YAML-body invalid (returns sentinel for caller to log and skip per FR-013c) (use devs:rust-dev agent)
- [X] T035 [P] Implement `src/plugin/components.rs` — walk `skills/`, `agents/`, `commands/`, `hooks/`, `.mcp.json` and return `ComponentCounts` (use devs:rust-dev agent)
- [X] T036 [P] Implement `tests/frontmatter.rs` — table-driven cases: valid + extra fields, missing name, missing description, both missing, malformed delimiters, malformed YAML body (use devs:rust-dev agent)
- [X] T037 [GIT] Commit: feat(plugin): add identity, manifest, frontmatter, and component parsers

### Index database core

- [X] T038 Create `src/index/mod.rs` declaring submodules and public surface (use devs:rust-dev agent)
- [X] T039 [P] Implement `src/index/vec_ext.rs` — load the compiled-in `sqlite-vec` extension into a `rusqlite::Connection`; expose `register(&Connection) -> Result<(), TomeError>` mapping load failures to `VectorExtensionInitFailure` (use devs:rust-dev agent)
- [X] T040 [P] Implement `src/index/schema.rs` — mirror of `contracts/index-schema.sql`; expose `CREATE_STATEMENTS: &[&str]` and a `bootstrap(&mut Connection)` that runs them inside a transaction and seeds `meta` rows (schema_version=1, embedder/reranker name+version, created_at) (use devs:rust-dev agent)
- [X] T041 Create `migrations/` directory with no v1 SQL file yet (v0→v1 is bootstrap, not a migration). Implement `src/index/migrations.rs` with an empty `MIGRATIONS: &[Migration]` array and the apply-forward / refuse-backward logic per research §R3 (use devs:rust-dev agent)
- [X] T042 [P] Implement `src/index/lock.rs` — advisory write lock using `std::fs::File` + OS file locks (verify Rust 1.93's `try_lock_exclusive` availability; fall back to `fs2` if unstable). Expose `AcquireGuard` that releases on drop. Map contention to `IndexBusy` (use devs:rust-dev agent)
- [X] T043 Implement `src/index/db.rs` — `open()` resolves path via `paths::index_db_path()`, runs bootstrap or migrations, sets `PRAGMA journal_mode = WAL`, `synchronous = NORMAL`, `foreign_keys = ON`, `busy_timeout = 5000`; loads vec extension; returns a `Connection`. Maps schema-too-new to `SchemaTooNew` (use devs:rust-dev agent)
- [X] T044 [P] Implement `src/index/meta.rs` — strongly typed `MetaKey` enum, `read(&Connection, MetaKey) -> Result<Option<String>>`, `write(&mut Connection, MetaKey, &str)`, and `detect_drift(&Connection, configured_embedder, configured_reranker) -> DriftStatus` returning the variant from data-model §11 (use devs:rust-dev agent)
- [X] T045 [P] Implement `src/index/integrity.rs` — `PRAGMA integrity_check` wrapper returning structured result; maps failure to `IndexIntegrityCheckFailure` (use devs:rust-dev agent)
- [X] T046 [P] Implement `src/index/skills.rs` — CRUD: `upsert_skill`, `mark_enabled`, `mark_all_disabled_for_plugin`, `delete_by_plugin`, `list_for_plugin`, `content_hash(name, description) -> String`. Includes the `tempfile`-style transactional `enable_plugin_atomic(conn, plugin_id, skills, embedder) -> Result` that wraps embed-then-insert in one transaction per FR-004 (use devs:rust-dev agent)
- [X] T047 [P] Implement `src/index/query.rs` — `knn(conn, query_vec, top_k, catalog_filter, plugin_filter) -> Vec<Candidate>` using `sqlite-vec`'s `vec0` KNN syntax joined with `skills WHERE enabled = 1`. No reranker invocation at this layer (that's the embedding crate's job) (use devs:rust-dev agent)
- [X] T048 [P] Add `tests/index_schema_bootstrap.rs` — open fresh DB into a `TempDir`, assert schema_version=1, every `CREATE_STATEMENTS` row applied, vec extension reachable (use devs:rust-dev agent)
- [X] T049 [P] Add `tests/index_lock.rs` — two-process / two-thread test: first acquires the lock, second fails with `IndexBusy` inside the documented timeout window (use devs:rust-dev agent)
- [X] T050 [GIT] Commit: feat(index): add db, schema, migrations, lock, meta, integrity, skills, and query

### Embedding core (with #[cfg(test)] stub)

- [X] T051 Create `src/embedding/mod.rs` defining `pub trait Embedder { fn embed(&self, text: &str) -> Result<Vec<f32>, TomeError>; }` and `pub trait Reranker { fn rerank(&self, query: &str, candidates: &[Candidate]) -> Result<Vec<Scored>, TomeError>; }`. Declare submodules (use devs:rust-dev agent)
- [X] T052 [P] Implement `src/embedding/registry.rs` — `MODEL_REGISTRY: &[ModelEntry]` with two pinned entries (bge-small-en-v1.5 INT8 and bge-reranker-base INT8), each carrying name, version, source URL, SHA-256, size_bytes, licence ("MIT"), files list. Values discovered + verified in research §R5 (use devs:rust-dev agent)
- [X] T053 [P] Implement `src/embedding/download.rs` — `download_model(entry, dest_dir) -> Result<ModelManifest, TomeError>` using `reqwest::blocking` with streaming SHA-256, partial-dir + atomic rename + atomic manifest.json write per FR-020a. Honour SIGINT atomic flag (FR-053). Map checksum mismatch to `ModelChecksumMismatch`, parse errors on existing manifest to `ModelRegistrationParseError`, IO to exit 7, network to exit 7 (use devs:rust-dev agent)
- [X] T054 [P] Implement `src/embedding/runtime.rs` — `ort::Environment` lazy initialiser with CPU EP only; map init failures to `InferenceRuntimeInitFailure` (use devs:rust-dev agent)
- [X] T055 Implement `src/embedding/fastembed.rs` — `FastembedEmbedder` and `FastembedReranker` wrapping `fastembed`'s text embedder + reranker, loading models from `models_dir()`; map missing files to `ModelMissing`, load failures to `ModelCorrupt`, runtime failures to `EmbeddingGenerationFailure` / `RerankingFailure` (use devs:rust-dev agent)
- [X] T056 [P] Implement `src/embedding/stub.rs` under `#[cfg(test)]` — `StubEmbedder` returning SHA-256-derived deterministic 384-dim vectors; `StubReranker` identity; `ReverseStubReranker` reverses order. Provide a `make_test_pair()` constructor for use in tests (use devs:rust-dev agent)
- [X] T057 [P] Add `tests/model_download.rs` — point `MODEL_REGISTRY` (via `#[cfg(test)]` override or a small fixture trait) at a local HTTP server serving a synthetic file; assert success path, checksum-mismatch path, partial-rename safety, interrupt safety (use devs:rust-dev agent)
- [X] T058 [P] Add `tests/embedding_stub.rs` — assert determinism (same input → identical vector), distinguishability (different inputs → cosine < 0.99), 384-dim length (use devs:rust-dev agent)
- [X] T059 [GIT] Commit: feat(embedding): add registry, download, runtime, fastembed impl, and test stub

### Credential-scrubbing extension

- [X] T060 [P] Extend `src/catalog/git.rs::scrub_credentials` (or move to a shared `src/scrub.rs`) so it also runs over `reqwest` error chains and any URL surfaced by `embedding::download` (use devs:rust-dev agent)
- [X] T061 [P] Extend `tests/scrubbing.rs` with cases for signed-URL query strings and `https://user:token@host/` model URLs (use devs:rust-dev agent)
- [X] T062 [GIT] Commit: feat(scrub): extend credential scrubber to model download surfaces

### Phase 2 closing

- [X] T063 Run `cargo test --workspace` — all foundational tests pass. Run `cargo clippy --all-targets --all-features -- -D warnings` — green
- [X] T064 Run `/sdd:map incremental` to refresh `.sdd/codebase/*.md` against Phase 2 foundational code
- [X] T065 Review `retro/P2.md`, capture what worked / didn't / workarounds, then extract critical universal learnings to `CLAUDE.md` (conservative)
- [X] T066 [GIT] Commit: docs: refresh codebase docs and finalise Phase 2 foundational retro

### Phase 2 Completion

- [ ] T067 [GIT] Push branch to origin (ensure pre-push hooks pass)
- [ ] T068 [GIT] Update PR body with Phase 2 foundational summary (modules added, no user-facing surface yet)
- [ ] T069 [GIT] Verify all CI checks pass including the binary-size assertion
- [ ] T070 [GIT] Report PR ready status

---

## Phase 3: User Story 1 (P1) — Enable a plugin and find a skill by description

**Story goal**: from a fresh install with one catalog, the developer runs `tome plugin enable <id>` and then `tome query <text>` and sees the right skill ranked first.

**Independent test**: integration test in `tests/query.rs` enables a fixture plugin (stub embedder), runs `tome query` with a known-matching string, asserts the expected skill is top-1.

### Phase Start

- [X] T071 [GIT] Verify working tree is clean before starting Phase 3
- [X] T072 [US1] Create `specs/002-phase-2-plugins-index/retro/P3.md` from the standard retro template

### Implementation

- [X] T073 [P] [US1] Create `src/plugin/lifecycle.rs` — `enable(plugin_id, conn, embedder) -> Result<EnableSummary>` and `disable(plugin_id, conn) -> Result<DisableSummary>` per `contracts/plugin-commands.md`. Enable is atomic per plugin (one transaction wrapping embed-and-insert per FR-004); skill-header parse failures (delimiters) abort with `SkillFrontmatterParseError`, malformed YAML body skips the skill with a warning per FR-013c (use devs:rust-dev agent)
- [X] T074 [US1] Wire the model-presence prompt into lifecycle: if embedder / reranker is missing, prompt via `presentation::prompt` when TTY (download with progress, then proceed); exit `ModelMissing` when non-TTY (FR-024 / FR-025) (use devs:rust-dev agent)
- [X] T075 [US1] Create `src/commands/plugin.rs` with `enable`, `list`, `show` subcommand handlers. Wire CLI parsing via `clap` derive in `src/cli.rs`. Output via `presentation::tables` for human form, JSON via `output::Json`. Apply structured errors per `contracts/plugin-commands.md` (use devs:rust-dev agent)
- [X] T076 [GIT] Commit: feat(plugin): add lifecycle enable/disable + plugin CLI surface (enable/list/show)
- [X] T077 [P] [US1] Create `src/commands/query.rs` with the `query` subcommand. Read DB read-only; load embedder; embed query; KNN via `index::query`; rerank via `embedding::fastembed` unless `--no-rerank`; apply `--strict` + `--min-score` per contract; render via `presentation::tables`. Map drift to exit 41 / 42 (use devs:rust-dev agent)
- [X] T078 [US1] Wire `query` into `src/cli.rs` derive enum (use devs:rust-dev agent)
- [X] T079 [GIT] Commit: feat(query): add tome query with reranker + strict mode

### Tests

- [X] T080 [P] [US1] Add `tests/plugin_enable.rs` — enable a fixture plugin against a `TempDir` data dir, assert skill rows present with `enabled=1`, assert content_hash recorded, assert atomicity (interrupt → rolled back), assert idempotency exit 21 on second enable (use devs:rust-dev agent)
- [X] T081 [P] [US1] Add `tests/plugin_list.rs` — list reports plugin status / version / skill count for a registered catalog with one enabled and one disabled plugin (use devs:rust-dev agent)
- [X] T082 [P] [US1] Add `tests/plugin_show.rs` — show reports plugin metadata + component counts; tolerates third-party plugin.json with extra fields (lenient FR-013a) (use devs:rust-dev agent)
- [X] T083 [P] [US1] Add `tests/query.rs` — enable fixture plugin, run query, assert top-1 result matches expected skill (stub embedder is deterministic); assert `--json` output structure; assert `--no-rerank` flips the banner / scoring field; assert filter flags work (use devs:rust-dev agent)
- [X] T084 [P] [US1] Add `tests/atomicity_enable.rs` (or extend `tests/atomicity.rs`) — inject mid-pipeline failure via the stub embedder's `force_fail_after(n)` mode; assert no skill rows for that plugin after the failure (use devs:rust-dev agent)
- [X] T085 [P] [US1] Add a `tests/fixtures/sample-plugin/` directory with `.claude-plugin/plugin.json` + `skills/skill-a/SKILL.md` + `skills/skill-b/SKILL.md`, varied frontmatter (good, missing name, missing description, extra fields) (use devs:rust-dev agent)
- [X] T086 [GIT] Commit: test: cover plugin enable, list, show, query, and atomicity

### Closing

- [X] T087 [US1] Run `cargo test` — full suite green (187/187 across 25 suites)
- [ ] T088 [US1] Manually verify SC-001 / SC-002 against the real `bge-small-en-v1.5` / `bge-reranker-base` models on a developer machine; record numbers in `retro/P3.md` (template added; awaits developer-machine pass)
- [X] T089 [US1] Run `/sdd:map incremental` to refresh codebase docs for Phase 3 changes
- [X] T090 [US1] Review `retro/P3.md`; extract any universal learnings to `CLAUDE.md` (conservative)
- [X] T091 [GIT] Commit: docs: codebase refresh + finalise Phase 3 retro

### Phase 3 Completion

- [ ] T092 [GIT] Push branch to origin
- [ ] T093 [GIT] Update PR body with Phase 3 (MVP slice) summary
- [ ] T094 [GIT] Verify all CI checks pass
- [ ] T095 [GIT] Report PR ready status

---

## Phase 4: User Story 2 (P2) — Browse catalogs interactively

**Story goal**: `tome plugin` with no subcommand drops the developer into an interactive catalog → plugin → action flow with Back/Quit at every level.

**Independent test**: `tests/plugin_interactive.rs` drives the flow via `inquire`'s built-in test backend (or a pty harness), asserts state transitions and exit-on-non-TTY behaviour.

### Phase Start

- [X] T096 [GIT] Verify working tree is clean before starting Phase 4
- [X] T097 [US2] Create `retro/P4.md`

### Implementation

- [X] T098 [US2] Implement the interactive flow in `src/commands/plugin.rs` (`fn run_interactive()`): catalog selector → plugin browser → plugin view → action prompt. Use `presentation::prompt` wrappers; refuse on non-TTY with `NotATerminal` per FR-051. Loop with Back/Quit handling at every level (use devs:rust-dev agent)
- [X] T099 [US2] Wire the no-subcommand form into `src/cli.rs` so `tome plugin` (bare) dispatches to `run_interactive` (use devs:rust-dev agent)
- [X] T100 [GIT] Commit: feat(plugin): add interactive catalog/plugin browse flow

### Tests

- [X] T101 [P] [US2] Add `tests/plugin_interactive.rs` — drive a scripted session (select catalog → select plugin → enable → back → back → quit); assert the plugin ends up enabled in the DB and the process exited 0 (use devs:rust-dev agent)
- [X] T102 [P] [US2] Add a non-TTY case asserting bare `tome plugin` exits 54 (`NotATerminal`) with the documented pointer message (use devs:rust-dev agent)
- [X] T103 [GIT] Commit: test: cover interactive plugin flow including non-TTY refusal

### Closing

- [X] T104 [US2] Run `/sdd:map incremental` and update CLAUDE.md if needed
- [X] T105 [US2] Review `retro/P4.md`; extract critical learnings (conservative)
- [X] T106 [GIT] Commit: docs: Phase 4 codebase refresh + retro

### Phase 4 Completion

- [ ] T107 [GIT] Push branch
- [ ] T108 [GIT] Update PR body with Phase 4 summary
- [ ] T109 [GIT] Verify all CI checks pass
- [ ] T110 [GIT] Report PR ready status

---

## Phase 5: User Story 3 (P2) — Disable a plugin without losing its index

**Story goal**: `tome plugin disable <id>` flips `enabled=0` on a plugin's skill rows with confirmation; re-enable of unchanged content is essentially instant.

**Independent test**: `tests/plugin_disable.rs` disables an enabled fixture plugin, verifies it disappears from query results, re-enables it, verifies the embedder was NOT invoked (cheap re-enable per FR-006).

### Phase Start

- [X] T111 [GIT] Verify working tree is clean before starting Phase 5
- [X] T112 [US3] Create `retro/P5.md`

### Implementation

- [X] T113 [US3] Implement the `disable` subcommand handler in `src/commands/plugin.rs`; require confirmation (`presentation::prompt::confirm`) unless `--force`; non-TTY without `--force` → exit 54 per FR-007/FR-051 (use devs:rust-dev agent)
- [X] T114 [US3] Extend `plugin::lifecycle::enable` to short-circuit on cheap re-enable: compare stored `content_hash` against newly-computed; flip `enabled=1` without invoking the embedder when all hashes match (FR-006). Already partially handled in T073's design — verify the implementation matches and add the explicit cheap-path test (use devs:rust-dev agent)
- [X] T115 [GIT] Commit: feat(plugin): add disable subcommand and cheap re-enable path

### Tests

- [X] T116 [P] [US3] Add `tests/plugin_disable.rs` — disable enabled plugin, assert rows present but `enabled=0`, assert `tome query` no longer surfaces them, assert non-interactive without `--force` exits 54, assert `--force` bypasses prompt (use devs:rust-dev agent)
- [X] T117 [P] [US3] Add a cheap re-enable test (in the same file or `tests/plugin_enable.rs`) — disable, re-enable unchanged content, instrument the stub embedder to count calls, assert zero new embed calls (use devs:rust-dev agent)
- [X] T118 [P] [US3] Add a `tests/plugin_repeated.rs` case asserting exit 21 on enable-of-enabled and disable-of-disabled (FR-008) (use devs:rust-dev agent)
- [X] T119 [GIT] Commit: test: cover disable, cheap re-enable, and repeated-state idempotency

### Closing

- [X] T120 [US3] Run `/sdd:map incremental`
- [X] T121 [US3] Review `retro/P5.md`
- [X] T122 [GIT] Commit: docs: Phase 5 codebase refresh + retro

### Phase 5 Completion

- [ ] T123 [GIT] Push branch
- [ ] T124 [GIT] Update PR body
- [ ] T125 [GIT] Verify CI green
- [ ] T126 [GIT] Report PR ready status

---

## Phase 6: User Story 4 (P3) — Explicit model management commands

**Story goal**: `tome models download | list | remove` with `--force`, `--verify`, atomic install / removal, structured output.

**Independent test**: `tests/models_*.rs` exercise a stubbed HTTP server, asserting download → list → remove cycle, plus error paths (checksum mismatch, missing model state).

### Phase Start

- [X] T127 [GIT] Verify working tree is clean before starting Phase 6
- [X] T128 [US4] Create `retro/P6.md`

### Implementation

- [X] T129 [P] [US4] Implement `src/commands/models.rs` with `download` / `list` / `remove` subcommand handlers wired to `embedding::download`, `embedding::registry`, and a small reader for `manifest.json`. Apply `--verify` to switch list from cheap (existence + size) to full SHA-256. Render via `presentation::tables` (use devs:rust-dev agent)
- [X] T130 [US4] Wire `models` into `src/cli.rs` derive enum; ensure the subcommand carries `--force` (download, remove) and `--verify` (list) consistently with Phase 1 flag naming (use devs:rust-dev agent)
- [X] T131 [GIT] Commit: feat(models): add download / list / remove subcommands

### Tests

- [X] T132 [P] [US4] Add `tests/models_download.rs` — local HTTP server fixture (e.g., `hyper` or `tiny_http` already in dev-deps if not, add as dev-dep); assert success path, checksum mismatch (exit 32), partial-rename safety on simulated interrupt (FR-020a), idempotent re-run without `--force` is a no-op (use devs:rust-dev agent)
- [X] T133 [P] [US4] Add `tests/models_list.rs` — list with no models installed shows missing for both; list with one installed cleanly reports ok; list with `--verify` against a tampered file reports `ChecksumMismatched` (use devs:rust-dev agent)
- [X] T134 [P] [US4] Add `tests/models_remove.rs` — remove with confirmation prompt; `--force` bypass; non-TTY without `--force` exits 54; remove of missing model exits 30 (use devs:rust-dev agent)
- [X] T135 [GIT] Commit: test: cover models download/list/remove and error paths

### Closing

- [X] T136 [US4] Run `/sdd:map incremental`
- [X] T137 [US4] Review `retro/P6.md`
- [X] T138 [GIT] Commit: docs: Phase 6 codebase refresh + retro

### Phase 6 Completion

- [ ] T139 [GIT] Push branch
- [ ] T140 [GIT] Update PR body
- [ ] T141 [GIT] Verify CI green
- [ ] T142 [GIT] Report PR ready status

---

## Phase 7: User Story 5 (P3) — Keep the index in sync with upstream catalogs

**Story goal**: `tome catalog update` re-embeds only changed skills for enabled plugins, auto-disables plugins removed upstream, prints a clear summary. `tome reindex [scope] [--force]` provides an explicit escape hatch.

**Independent test**: `tests/catalog_update_reindex.rs` runs Git operations against a local fixture catalog, mutates a skill upstream, refreshes, asserts only the mutated skill is re-embedded (counts the stub embedder's invocations).

### Phase Start

- [X] T143 [GIT] Verify working tree is clean before starting Phase 7
- [X] T144 [US5] Create `retro/P7.md`

### Implementation

- [X] T145 [US5] Extend `src/commands/catalog.rs::update` to call `plugin::lifecycle::reindex_changed(plugin_id, conn, embedder)` for every enabled plugin in each refreshed catalog, accumulating a summary. Detect removed-upstream plugins and auto-disable them per FR-033 (use devs:rust-dev agent)
- [X] T146 [P] [US5] Add `reindex_changed` and `reindex_force` to `src/plugin/lifecycle.rs` — diff content hashes, re-embed only modified skills; force-mode re-embeds the whole scope (use devs:rust-dev agent)
- [X] T147 [GIT] Commit: feat(catalog): re-embed changed skills on update; auto-disable orphaned plugins
- [X] T148 [P] [US5] Implement `src/commands/reindex.rs` with the `reindex [scope]` subcommand wired to lifecycle functions; honour `--force`; render summary; map errors per contract (use devs:rust-dev agent)
- [X] T149 [US5] Wire `reindex` into `src/cli.rs` (use devs:rust-dev agent)
- [X] T150 [GIT] Commit: feat(reindex): add explicit reindex subcommand

### Tests

- [X] T151 [P] [US5] Add `tests/catalog_update_reindex.rs` — fixture catalog with one enabled plugin; mutate one SKILL.md upstream, refresh; assert only the modified skill was re-embedded (stub-embedder call count); assert summary table fields; assert no spurious work when nothing changed (use devs:rust-dev agent)
- [X] T152 [P] [US5] Add a removed-upstream case in the same file — delete a plugin upstream, refresh, assert auto-disable + row deletion + loud-warning stderr line (use devs:rust-dev agent)
- [X] T153 [P] [US5] Add `tests/reindex.rs` — `tome reindex` (no scope) hits every enabled plugin; `tome reindex <catalog>` scopes; `tome reindex --force` re-embeds even unchanged skills (use devs:rust-dev agent)
- [X] T154 [GIT] Commit: test: cover catalog update reindex and explicit reindex

### Closing

- [X] T155 [US5] Run `/sdd:map incremental`
- [X] T156 [US5] Review `retro/P7.md`
- [X] T157 [GIT] Commit: docs: Phase 7 codebase refresh + retro

### Phase 7 Completion

- [ ] T158 [GIT] Push branch
- [ ] T159 [GIT] Update PR body
- [ ] T160 [GIT] Verify CI green
- [ ] T161 [GIT] Report PR ready status

---

## Phase 8: User Story 6 (P3) — Verify local installation health (status + version)

**Story goal**: `tome status` reports each subsystem independently with non-zero exit on any failure; `tome --version` identifies the configured embedder and reranker.

**Independent test**: `tests/status.rs` exercises healthy, degraded (reranker-only drift), and unhealthy (model missing, schema-too-new) cases; `tests/version_output.rs` asserts the output format.

### Phase Start

- [ ] T162 [GIT] Verify working tree is clean before starting Phase 8
- [ ] T163 [US6] Create `retro/P8.md`

### Implementation

- [ ] T164 [P] [US6] Implement `src/commands/status.rs` per `contracts/status.md` — assemble a `StatusReport` from `embedding::registry`, `paths`, `index::integrity`, `index::meta`, and the configured model identities. Render human + JSON. Non-zero exit on `Degraded` / `Unhealthy` (use devs:rust-dev agent)
- [ ] T165 [US6] Wire `status` into `src/cli.rs` (use devs:rust-dev agent)
- [ ] T166 [US6] Extend the `clap` `--version` output to print Tome version + embedder name/version + reranker name/version per `contracts/version-output.md`; add a `--json` path that emits the structured form (use devs:rust-dev agent)
- [ ] T167 [GIT] Commit: feat(status,version): add tome status and extend --version with model identities

### Tests

- [ ] T168 [P] [US6] Add `tests/status.rs` — healthy / degraded (manually mutate `meta.reranker_name` then assert exit 1 + DriftStatus::RerankerDrift) / unhealthy (delete embedder file, assert ModelStatus::Missing + exit 1) (use devs:rust-dev agent)
- [ ] T169 [P] [US6] Add `tests/version_output.rs` — assert plain-text and JSON forms include all three identities (use devs:rust-dev agent)
- [ ] T170 [GIT] Commit: test: cover tome status and --version surface

### Closing

- [ ] T171 [US6] Run `/sdd:map incremental`
- [ ] T172 [US6] Review `retro/P8.md`
- [ ] T173 [GIT] Commit: docs: Phase 8 codebase refresh + retro

### Phase 8 Completion

- [ ] T174 [GIT] Push branch
- [ ] T175 [GIT] Update PR body
- [ ] T176 [GIT] Verify CI green
- [ ] T177 [GIT] Report PR ready status

---

## Phase 9: User Story 7 (P3) — Remove a catalog safely when its plugins are enabled

**Story goal**: `tome catalog remove <name>` refuses on enabled plugins (exit 53); `--force` cascades disable + row drop + catalog removal inside the lockfile boundary.

**Independent test**: `tests/catalog_remove_cascade.rs` covers the refuse case and the cascade case end-to-end.

### Phase Start

- [ ] T178 [GIT] Verify working tree is clean before starting Phase 9
- [ ] T179 [US7] Create `retro/P9.md`

### Implementation

- [ ] T180 [US7] Extend `src/commands/catalog.rs::remove` with a pre-check that queries `skills` for `enabled = 1 AND catalog = ?`. On non-empty + no `--force`, exit 53 listing the enabled plugins. On `--force`, run a cascade inside the index lockfile: disable each enabled plugin, delete its skill rows, then proceed with the Phase 1 catalog-remove logic (use devs:rust-dev agent)
- [ ] T181 [GIT] Commit: feat(catalog): refuse remove on enabled plugins; cascade with --force

### Tests

- [ ] T182 [P] [US7] Add `tests/catalog_remove_cascade.rs` — refuse case exits 53 with named-plugin message; `--force` cascade exits 0, drops rows, removes the catalog cleanly; no-enabled-plugins case behaves identically to Phase 1 (use devs:rust-dev agent)
- [ ] T183 [GIT] Commit: test: cover catalog remove cascade

### Closing

- [ ] T184 [US7] Run `/sdd:map incremental`
- [ ] T185 [US7] Review `retro/P9.md`
- [ ] T186 [GIT] Commit: docs: Phase 9 codebase refresh + retro

### Phase 9 Completion

- [ ] T187 [GIT] Push branch
- [ ] T188 [GIT] Update PR body
- [ ] T189 [GIT] Verify CI green
- [ ] T190 [GIT] Report PR ready status

---

## Phase 10: Polish & Cross-Cutting Concerns

**Purpose**: cross-cutting tests (concurrency, schema migration boundary, exit-code coverage), documentation, performance verification, final readiness.

### Phase Start

- [ ] T191 [GIT] Verify working tree is clean before starting Phase 10
- [ ] T192 Create `retro/P10.md`

### Cross-cutting tests

- [ ] T193 [P] Add `tests/concurrency.rs` — two-process scenarios: reader during writer succeeds; second writer fails fast with `IndexBusy` (exit 50); reader during a long enable does not block on the writer (use devs:rust-dev agent)
- [ ] T194 [P] Add `tests/schema_migrations.rs` — injected v0→v1 boundary: a partial migration crash leaves the DB at v0; a clean migration moves it to v1; a v2-stamped DB run against this v1-only tool exits with `SchemaTooNew` (exit 52) (use devs:rust-dev agent)
- [ ] T195 [P] Extend `tests/exit_codes.rs` to cover every code path that produces a Phase 2 exit code end-to-end (one CLI invocation per code), not just the unit-level enum→code mapping (use devs:rust-dev agent)
- [ ] T196 [P] Extend `tests/atomicity.rs` with the model-download interrupt scenario (FR-020a) using a slow / cancellable HTTP fixture (use devs:rust-dev agent)
- [ ] T197 [GIT] Commit: test: concurrency, schema migration, exit-code coverage, atomicity edges

### Documentation

- [ ] T198 [P] Update `README.md` with a Phase 2 section: enable / disable / query examples, model management, status, where things live on disk (use devs:rust-dev agent)
- [ ] T199 [P] Update `CHANGELOG.md` with the Phase 2 entry, listing user-visible additions and the new exit-code range (use devs:rust-dev agent)
- [ ] T200 [P] Refresh `CLAUDE.md` "Recent Changes" with the Phase 10 polish entry; verify "Active Technologies" still reflects shipped reality
- [ ] T201 [GIT] Commit: docs: Phase 2 README, CHANGELOG, CLAUDE.md updates

### Performance verification

- [ ] T202 Run the manual end-to-end SC-001 check on a recent laptop with real models: register a fixture catalog, `tome plugin enable` it, time the wall-clock; record in `retro/P10.md` and confirm ≤ 10 s (use devs:rust-dev agent)
- [ ] T203 Run the manual SC-002 check: enable midnight-experts/compact-expert (or equivalent), issue the canonical "how do I write a compact circuit" query, confirm relevant skill in top 3; record in `retro/P10.md`
- [ ] T204 Run `cargo build --release --locked` on both `macos-latest` and `ubuntu-latest` (locally or via CI); confirm binary stripped ≤ 10 MB; record headroom in `retro/P10.md`

### Closing

- [ ] T205 Run `cargo test --workspace --release` — full Phase 2 surface green
- [ ] T206 Run `/sdd:map incremental` one final time
- [ ] T207 Review `retro/P10.md`; extract universal learnings to `CLAUDE.md` (conservative)
- [ ] T208 [GIT] Commit: docs: final codebase refresh + Phase 2 retros

### Phase 10 Completion

- [ ] T209 [GIT] Push branch
- [ ] T210 [GIT] Update PR body with the complete Phase 2 summary (all 7 user stories shipped, success criteria evidence linked)
- [ ] T211 [GIT] Verify all CI checks pass — including binary-size assertion
- [ ] T212 [GIT] Report PR ready status

---

## Dependencies — story completion order

```
Phase 1 Setup
   │
   ▼
Phase 2 Foundational   (blocks everything below)
   │
   ▼
Phase 3 US1 Enable + Query   ──┐
   │                            │  ← P1 MVP slice; everything else extends or qualifies this
   ├─ Phase 4 US2 Interactive   │  (depends on US1 enable; no DB schema change)
   ├─ Phase 5 US3 Disable       │  (depends on US1 enable to have rows to flip)
   ├─ Phase 6 US4 Models cmds   │  (depends on Phase 2 download primitives; orthogonal to US1)
   ├─ Phase 7 US5 Update sync   │  (depends on US1 lifecycle)
   ├─ Phase 8 US6 Status        │  (depends on Phase 2 + US1; reads everything)
   └─ Phase 9 US7 Remove cascade (depends on US3 disable + Phase 2 index)
   │
   ▼
Phase 10 Polish & Cross-cutting
```

Phases 4–9 can be reordered within the P2/P3 grouping if a different ordering serves review better; the dependencies above are the minimal hard graph.

---

## Parallel execution map per phase

| Phase | Parallelizable tasks (independent files) |
|---|---|
| Phase 1 Setup | T005, T007, T008 (vendor + deny + typos config) |
| Phase 2 Foundational | T025/26/27/28 (presentation parts), T032/33/34/35 (plugin parsers), T039/40/41/42/44/45/46/47 (index submodules), T052/53/54/56 (embedding submodules), T019/22/36/48/49/57/58/61 (tests) |
| Phase 3 US1 | T077 + T073 (different files); T080/81/82/83/84/85 (tests) |
| Phase 4 US2 | T101/102 (tests) |
| Phase 5 US3 | T116/117/118 (tests) |
| Phase 6 US4 | T132/133/134 (tests) |
| Phase 7 US5 | T146/148 (lifecycle ext + reindex cmd); T151/152/153 (tests) |
| Phase 8 US6 | T164 + T166 (different files); T168/169 (tests) |
| Phase 9 US7 | T182 (test only) |
| Phase 10 Polish | T193/194/195/196 (cross-cutting tests); T198/199/200 (docs) |

---

## Independent test criteria per user story

| Story | Independent test | Acceptance |
|---|---|---|
| US1 | `tests/query.rs` end-to-end | Enable a fixture plugin, query, top-1 matches expected skill; `--json` shape matches contract |
| US2 | `tests/plugin_interactive.rs` | Scripted catalog → plugin → enable session ends with the plugin enabled; non-TTY exits 54 |
| US3 | `tests/plugin_disable.rs` + cheap re-enable test | Disable hides results; re-enable with unchanged content makes zero embedder calls |
| US4 | `tests/models_*.rs` | Download / list / remove cycle works against a local HTTP fixture; checksum mismatch exits 32 |
| US5 | `tests/catalog_update_reindex.rs` | Modifying one upstream skill triggers exactly one re-embedding call; removed plugin auto-disabled |
| US6 | `tests/status.rs` + `tests/version_output.rs` | Healthy/degraded/unhealthy cases each produce the documented exit code and structured fields |
| US7 | `tests/catalog_remove_cascade.rs` | Refuse case exits 53; `--force` cascade drops rows and removes the catalog cleanly |

---

## Implementation strategy

- **MVP scope**: Phase 1 (setup) + Phase 2 (foundational) + Phase 3 (US1). Everything else is incremental delivery.
- **Each phase ships as its own PR-ready increment**: the Phase Completion git tasks push, update the PR body, and wait for green CI before the next phase begins. The PR may be reviewed phase-by-phase or held until the full Phase 2 is complete; both flows are supported.
- **Reranker stub vs real model**: CI never downloads real models. Every PR's CI uses the `#[cfg(test)]` stub. SC-001 / SC-002 are verified manually once per phase against the real models and recorded in the phase retro. Phase 10 records the final numbers.
- **Binary-size gate** (CI step T009) is in place from Phase 1; every subsequent commit is evaluated against the 10 MB cap. If a commit breaches the cap, the contingency ladder in research §R1 applies in order (cut `inquire` features first, then `comfy-table` minimal, then `ureq` swap, etc.).
- **Constitution check post-design**: PASS with the documented embedder-stub deviation. Re-evaluate before merging Phase 10 — if any new dependency creeps in during implementation, justify it then.

---

## Validation summary

- Total tasks: **212**
- Tasks per user story phase: US1=25 (T071–T095), US2=15 (T096–T110), US3=16 (T111–T126), US4=16 (T127–T142), US5=19 (T143–T161), US6=16 (T162–T177), US7=13 (T178–T190).
- Phase totals: Setup=15, Foundational=55, US1=25, US2=15, US3=16, US4=16, US5=19, US6=16, US7=13, Polish=22.
- Parallelizable tasks: ~60 (marked `[P]`).
- Every task carries: checkbox, ID, optional `[P]` and `[Story]`/`[GIT]` labels, action verb, file path or scope, and (where code-modifying) the `devs:rust-dev` agent reference.
- Every phase has explicit phase-start (clean tree, retro), implementation, tests, codebase mapping, retro extraction, and phase-completion (push/PR/CI/ready) tasks.

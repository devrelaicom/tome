# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13 (Phase 3 User Story 1) + 2026-05-13 (Phase 4 User Story 2 — interactive browse) + 2026-05-13 (Phase 5 User Story 3 — plugin disable subcommand) + 2026-05-13 (Phase 6 User Story 4 slice 1 — models commands) + 2026-05-13 (Phase 7 User Stories 5–7 — reindex orchestrator, catalog-update cascade, explicit CLI) + 2026-05-13 (Phase 8 User Story 6 — health diagnostics) + 2026-05-14 (Phase 9 User Story 7 — catalog remove cascade) + 2026-05-14 (Foundational F7 + F8 — schema migrations framework, MCP async island)

## Architecture Overview

Tome is a synchronous Rust CLI following a classic **parse → dispatch → execute → map-errors → exit** pipeline. The codebase is organized around a **capability-driven** modular architecture where each module owns a distinct responsibility (catalog management, Git operations, configuration, logging, path resolution, output formatting, plugin metadata parsing, skill indexing, model embedding, model lifecycle management, index reconciliation, health diagnostics, interactive presentation, and the future MCP server boundary). Error handling is centralized in a closed `TomeError` enum that enforces exhaustive exit-code mapping at compile time. Signal handling (SIGINT) is global and atomic, allowing long-running operations (git clone, model download, embedding, reindexing) to be cancelled gracefully with a well-defined exit code. A forward-only schema migration framework governs index evolution with three dedicated exit codes (51 for integrity failures, 73 for schema-version-too-new on write, 74 for migration application errors). An MCP async island under `src/mcp/` provides the structural boundary anticipated by the constitution for Phase 3's server-side logic, with `tokio` scoped exclusively to that module and a test enforcement gate.

## Architecture Pattern

| Pattern | Description |
|---------|-------------|
| **Sync-only CLI** | No async runtime (`tokio`). All I/O and process orchestration use `std::process` and blocking calls. MCP server is the async island (Phase 3 future forcing function). |
| **Closed Error Set** | All failure paths map to a single `TomeError` enum with explicit exit codes; no `Other` or `Unknown` arms. Adding a failure mode requires specification, error type, and test updates. |
| **Atomic Writes** | Registry mutations, cache operations, and index writes use `tempfile` + rename for POSIX atomicity; SQLite WAL provides the index concurrency contract. Interruptions cannot corrupt state. |
| **Capability-Organized Modules** | Modules group related functionality: `catalog/` (manifest + Git + store), `commands/` (CLI handlers), `config/` (manifest deserialization), `paths/` (XDG resolution), `logging/` (tracing setup), `output/` (human/JSON formatting), `plugin/` (metadata parsing + lifecycle), `index/` (SQLite skills DB + KNN + migrations), `embedding/` (fastembed wrapper + model registry + download), `presentation/` (tables / progress / colour / prompts), `mcp/` (async server boundary + preflight). |
| **Credential Scrubbing at Boundary** | All captured `git` and `reqwest` output passes through credential scrubbing before reaching logging, error display, or structured output. |
| **Trait-based Embedding Abstraction** | `Embedder` and `Reranker` are seam interfaces; `FastembedEmbedder` wraps `fastembed-rs`, and a deterministic `StubEmbedder` (unit-test only) provides testability without model files. |
| **Plugin-Dir Resolution: Manifest-First** | `lifecycle::resolve_plugin_dir` reads `tome-catalog.toml`, looks up `id.plugin` in the declared `plugins[].name`, joins with the source; falls back to flat `entry.path.join(&id.plugin)` for backward compat when manifest is absent. Single shared function across `enable`, `disable`, `list`, `show` fixes inconsistency. |
| **Interactive Three-Level Loop Pattern** | Bare `tome plugin` (no subcommand) enters an interactive flow: `catalog_loop` → `plugin_loop` → `view_loop`, each with a `LoopExit` enum to handle Back/Quit unwinds and error propagation (Phase 4, User Story 2). |
| **Per-Plugin Atomic Reindex** | `lifecycle::reindex_plugin` mirrors `lifecycle::enable` atomicity: each plugin's reindex is one SQLite transaction under one advisory lock. Batch operations (`tome catalog update`, `tome reindex`) loop per-plugin, committing each before moving to the next. SIGINT between plugins leaves earlier plugins committed (per-plugin boundary). |
| **Lazy Embedder Loading** | Heavy embedder (~345 MB ONNX) is loaded only when reindexing will actually call it. `tome catalog update` and `tome reindex` defer load until the first enabled plugin is encountered; a sync with zero enabled plugins never touches model files. |
| **Health Diagnostics (Read-Only)** | `status` command reads subsystem state without mutation: models via manifest checks, index via read-only connection + `PRAGMA integrity_check`, drift via stored identity comparison. Never acquires advisory lock, never downloads models. Exits 0 on Ok, 1 on Degraded/Unhealthy. |
| **Single-Lock-Per-Batch Cascade** | `lifecycle::cascade_disable_for_catalog` acquires the advisory lock once, disables + drops all enabled plugins for a catalog, then releases. Different from per-plugin operations; chosen to match the contract in `specs/002-phase-2-plugins-index/contracts/catalog-extensions.md` §"tome catalog remove". |
| **Forward-Only Schema Migrations** | `src/index/migrations.rs` enforces a registration framework: `apply_pending(conn, current, target)` applies registered steps within a read lock. Three exit codes govern the migration domain: 51 (`IndexIntegrityCheckFailure` — unknown state post-migration), 73 (`SchemaVersionTooNew` — write path refuses newer-on-disk schemas), 74 (`SchemaMigrationFailed` — registered step apply error). Read path via `open_read_only` keeps legacy 52 (`SchemaTooNew`) for backward compat. Phase 7 tests inject synthetic `MIGRATIONS_OVERRIDE` via `thread_local!`. |
| **MCP Async Island Boundary** | `src/mcp/` directory is a tokio-scoped async boundary (research §R-2 pinned to current-thread runtime per Phase 3 plan). `mod.rs` provides a sync entry point `McpStartupFailed` stub (US1 pending). Preflight validates scope-resolved index state before handoff: schema gate (emits 73), drift detect, SHA-256 verify, eager-load embedder. `log.rs` wires size-based rotation (FR-227) + JSON-lines tracing registry (FR-226) + stderr-only error layer. `runtime.rs` and `preflight.rs` live inside; all other modules stay sync. Structural test `tests/sync_boundary.rs` enforces the exemption. |

## Core Components

### CLI & Parsing (`src/cli.rs`, `src/main.rs`)

- **Purpose**: Parse global flags (`--json`, `-v`/`-vv`) and dispatch to subcommand handlers.
- **Location**: `src/main.rs` (entry), `src/cli.rs` (clap derive definitions).
- **Dependencies**: `clap` (argument parsing), `catalog::git` (signal handler installation).
- **Dependents**: `commands/` modules (receive parsed args).
- **Pipeline Entry**: `main()` parses CLI → installs signal handler → dispatches to handler → maps result to exit code.
- **Phase 4 Change**: `PluginArgs` now wraps an `Option<PluginCommand>` to allow bare `tome plugin` with no subcommand. Routes to `commands::plugin::run_interactive()` when the command is `None`.
- **Phase 5 Change**: `PluginCommand` now includes `Disable(PluginDisableArgs { id: String, force: bool })` variant.
- **Phase 6 Change**: `ModelsCommand` enum added with `Download`, `List`, `Remove` variants; routes via `Command::Models(ModelsCommand)` to `commands::models::run()`.
- **Phase 7 Change**: `ReindexArgs` and `ReindexCommand` added for `tome reindex [<scope>] [--force]`; routes via `Command::Reindex(ReindexArgs)` to `commands::reindex::run()`.
- **Phase 8 Change**: `StatusArgs` added with `--verify` flag; routes via `Command::Status(StatusArgs)` to `commands::status::run()`. Pre-parse hook intercepts `--version` / `-V` BEFORE clap dispatch so extended output can include MODEL_REGISTRY identities and honour `--json` flag.

### Catalog Management (`src/catalog/`, `src/commands/catalog/`)

- **Purpose**: Orchestrate catalog registration, refresh, removal, and inspection; manage Git cloning and credential scrubbing.
- **Location**: `src/catalog/` (core logic: git, manifest, store), `src/commands/catalog/` (subcommand handlers).
- **Dependencies**: `git` (shell-outs), `manifest` (TOML parsing + validation), `store` (atomic writes), `config` (registry persistence).
- **Dependents**: Main CLI, integration tests.
- **Key Invariants**:
  - Catalogs are cached at `~/.local/share/tome/catalogs/<sha256(url)>/`.
  - Config is persisted at `~/.config/tome/config.toml` atomically.
  - Git operations capture stderr and pass it through credential scrubbing before error display.
- **Phase 7 Change**: `tome catalog update` now reindexes enabled plugins in each catalog after a Git refresh. Per-plugin atomicity: each `lifecycle::reindex_plugin` call owns its own lock. Auto-disable cascade on `PluginNotFound` / `PluginManifestParseError` via `lifecycle::auto_disable_orphan()`.
- **Phase 9 Change**: `tome catalog remove` now refuses with exit 53 (`CatalogHasEnabledPlugins`) if enabled plugins exist, unless `--force` is passed. On `--force`, calls `lifecycle::cascade_disable_for_catalog()` to drop all enabled plugin rows in one lock window, then proceeds with Phase 1 removal logic.

### Plugin Metadata & Lifecycle (`src/plugin/`, `src/commands/plugin/`)

- **Purpose**: Parse plugin manifests and SKILL.md frontmatter (lenient), manage plugin enable/disable/reindex state, orchestrate skill embedding and indexing.
- **Location**: `src/plugin/` (metadata parsers, lifecycle orchestrator), `src/commands/plugin/` (CLI handlers + interactive flow).
- **Dependencies**: `catalog::manifest` (read_catalog_manifest), `index::` (open DB, acquire lock, enable_plugin_atomic, reindex_plugin_atomic), `embedding::` (embedder + reranker, model registry, download).
- **Dependents**: Commands.
- **Key Patterns**:
  - `lifecycle::enable()`: parse manifest (exit 22) → check already-enabled (exit 31) → ensure models present (exit 30 unless allow_model_download) → acquire lock → walk skills → collect PendingSkill → embed + insert under one transaction (atomic per FR-004) → release lock.
  - `lifecycle::disable()`: check not-disabled (exit 32) → acquire lock → flip enabled=0 for all (catalog, plugin) rows → release lock. Cheap re-enable follows since embeddings are retained.
  - `lifecycle::reindex_plugin()`: walk on-disk skills → acquire lock → diff against index → re-embed modified (or all if force=true) → delete orphaned rows → release lock. Mirrors enable atomicity (Phase 7).
  - `lifecycle::auto_disable_orphan()`: called by `tome catalog update` when a plugin is not found post-refresh; de-indexes all rows for the plugin and emits a warning.
  - `lifecycle::cascade_disable_for_catalog()` (Phase 9): acquires lock once, calls `delete_by_plugin()` per plugin in the catalog, then releases. Returns total dropped skill rows. Used by `tome catalog remove --force` to cascade-disable all enabled plugins before removing the catalog. Unlike per-plugin operations, does not take a `LifecycleDeps` — the cascade is pure deletion without embedder reference.
  - Frontmatter parse: delimiter error is fatal (exit 23); YAML-body error skips one skill + warn (FR-013c).
  - Models: embedder + reranker required by enable and query; optional download in `enable` (CLI owns the TTY prompt; `lifecycle::allow_model_download` is the decision).

### Plugin Disable Subcommand (`src/commands/plugin/disable.rs`)

- **Purpose**: Thin CLI wrapper over `plugin::lifecycle::disable`; owns confirmation-prompt UX (`--force` short-circuit, non-TTY refusal with pointer message).
- **Location**: `src/commands/plugin/disable.rs` (~108 lines).
- **Public Interface**: `pub fn run(args: PluginDisableArgs, mode: output::Mode) -> Result<(), TomeError>`.
- **Flow**:
  1. Parse `PluginId` from args.
  2. Load config, verify plugin exists (fail fast before prompt).
  3. If not `--force`, check TTY (non-TTY → emit pointer line to stderr, return `NotATerminal` exit 54).
  4. TTY: prompt with default "no" per spec. User decline → clean exit Ok(()) + optional stderr note.
  5. User accept or `--force`: call `lifecycle::disable()` (returns `DisableOutcome`).
  6. Emit human or JSON output.
- **Error Semantics**: Same exit codes as `lifecycle::disable` (exit 32 if already disabled). Non-TTY without `--force` → exit 54 (`NotATerminal`).
- **Pattern**: Mirrors `enable.rs` in structure (validate → prompt → call library → emit). No embedder construction — index-only UPDATE. Cheap re-enable tested via `tests/plugin_enable.rs::cheap_reenable_after_disable_invokes_embedder_zero_times`.

### Interactive Browse Flow (`src/commands/plugin/interactive.rs`)

- **Purpose**: Bare `tome plugin` (no subcommand) — provide an interactive catalog → plugin → action browse loop (Phase 4, User Story 2).
- **Location**: `src/commands/plugin/interactive.rs` (~515 lines).
- **Public Interface**: `pub fn run_interactive(mode: output::Mode) -> Result<(), TomeError>`.
- **Loop Architecture** (three levels):
  1. **`catalog_loop()`**: Display catalog selector; user picks one or Quit.
  2. **`plugin_loop(catalog_name)`**: Browse plugins in the selected catalog; user picks one or Back.
  3. **`view_loop(id, plugin_manifest)`**: Display plugin view (mirrors `plugin show`); user selects action: Enable, Disable, or Back.
- **Control Flow**: Each loop level uses a private `LoopExit` enum to encode:
  - `Continue` → advance to next level.
  - `Back` → unwind to previous level.
  - `Quit` → clean exit with `Ok(())` (exit 0).
- **Error Handling**:
  - Enable/disable errors propagate verbatim (same exit codes as non-interactive subcommands).
  - User cancellation (Esc / Ctrl-C from `inquire` prompts) surfaces as `TomeError::Interrupted`, which is trapped and translated to `Ok(())` per the contract ("always exits 0 on clean exit").
- **TTY Enforcement** (FR-051): Non-TTY invocation via `presentation::prompt::select()` will refuse with `NotATerminal`, propagating as exit code 98 (FR-022a).

### Model Lifecycle Management (`src/commands/models/`)

- **Purpose**: Provide user-facing CLI for managing downloaded model artefacts (download, list, remove).
- **Location**: `src/commands/models/` (dispatch + per-subcommand handlers).
- **Dependencies**: `embedding::registry` (MODEL_REGISTRY, ModelEntry), `embedding::download` (download_model, sha256_file), `paths` (models_dir), `presentation` (tables, progress, prompts).
- **Dependents**: CLI main dispatcher.
- **Subcommands** (Phase 6, User Story 4):
  - **`download`** (`src/commands/models/download.rs`): Iterate MODEL_REGISTRY, skip if manifest exists and valid unless `--force`, atomic download via `embedding::download::download_model` with indicatif spinner. Emits human or NDJSON per mode.
  - **`list`** (`src/commands/models/list.rs`): Cheap path = check manifest + file existence + size; `--verify` flag rehashes via `embedding::download::sha256_file`. Renders ModelState (Ok / Missing / Corrupt / ChecksumMismatched) as table (human) or NDJSON (JSON).
  - **`remove`** (`src/commands/models/remove.rs`): Check model is not in use by any enabled plugin, check model exists (exit 30 if missing), confirm prompt with `--force` short-circuit, non-TTY without `--force` → exit 54 with pointer message. Deletes manifest first, then directory.
- **Shared Pattern**: Mirrors `plugin/` module layout (per-subcommand file under group `mod.rs` that dispatches). Owns user-facing UX (prompts, spinners, table rendering). Calls library functions (`embedding::download`, `embedding::registry`) for the actual work.

### Health & Diagnostics (`src/commands/status.rs`)

- **Purpose**: Report per-subsystem health (models, index, drift) in a read-only, non-mutating diagnostic command.
- **Location**: `src/commands/status.rs` (~330 lines).
- **Public Interfaces**:
  - `pub fn run(args: StatusArgs, mode: Mode) -> Result<(), TomeError>` — CLI entry; emits report, exits 0 on Ok, 1 on Degraded/Unhealthy.
  - `pub fn assemble_report(paths: &Paths, verify: bool) -> Result<StatusReport, TomeError>` — library entry (used by tests); produces report without exit side-effect.
  - `pub fn print_version(json: bool) -> Result<(), TomeError>` — extended --version output (called by pre-parse hook in main.rs).
- **Health Model** (`StatusReport`):
  - `tome: ModelHealth` — embedder model manifest checks + optional SHA-256 verification.
  - `embedder: ModelHealth` — embedder manifest, size, hash state.
  - `reranker: ModelHealth` — reranker manifest, size, hash state.
  - `index: IndexHealth` — `PRAGMA integrity_check` result, enabled plugin count, enabled skill count.
  - `drift: DriftStatus` — identity mismatch between stored and configured embedder/reranker.
  - `overall: OverallHealth` — classification (Ok / Degraded / Unhealthy).
- **Classification Rules**:
  - **Unhealthy**: embedder missing/corrupt OR index integrity fail OR embedder drift OR reranker drift (new in Phase 8; was silent warn before).
  - **Degraded**: reranker missing/corrupt.
  - **Ok**: all subsystems healthy.
- **Design Invariants**:
  - Read-only: opens index with `OpenOptions { readonly: true }`, never acquires advisory lock.
  - No downloads: rejects models if not on-disk (exit 30 if missing); does not prompt or auto-download.
  - Verifiable: `--verify` flag re-hashes via `embedding::download::sha256_file` (slow but thorough).
  - Compile-time model identities: MODEL_REGISTRY consts drive both --version output and drift detection; bumping a model version auto-updates both without code changes.
- **Exit Semantics**:
  - Exit 0: `overall == Ok`.
  - Exit 1: `overall == Degraded` or `overall == Unhealthy`.
  - Non-zero cases call `std::process::exit(1)` after emitting the report (not propagated as `TomeError`).

### Index Reconciliation & Reindex (`src/commands/reindex.rs`, `src/plugin/lifecycle.rs`, `src/index/skills.rs`)

- **Purpose**: Re-embed and reconcile on-disk plugin skills against the index; driven by embedder upgrades (FR-016 recovery) and integrity recovery.
- **Location**: `src/commands/reindex.rs` (CLI + scope resolution), `src/plugin/lifecycle.rs` (orchestrator), `src/index/skills.rs` (atomic reindex transaction).
- **Scope Grammar**:
  - Omitted: every enabled plugin across every registered catalog.
  - `<catalog>`: every enabled plugin in one catalog.
  - `<catalog>/<plugin>`: exactly one plugin.
- **Flow** (`tome reindex [<scope>] [--force]`):
  1. Parse and validate scope (catalog/plugin names must exist in config).
  2. Resolve target plugins (read enabled plugins from index per scope).
  3. Load embedder once (lazy pattern; skip if no enabled plugins in scope).
  4. For each target plugin:
     - Call `lifecycle::reindex_plugin(id, deps, force)`.
     - Each call acquires lock → diffs on-disk vs index → re-embeds if hash changed or force=true → deletes orphans → releases lock.
     - Per-plugin atomicity: SIGINT between plugins leaves earlier plugins committed.
  5. Emit aggregate record (human: summary line; JSON: NDJSON record).
- **Reindex Algorithm** (`reindex_plugin_atomic` in `index/skills.rs`, ~110 lines):
  - **Pass 1** — Walk pending (on-disk) skills:
    - Unchanged (content_hash match, force=false): touch metadata only (UPDATE).
    - Modified (hash mismatch or force=true): re-embed, upsert skill + embedding.
    - Added (new on-disk): embed, insert skill + embedding.
  - **Pass 2** — Walk leftover indexed rows (no longer on-disk):
    - Remove: delete skill row + embedding.
  - All under one SQLite transaction. On SIGINT, transaction rolls back.
- **Content-Hash Smart Re-Embedding** (FR-032): When a skill's `(name, description)` text composition is identical to what was indexed, Tome skips the embedder call and reuses the vector. Hash is `SHA256(name + "\n\n" + description)`.
- **Sqlite-Vec Virtual Table Workaround**: `skill_embeddings` is a `vec0` virtual table that does not support `INSERT OR REPLACE`. Upsert logic: DELETE-then-INSERT per skill ID. The DELETE is a no-op on first insert.
- **Cascade on Catalog Refresh** (Phase 7): `tome catalog update` wires `lifecycle::reindex_plugin` per enabled plugin after a Git refresh. If plugin is not found post-refresh, `lifecycle::auto_disable_orphan()` de-indexes all rows and emits a loud warning (FR-033).
- **Library Test Entry Point** (Phase 7): `src/commands/reindex::run_with_deps(scope, plugins, deps, force, mode)` allows tests to drive the reindex logic with `StubEmbedder`, keeping the CLI binary's `FastembedEmbedder` out of CI.

### Skill Query & Search (`src/index/query.rs`, `src/commands/query.rs`)

- **Purpose**: Embed the user's query text, perform KNN over enabled skills, optionally rerank, filter, and render results.
- **Location**: `src/commands/query.rs` (CLI entry and result presentation), `src/index/query.rs` (KNN SQL + filter logic).
- **Dependencies**: `index::` (open read-only, knn), `embedding::` (embedder + reranker), `catalog::manifest` (filter validation).
- **Flow**:
  1. Parse query text and filter flags (`--catalog`, `--plugin`, `--no-rerank`, `--min-score`, `--strict`).
  2. Validate filter flags against registered catalogs (cheap manifest reads).
  3. Open index read-only.
  4. Check embedder drift (exit 41/42 if stale); check reranker drift (warn-only).
  5. Check model presence (embedder always required; reranker if not `--no-rerank`).
  6. Load embedder (always); load reranker (unless `--no-rerank`).
  7. Embed query text.
  8. KNN with `candidate_k = top_k × 4` if reranking, else `top_k`.
  9. Apply reranker or cosine-similarity scoring.
  10. Trim to `top_k`, apply optional `--strict` threshold filter.
  11. Render as table (human) or NDJSON (JSON).

### Schema Migrations Framework (`src/index/migrations.rs`)

- **Purpose**: Enforce forward-only schema evolution with registration-based step application and three dedicated exit codes (Foundational F7).
- **Location**: `src/index/migrations.rs` (~120 lines).
- **Public Interface**: `pub fn apply_pending(conn: &mut Connection, current: u32, target: u32) -> Result<u32, TomeError>`.
- **Design**:
  - `MIGRATIONS` const array of `(version, name, apply_fn)` tuples. Phase 2 ships with zero registered migrations + a synthetic fixture e2e test per the plan (Phase 3 design).
  - `apply_pending(conn, current, target)` iterates registered steps between current and target, executing each within a read-lock window.
  - Three exit codes govern the domain:
    - **51** (`IndexIntegrityCheckFailure` — "DB in unknown state") on `PRAGMA integrity_check` failure post-migration.
    - **73** (`SchemaVersionTooNew` — "write-path refuses newer-on-disk schema") when on-disk schema > target.
    - **74** (`SchemaMigrationFailed` — "registered step apply error") when a registered migration step fails.
  - Read path via `open_read_only` maintains legacy 52 (`SchemaTooNew`) for backward compat.
  - Tests inject synthetic migrations via `thread_local! { MIGRATIONS_OVERRIDE }` to verify the framework end-to-end without waiting for real schema changes.
- **Key Invariant**: Migrations are idempotent (per step, check before re-apply). Failed migrations block further steps (fail-fast gate).

### MCP Async Island (`src/mcp/`)

- **Purpose**: Provide a structural boundary for Phase 3's async server-side logic (anticipated forcing function). `tokio` scoped exclusively here.
- **Location**: `src/mcp/` (four files: `mod.rs`, `runtime.rs`, `log.rs`, `preflight.rs`).
- **Files**:
  - `mod.rs` (~15 lines): Sync entry point `pub fn start_server() -> Result<(), TomeError>` currently returns `McpStartupFailed` (US1 pending). Phase 3 fills the loop.
  - `runtime.rs` (~50 lines): Current-thread tokio runtime initialization (research §R-2 pinned per Phase 3 plan). No other modules import.
  - `log.rs` (~100 lines): Size-based rotation (FR-227) wiring + JSON-lines `tracing-subscriber` registry (FR-226) + stderr-only error layer (FR-220).
  - `preflight.rs` (~120 lines): Pre-flight validation (FR-110) — scope-resolved index read-only open → schema gate (emits 73 on too-new) → drift detect → SHA-256 verify primary file → eager-load `FastembedEmbedder`. Reranker deferred per FR-109. Returns `PreflightReport`.
- **Async Boundary**:
  - Only files under `src/mcp/` use `tokio`. All other modules remain sync.
  - Entry point is sync: `main.rs` calls `mcp::start_server()` as a last resort after CLI dispatch (not yet wired; US1).
  - Structural test `tests/sync_boundary.rs` enforces: exempts every file under `src/mcp/`, gates every other module for `tokio` imports. Detects leakage.
- **Design Invariants**:
  - No MCP modules import from `commands/`, `plugin/`, `index/` — instead, Phase 3 will refactor read-side (query, plugin list/show) as shared library functions (`query::run_with_deps`, etc.).
  - Preflight is defensive: errors during validation do not crash the server; they surface as exit codes to the harness.
  - Log rotation aware of MCP's continuous nature (file size cap, not line count).

### Git Interface (`src/catalog/git.rs`)

- **Purpose**: Spawn `git` processes, scrub credentials from captured output, handle SIGINT cancellation.
- **Location**: `src/catalog/git.rs`.
- **Dependencies**: `regex` (credential patterns), `ctrlc` (signal handling), `std::process`.
- **Key Methods**:
  - `clone_shallow(url, dest, ref)`: Clone a specific branch/tag/commit.
  - `scrub_credentials(bytes)`: Apply regex rules (R-8 from research.md) to mask tokens, SSH hostnames, etc.
  - `install_signal_handler()`: Set up SIGINT handler (idempotent).
  - `was_cancelled()`: Check if SIGINT fired.
- **Signal Handling**: A global `AtomicBool` is flipped when SIGINT is received; spawned child processes are killed and `TomeError::Interrupted` (exit code 8) is returned.

### Manifest Parsing & Validation (`src/catalog/manifest.rs`)

- **Purpose**: Parse `tome-catalog.toml` (strict) and plugin `plugin.json` (lenient); validate structure and semantic constraints.
- **Location**: `src/catalog/manifest.rs`.
- **Schema Enforcement**:
  - `tome-catalog.toml`: `#[serde(deny_unknown_fields)]` on every struct; unknown fields produce `ManifestInvalid::UnknownField`.
  - `plugin.json`: lenient parsing (serde_json, unknown fields ignored) per FR-013a.
- **Validation Pipeline** (for `tome-catalog.toml`):
  1. UTF-8 decode.
  2. TOML syntax parse.
  3. Required field check (name, description, version, owner.name, owner.email).
  4. Semantic validation (semver version, valid email).
  5. Unique plugin names.
  6. Relative-path plugin sources (no `..`, no absolute paths, no URLs, must resolve within catalog).
- **Error Propagation**: Each failure produces a specific `ManifestInvalid` variant that maps to exit code 5.

### Index & Skills Database (`src/index/`)

- **Purpose**: Maintain a local SQLite skills index with vector embeddings, enable/disable state tracking, drift detection, KNN search, and forward-only schema evolution.
- **Location**: `src/index/`.
- **Concurrency**: WAL mode + 5s `busy_timeout` + optional advisory lockfile (`${XDG_DATA_HOME}/tome/index.lock`). Read-only operations (`query`, `plugin list/show`, `status`) do not take the lock; mutating operations (`plugin enable/disable/reindex`) do. Contention surfaces as `TomeError::IndexBusy` (exit 50) within milliseconds.
- **Schema**:
  - `meta`: embedder + reranker identity + drift flags.
  - `skills`: `(catalog, plugin, name, description, path, plugin_version, embedding, enabled, indexed_at, content_hash)` — content-hash column for smart re-embedding.
  - `skill_embeddings`: virtual `vec0` table (sqlite-vec); does not support `INSERT OR REPLACE`.
  - Vectors are L2-normalized 384-dim floats.
- **Migrations** (Foundational F7):
  - Framework at `src/index/migrations.rs` enforces forward-only application.
  - `apply_pending(current, target)` iterates registered steps; exits 73 on schema-too-new, 74 on step failure, 51 on post-migration integrity failure.
  - Phase 2 ships zero registered migrations (plan defers schema changes to Phase 10); tests use `MIGRATIONS_OVERRIDE` to verify the framework.
- **Key Operations**:
  - `enable_plugin_atomic()`: walk PendingSkill vec, embed each, insert under one transaction, return EnableSummary (total / newly_embedded counts).
  - `reindex_plugin_atomic()`: diff on-disk pending skills against index, re-embed modified/added, delete removed, return ReindexSummary (added / modified / removed / unchanged counts). Mirrors enable atomicity.
  - `mark_all_disabled_for_plugin()`: flip `enabled = 0` for all rows matching `(catalog, plugin)`.
  - `delete_by_plugin()` (Phase 9): delete all skill rows for a `(catalog, plugin)` pair. Used by `cascade_disable_for_catalog()` to drop enabled plugins before catalog removal.
  - `knn(query_vec, k, filters)`: search by `enabled = 1`, apply optional catalog/plugin filters, return top k candidates by cosine distance.

### Embedding & Model Registry (`src/embedding/`)

- **Purpose**: Wrap `fastembed-rs` and `ort` into a testable trait interface; manage model downloads and registry.
- **Location**: `src/embedding/`.
- **Model Registry** (`src/embedding/registry.rs`):
  - Two models pinned: `bge-small-en-v1.5` (embedder, 45 MB, INT8) and `bge-reranker-base` (reranker, 280 MB, INT8).
  - Strict `ModelManifest` JSON schema (per model's `manifest.json`); downloaded models are atomically persisted.
  - Checksums (SHA-256) validated on download; placeholder checksums rejected.
- **Embedder & Reranker Traits** (`src/embedding/mod.rs`):
  - `Embedder::embed(text: &str) -> Result<Vec<f32>>` — produces 384-dim L2-normalized vectors.
  - `Reranker::rerank(text: &str, candidates: Vec<Candidate>) -> Result<Vec<Scored>>` — cross-encoder logits.
  - `FastembedEmbedder` / `FastembedReranker`: production implementations via `ort`.
  - `StubEmbedder` / identity reranker: deterministic stubs for unit tests (no network, no files).
- **Download** (`src/embedding/download.rs`):
  - Atomic downloads: write to temp, verify checksum, rename.
  - SIGINT-aware: polls `git::was_cancelled()`.
  - Credential scrubbing on `reqwest` errors.
  - **Phase 6 addition**: `pub fn sha256_file(path) -> Result<String, TomeError>` streaming SHA-256 helper for `models list --verify`.

### Presentation Layer (`src/presentation/`)

- **Purpose**: Wrap table rendering, progress spinners, colour output, and interactive prompts with TTY awareness and `NO_COLOR` support.
- **Location**: `src/presentation/`.
- **Modules**:
  - `tables.rs`: `comfy-table` helpers; falls back to plain text on non-TTY or `NO_COLOR`.
  - `progress.rs`: `indicatif` spinners; auto-suppressed on non-TTY stderr.
  - `colour.rs`: `owo-colors` + `NO_COLOR` env + `--no-color` flag.
  - `prompt.rs`: `inquire` (Select / MultiSelect / Confirm); refuses on non-TTY with `NotATerminal` error.

### Configuration (`src/config.rs`)

- **Purpose**: Define `Config` and `CatalogEntry` structures; serialize/deserialize via `serde` + `toml`.
- **Location**: `src/config.rs`.
- **Key Types**:
  - `Config`: Top-level document; keyed by catalog display name (BTreeMap for deterministic ordering).
  - `CatalogEntry`: Name, URL, tracked ref, local path, last-synced timestamp.
- **Strict Parsing**: `#[serde(deny_unknown_fields)]` on all structs.

### Path Resolution (`src/paths.rs`)

- **Purpose**: Resolve XDG-aware configuration and data directories; compute content-addressed cache keys; resolve index DB, lock, and model paths.
- **Location**: `src/paths.rs`.
- **XDG Compliance**: Honour `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, fall back to `~/.config` and `~/.local/share`.
- **Phase 2 additions**:
  - `index_db`: `${XDG_DATA_HOME}/tome/index.db`.
  - `index_lock`: `${XDG_DATA_HOME}/tome/index.lock`.
  - `models_dir`: `${XDG_DATA_HOME}/tome/models/`.
  - `model_path(name)` / `model_manifest(name)`: resolve model directories and manifest.json files.

### Logging (`src/logging.rs`)

- **Purpose**: Initialize `tracing-subscriber` with stderr-only output, orthogonal to `--json`.
- **Location**: `src/logging.rs`.
- **Verbosity**: `-v` = info, `-vv` = debug; env vars `TOME_LOG`, `RUST_LOG` supported.
- **Key Invariant**: Logs go to stderr; primary command output (--json or human) goes to stdout. This keeps structured data uncontaminated by debug output.

### Output Formatting (`src/output.rs`)

- **Purpose**: Format results as human-readable text or machine-readable JSON; handle TTY detection and `NO_COLOR`.
- **Location**: `src/output.rs`.
- **Modes**:
  - `Mode::Human`: Friendly multiline text, colours enabled (auto-disabled on non-TTY or NO_COLOR env var).
  - `Mode::Json`: One JSON object per line (NDJSON); always valid for piping.
- **Error Handling**: Error records include category, exit code, and message; always written to stderr.
- **Phase 6 Change**: Relaxed `write_json` to accept `T: Serialize + ?Sized` for JSON serialization flexibility.

### Error Handling (`src/error.rs`)

- **Purpose**: Define the closed `TomeError` enum and map each variant to an exit code and category.
- **Location**: `src/error.rs`.
- **Variants** (21+ total, Phase 3 additions + Foundational F7):
  - Phase 1: `Internal`, `Usage`, `CatalogNotFound`, `CatalogAlreadyExists`, `ManifestInvalid`, `GitFailed`, `Io`, `Interrupted`.
  - Phase 2: `IndexIntegrityCheckFailure` (51), `IndexBusy` (50), `ModelMissing` (30), `PluginNotFound` (20), `SkillFrontmatterParseError` (23).
  - Phase 3: `PluginAlreadyInState` (31/32), `QueryNoResultsStrict`, drift checks (exit 41/42).
  - Phase 9: `CatalogHasEnabledPlugins` (exit 53).
  - **Foundational F7** (schema migration domain): `SchemaVersionTooNew` (73), `SchemaMigrationFailed` (74). Legacy `SchemaTooNew` (52) retained for read-path.
- **Compile-Time Enforcement**: The `TomeError::exit_code()` method is exhaustive; adding a variant forces edits to `tests/exit_codes.rs`, the spec, and the PRD.

## Data Flow

### Primary User Flow: `tome plugin enable <catalog>/<plugin>`

```
CLI parse (--json, -v, args)
       ↓
dispatch to plugin::enable::run()
       ↓
parse PluginId
       ↓
load config, paths
       ↓
resolve plugin directory (manifest-first)
       ↓
probe model presence + prompt user if missing (TTY only)
       ↓
load FastembedEmbedder
       ↓
lifecycle::enable(id, deps {embedder, allow_model_download=false, …})
  Step 2 → parse plugin.json (lenient)
  Step 3 → check already-enabled (exit 31)
  Step 4 → ensure models present (skipped: allow_model_download=false, already prompted)
  Step 5 → acquire advisory lock
  Step 6–9 → walk skills → collect PendingSkill (with frontmatter parse)
         → embed each (with SIGINT poll)
         → insert under one transaction
         → return EnableSummary
       ↓
format output (human: ✓ N skills / JSON: NDJSON record)
       ↓
exit(0)
```

### Plugin Disable Flow: `tome plugin disable <catalog>/<plugin>`

```
CLI parse
       ↓
dispatch to plugin::disable::run()
       ↓
parse PluginId, load config, paths
       ↓
resolve plugin directory
       ↓
check plugin exists (fail-fast before prompt)
       ↓
if not --force:
  → check TTY (non-TTY → emit pointer, return NotATerminal exit 54)
  → confirm prompt (default "no"; abort → Ok(()), no state change)
       ↓
lifecycle::disable(id, paths, config, embedder_seed, reranker_seed)
  → acquire lock
  → mark_all_disabled_for_plugin() [flip enabled=0]
  → release lock
  → return DisableOutcome (skills_retained count)
       ↓
format output (human: ✓ disabled N records / JSON: NDJSON record)
       ↓
exit(0)
```

### Catalog Remove Cascade: `tome catalog remove <name> --force`

```
CLI parse (--json, -v, name, --force)
       ↓
dispatch to catalog::remove::run()
       ↓
load config, paths
       ↓
check catalog exists (exit 15 if not)
       ↓
pre-check: read enabled plugins in this catalog (cheap, no lock)
       ↓
if enabled_plugins.len() > 0 and not --force:
  → return CatalogHasEnabledPlugins (exit 53) + plugins list
       ↓
if not --force:
  → check TTY (non-TTY → return Usage error)
  → prompt for confirmation (default "no"; abort → Ok(()), no state change)
       ↓
if !enabled_plugins.is_empty():
  → call lifecycle::cascade_disable_for_catalog(paths, catalog, plugins, seeds)
       ↓
    lifecycle::cascade_disable_for_catalog():
      → acquire lock (once per batch)
      → open index
      → for each plugin:
          → call delete_by_plugin(conn, catalog, plugin)
          → sum dropped rows
      → release lock
      → log info + return total
       ↓
delete config entry (atomic)
       ↓
delete cache directory (best-effort)
       ↓
format output (human or JSON with cascade array if present)
       ↓
exit(0)
```

### Index Reconciliation Flow: `tome reindex [<scope>] [--force]`

```
CLI parse (scope, --force flag)
       ↓
dispatch to reindex::run()
       ↓
parse and validate scope (All / Catalog / Plugin)
       ↓
resolve targets: read enabled plugins from index per scope
       ↓
if scope is Plugin, cross-check plugin exists in index (fail fast exit 20)
       ↓
for each target plugin:
  → call lifecycle::reindex_plugin(id, deps, force)
       ↓
     lifecycle::reindex_plugin():
       → parse plugin.json (exit 22 on error)
       → acquire advisory lock
       → walk on-disk skills → collect PendingSkill
       → reindex_plugin_atomic():
           ✓ Unchanged (hash match, force=false): touch metadata
           ✓ Modified (hash mismatch or force=true): re-embed
           ✓ Added (new on-disk): embed
           ✓ Removed (orphaned index rows): delete
           All under one transaction
       → release lock
       → return ReindexOutcome (ReindexSummary + warnings)
       ↓
aggregate outcomes (one per plugin)
       ↓
format output (human: summary line / JSON: NDJSON record)
       ↓
exit(0)
```

### Catalog Update with Reindex: `tome catalog update [<name>]`

```
CLI parse (--json, -v, name)
       ↓
dispatch to catalog::update::run()
       ↓
load config, paths
       ↓
for each target catalog (all or named):
  → refresh_one() — Git clone/pull, update config
       ↓
  if refresh happened (not SHA-pinned):
    → read enabled plugins in catalog from index
    → if none: skip reindex, continue
    → lazy load embedder (only after first enabled plugin found)
    → for each enabled plugin:
        → lifecycle::reindex_plugin(id, deps, force=false)
           [same as explicit `tome reindex` flow]
    → catch PluginNotFound / PluginManifestParseError:
        → lifecycle::auto_disable_orphan(id)
        → emit loud warning (FR-033)
       ↓
emit human or JSON output (summary table per catalog)
       ↓
exit(0)
```

### Health Report Flow: `tome status [--verify]`

```
CLI parse (--verify flag)
       ↓
dispatch to status::run()
       ↓
assemble_report(paths, verify=flag):
  → load registered embedder/reranker from MODEL_REGISTRY
  → for each model:
      → cheap_state: check manifest, file, size
      → if --verify: re-hash via sha256_file (slow)
      → classify ModelState (Ok / Missing / Corrupt / ChecksumMismatched)
  → open index read-only (never lock)
  → run PRAGMA integrity_check
  → query enabled plugin count, enabled skill count
  → read stored embedder/reranker ident from index meta
  → detect_drift: compare (name, version) pairs
  → classify ModelHealth (Ok / Missing / Corrupt / ChecksumMismatched)
  → classify IndexHealth (integral / drift / plugin/skill counts)
  → classify OverallHealth:
      - Unhealthy: embedder missing OR index fail OR embedder/reranker drift
      - Degraded: reranker missing
      - Ok: all healthy
  → return StatusReport
       ↓
emit_report(report, mode):
  → human: multiline summary with colour
  → JSON: single NDJSON record
       ↓
if overall != Ok:
  std::process::exit(1)
else:
  exit(0)
```

### Extended Version Output: `tome --version` (or `-V`)

```
main.rs pre-parse hook:
  → check `std::env::args` for --version / -V BEFORE clap dispatch
  → if found:
       → parse global --json flag from args
       → call status::print_version(json)
       → emit three-line human or JSON record:
           - tome version (from Cargo.toml)
           - embedder identity (name + version from MODEL_REGISTRY)
           - reranker identity (name + version from MODEL_REGISTRY)
       → exit(0)
       → (clap's auto-version handler never runs)
```

### Interactive Browse Flow: `tome plugin` (no subcommand)

```
CLI parse (bare Command::Plugin with option=None)
       ↓
dispatch to plugin::run_interactive()
       ↓
check TTY (inquire will refuse non-TTY as NotATerminal)
       ↓
catalog_loop():
  → present Select over catalog names + Quit option
  → user picks catalog or Quit
  → Quit → return Ok(()) → exit(0)
  → catalog picked → call plugin_loop(catalog_name)
       ↓
plugin_loop(catalog_name):
  → load index, walk (catalog, plugin) pairs
  → present Select over plugin names + Back option
  → user picks plugin or Back
  → Back → return to catalog_loop
  → plugin picked → call view_loop(id, manifest)
       ↓
view_loop(id, manifest):
  → render plugin view (as in `plugin show`)
  → present Select: [Enable | Disable | Back]
  → Enable → call enable::run(id)
         → on success, redraw plugin view (loop within view)
         → on error, propagate (exit with non-zero code)
  → Disable → confirm prompt, call lifecycle::disable()
         → on success, redraw plugin view
         → on error, propagate
  → Back → return to plugin_loop
       ↓
Esc / Ctrl-C at any level:
  → inquire surfaces as TomeError::Interrupted
  → trap and convert to Ok(())
  → exit(0)
```

### Model Lifecycle Flow: `tome models download | list | remove`

```
CLI parse (--json, -v, subcommand-specific args)
       ↓
dispatch to models::{download,list,remove}::run()

Download subcommand:
  → iterate MODEL_REGISTRY
  → for each model:
      → check if manifest exists && is valid
      → if missing or --force:
          → show indicatif spinner
          → call embedding::download::download_model()
          → emit human line or NDJSON record
  → exit(0)

List subcommand:
  → iterate MODEL_REGISTRY
  → for each model:
      → cheap check: manifest exists + files exist + correct sizes
      → if --verify: stream SHA-256 via sha256_file()
      → compute ModelState (Ok / Missing / Corrupt / ChecksumMismatched)
  → render comfy-table (human) or NDJSON (JSON)
  → exit(0)

Remove subcommand:
  → parse model name
  → check model exists (exit 30 if missing)
  → check no enabled plugins use it
  → if not --force: check TTY, prompt (non-TTY → exit 54 with pointer)
  → delete manifest, then directory
  → emit human line or NDJSON record
  → exit(0)
```

### Query Flow: `tome query <text>`

```
CLI parse (--top-k, --catalog, --plugin, --no-rerank, --strict, --min-score, …)
       ↓
dispatch to query::run()
       ↓
validate filters (check catalogs exist)
       ↓
open index read-only
       ↓
check embedder drift (exit 41 if stale)
       ↓
check reranker drift (warn-only)
       ↓
check model presence (embedder required; reranker if not --no-rerank)
       ↓
load embedder + reranker (with spinners)
       ↓
embed query text
       ↓
KNN (candidate_k = top_k×4 if reranking)
       ↓
apply filters (--catalog, --plugin)
       ↓
optional rerank (cross-encoder logits)
       ↓
trim to top_k
       ↓
apply --strict threshold filter (or warn)
       ↓
render table (human) or NDJSON (JSON)
       ↓
exit(0)
```

### Schema Migration Flow: `apply_pending(conn, current, target)` (Foundational F7)

```
caller (e.g., open_db after bootstrap):
  → determine current schema version (PRAGMA user_version)
  → call apply_pending(conn, current, target_version)
       ↓
apply_pending(conn, current, target):
  → if current > target:
      → return SchemaVersionTooNew (exit 73)
       ↓
  → if current == target:
      → return Ok(current)
       ↓
  → for each migration step in MIGRATIONS where version > current and version <= target:
      → execute step's apply_fn(conn)
      → on error: return SchemaMigrationFailed (exit 74)
       ↓
  → run PRAGMA integrity_check
  → if failed:
      → return IndexIntegrityCheckFailure (exit 51)
       ↓
  → update PRAGMA user_version = target
  → return Ok(target)
```

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| **CLI** (`src/main.rs`, `src/cli.rs`) | Parse args, install signal handler, dispatch, map errors to exit codes. | Commands, logging, output. | Catalog, config, paths (indirectly via commands). |
| **Commands** (`src/commands/`) | Orchestrate catalog/plugin/query/models/reindex/status operations; call library logic and format output. | Lifecycle, catalog, config, paths, error, output, embedding, index, presentation. | Logging (by design; logging is orthogonal). |
| **Plugin Lifecycle** (`src/plugin/lifecycle.rs`) | Enable/disable/reindex orchestrator; compose metadata parsers, index, and embedding. | Plugin metadata (manifest, frontmatter), index (open, lock, enable_plugin_atomic, reindex_plugin_atomic, delete_by_plugin), embedding (embedder trait, model registry), catalog (manifest reader). | Commands (reverse dependency only). |
| **Index** (`src/index/`) | SQLite operations, schema, KNN, drift detection, advisory locks, forward-only migrations. | rusqlite, sqlite-vec, index schema, migrations framework. | Commands, embedding, plugin (reverse dependency only). |
| **Embedding** (`src/embedding/`) | Model registry, download, trait implementations (fastembed wrapper, stub). | reqwest, ort, fastembed-rs, serde. | Commands, index (reverse dependency only). |
| **Catalog** (`src/catalog/`) | Git operations, manifest parsing, atomic persistence. | Git (process spawning), manifest (parsing), store (writes), config (types). | Commands (reverse dependency only). |
| **Git** (`src/catalog/git.rs`) | Spawn and manage git subprocesses; scrub credentials from all output. | `std::process`, regex, ctrlc. | Manifest, config, commands. |
| **Manifest** (`src/catalog/manifest.rs`) | Parse and validate TOML (strict) and JSON (lenient); enforce schema constraints. | serde, toml, serde_json, error types. | Git, store, commands. |
| **Store** (`src/catalog/store.rs`) | Atomic read/write of config files. | tempfile, std::fs, config types. | Git, manifest, commands. |
| **Config** (`src/config.rs`) | Define and serialize registry and catalog entry structures. | serde, toml, time (timestamps). | Catalog, commands (reverse dependency only). |
| **Paths** (`src/paths.rs`) | Resolve XDG directories and content-addressed cache keys; resolve index/model paths. | sha2, hex, std::env. | All other modules. |
| **Logging** (`src/logging.rs`) | Initialize tracing. | tracing, tracing-subscriber. | All modules (orthogonal; no dependencies into logging). |
| **Output** (`src/output.rs`) | Format results as human or JSON; detect TTY. | serde_json, std::io, error types. | No other modules (clean boundary). |
| **Presentation** (`src/presentation/`) | Table, progress, colour, prompt rendering; TTY and `NO_COLOR` awareness. | comfy-table, indicatif, owo-colors, inquire, std::io. | Commands (reverse dependency only). |
| **Error** (`src/error.rs`) | Define closed error enum and exit code mapping. | thiserror, std::path, anyhow. | No other modules (consumed by all). |
| **MCP** (`src/mcp/`) | Async server boundary, preflight validation, log rotation. | tokio, tracing (for log.rs), index (read-only open), embedding (for preflight eager-load). | No other modules depend on mcp; mcp does NOT depend on commands, plugin lifecycle (Phase 3: will refactor as library functions). |

## Dependency Rules

1. **No cycles**: The dependency graph is a DAG. `main.rs` → `cli.rs` → `commands/` → `{plugin, index, embedding, catalog, config, paths, output, presentation, error}`. MCP is a leaf module (only called at startup after CLI dispatch).
2. **Library shapes**: `plugin::lifecycle`, `index::`, and (future) `mcp::preflight` are library-shaped (no CLI); they return structured outcomes (`EnableOutcome`, `DisableOutcome`, `ReindexOutcome`, `Candidate` vec, `Scored` vec, `PreflightReport`) that `commands/` layers format for output.
3. **Trait seams**: `Embedder` and `Reranker` traits decouple the library from model implementations; tests inject `StubEmbedder`.
4. **Error type at the root**: `error.rs` has no internal dependencies; all modules depend on it (or types it wraps).
5. **Orthogonal logging**: `logging.rs` is initialized at startup and orthogonal to `--json` mode. No module imports `logging`; the global subscriber is set up once in `main()`.
6. **Config types, not logic, in `config.rs`**: `config.rs` defines only data structures; I/O is in `store.rs`.
7. **Plugin-dir resolution is centralized**: `plugin::lifecycle::resolve_plugin_dir` is the single source of truth; re-exported to CLI handlers via `commands/plugin/mod.rs` to avoid cross-boundary imports.
8. **Interactive flow is command-layer only**: `commands/plugin/interactive.rs` uses presentation layer (prompts, tables), command handlers (enable, disable), and lifecycle/config APIs. It is test-driven via `rexpect` pty harness (`tests/plugin_interactive.rs`) rather than unit-test injection.
9. **Models commands mirror plugin commands layout**: `commands/models/` follows the same per-subcommand-file + group-dispatcher pattern as `commands/plugin/`. Library-side work (download, SHA-256) lives in `embedding/download.rs` and `embedding/registry.rs`; CLI-side work (prompts, progress, tables) lives in `commands/models/{download,list,remove}.rs`.
10. **Reindex commands and lifecycle**: `commands/reindex.rs` owns scope parsing and emission; `plugin::lifecycle::reindex_plugin` and `index::skills::reindex_plugin_atomic` own the orchestration and transaction. Lazy embedder loading is a `commands/` responsibility. `run_with_deps` in `commands/reindex.rs` is a library test entry point (no embedder construction).
11. **Status is read-only by design**: `commands/status.rs` never mutates state, never acquires advisory lock, never downloads models. It opens the index with `readonly=true` and reads compiled-time model registry identities. Library function `assemble_report` is testable; CLI-side `run` exits 1 on Degraded/Unhealthy via `std::process::exit()` rather than `TomeError`.
12. **Catalog remove cascade pattern** (Phase 9): `commands/catalog/remove.rs` handles the CLI logic (prompt, force flag, error handling); `plugin::lifecycle::cascade_disable_for_catalog()` is the library function that owns the single-lock-per-batch mutation pattern. Unlike per-plugin operations, the cascade does not take a `LifecycleDeps` — it only needs paths, catalog name, plugin list, and model seeds for metadata seeding.
13. **Schema migrations are library-shaped**: `index::migrations::apply_pending()` is stateless; it reads a registration table (`MIGRATIONS` const) and applies steps sequentially. Tests inject synthetic migrations via `MIGRATIONS_OVERRIDE` without touching the main codebase. Write paths call `apply_pending` before mutation; read paths use `open_read_only` to preserve backward compat.
14. **MCP async island isolation** (Foundational F7): `src/mcp/` does NOT import from `commands/`, `plugin/`, `index/`, or any CLI-shaped module. Phase 3 will refactor read-side operations (query, plugin list/show) as shared library entry points that both CLI and MCP can call. Structural test `tests/sync_boundary.rs` enforces the exemption by scanning for `tokio` imports outside `src/mcp/`.

## Key Interfaces & Contracts

| Interface | Purpose | Implementation |
|-----------|---------|-----------------|
| `TomeError` | Closed enum of all failure modes; exit codes are exhaustive. | `src/error.rs` |
| `CatalogManifest` | Schema for `tome-catalog.toml`; enforces strict parsing and semantic validation. | `src/catalog/manifest.rs` |
| `PluginManifest` | Schema for `plugin.json`; lenient parsing (unknown fields ignored). | `src/plugin/manifest.rs` |
| `SkillFrontmatter` | Parsed YAML header from `SKILL.md`; fallback logic for name/description. | `src/plugin/frontmatter.rs` |
| `PluginId` | Address `<catalog>/<plugin>`; `FromStr` implementation. | `src/plugin/identity.rs` |
| `PluginRecord` + `PluginStatus` | Display record for a plugin + tri-state status. | `src/plugin/mod.rs` |
| `Config` + `CatalogEntry` | Registry schema; persisted to `~/.config/tome/config.toml`. | `src/config.rs` |
| `Paths` | XDG-aware path resolution; index DB, lock, model paths. | `src/paths.rs` |
| `Git` | Facade for git operations; scrubs credentials from all output. | `src/catalog/git.rs` |
| `store::write_atomic` | Atomic file write for registry and cache mutations. | `src/catalog/store.rs` |
| `Embedder` + `Reranker` | Trait interfaces for embedding and reranking. | `src/embedding/mod.rs` |
| `FastembedEmbedder` + `FastembedReranker` | ONNX-backed implementations via `fastembed-rs` and `ort`. | `src/embedding/fastembed.rs` |
| `StubEmbedder` | Deterministic test double; produces SHA-derived vectors. | `src/embedding/stub.rs` (test-only by default, LTO-stripped from release). |
| `EnableOutcome` + `DisableOutcome` + `ReindexOutcome` | Structured results of plugin lifecycle operations. | `src/plugin/lifecycle.rs` |
| `LifecycleDeps` | Dependency injection struct for `lifecycle::enable/disable/reindex_plugin`. | `src/plugin/lifecycle.rs` |
| `Candidate` + `Scored` | KNN result and scored result records. | `src/embedding/mod.rs` |
| `ReindexSummary` | Outcome breakdown of one `reindex_plugin_atomic` call (added / modified / removed / unchanged). | `src/index/skills.rs` |
| `output::Mode` | Enum selecting human or JSON formatting. | `src/output.rs` |
| `LoopExit` | Private enum in `interactive.rs` encoding Back/Quit/Continue state. | `src/commands/plugin/interactive.rs` |
| `ModelState` | Classification of a registered model's on-disk install state (Ok / Missing / Corrupt / ChecksumMismatched). | `src/commands/models/mod.rs` |
| `Scope` | Reindex scope (All / Catalog / Plugin); used by `commands/reindex.rs` and tests. | `src/commands/reindex.rs` |
| `StatusReport` + `OverallHealth` + `ModelHealth` + `IndexHealth` | Health diagnostic data model (Phase 8). | `src/commands/status.rs` |
| `DriftStatus` | Embedder/reranker identity mismatch detection. | `src/index/meta.rs` |
| `MIGRATIONS` + `apply_pending` | Forward-only schema migration framework (Foundational F7). | `src/index/migrations.rs` |
| `PreflightReport` | Pre-flight validation result (schema, drift, model availability). | `src/mcp/preflight.rs` |

## Signal Handling & Cancellation

**Mechanism**: Global `AtomicBool` flipped by `ctrlc` handler.

**Installation**: Once in `main.rs` via `git::install_signal_handler()` (idempotent).

**Polling**: Commands check `git::was_cancelled()` periodically or after long-running operations (git clone, skill walk, embedding loop, model download, reindex loop).

**Exit Code**: 8 (`TomeError::Interrupted`).

**Invariants**:
- In-flight child processes are killed.
- Atomic writes ensure partial state is not left on disk.
- Index transactions are rolled back on interruption (via `was_cancelled()` checks inside `enable_locked`, `reindex_locked`).
- Tests can reset the flag via `git::reset_cancellation_for_tests()`.

## Atomic Writes & Concurrency

**Pattern**: Write to a temporary file in the same directory as the target, fsync, then rename.

**Locations**:
- Config persistence: `store::save()` → `store::write_atomic()`.
- Cache mutations: Temp dir cloned into, then atomically renamed to final location.
- Index mutations: SQLite WAL + advisory lockfile (`index.lock`). Mutating operations acquire the lock; read-only operations do not.
- Model persistence: Download to temp, verify checksum, rename.
- Reindex mutations: SQLite WAL + per-plugin advisory lock. Each `lifecycle::reindex_plugin` call acquires and releases the lock independently. Per-plugin atomicity: SIGINT between plugins leaves earlier plugins committed.
- Catalog remove cascade: SQLite WAL + single advisory lock. `lifecycle::cascade_disable_for_catalog()` acquires lock once, drops all plugins for the catalog in one transaction, then releases (Phase 9).

**POSIX Atomicity**: On single filesystem, rename is atomic; readers either see the old or new version, never partial state.

**SQLite Concurrency**: WAL mode + 5s `busy_timeout` allows multiple readers + one writer. Advisory lockfile is a Tome-owned per-FD OS-level lock; held for the duration of index mutations.

**Tested**: `tests/atomicity.rs` verifies interruption injection; `tests/concurrency.rs` verifies two-process index contention.

## Cross-Cutting Concerns

| Concern | Implementation | Location |
|---------|----------------|----------|
| **Credential Scrubbing** | Regex rules applied to all captured git and reqwest output before display or error propagation. | `src/catalog/git.rs::scrub_credentials()`, `src/embedding/download.rs` |
| **Error Mapping** | Every `Result<_, TomeError>` eventually reaches `main()`, which maps to exit code. | `src/error.rs`, `src/main.rs` |
| **Logging & Verbosity** | Global tracing subscriber initialized once; orthogonal to `--json` mode. | `src/logging.rs`, `src/main.rs` |
| **TTY Detection** | Used by interactive commands (removal confirmation, model download prompt, interactive browse, reindex scope confirmation), progress spinners, and output formatting. | `src/output.rs::stdin_is_tty()`, `stdout_is_tty()`, `src/presentation/prompt.rs`, `src/presentation/progress.rs`, `src/commands/plugin/interactive.rs`, `src/commands/models/remove.rs`, `src/commands/catalog/remove.rs` |
| **Model Presence** | Two-stage check: manifest.json exists on disk AND parses. Applied before `enable` and `query`. | `src/plugin/mod.rs::model_manifest_ok()`, `src/embedding/download.rs` |
| **Path Validation** | Plugin sources in `tome-catalog.toml` are validated relative to catalog root (no `..`, no escape). Plugin source in `lifecycle::resolve_plugin_dir` is manifest-declared or flat-layout fallback. | `src/catalog/manifest.rs::validate_source()`, `src/plugin/lifecycle.rs::resolve_plugin_dir()` |
| **Drift Detection** | Embedder and reranker identity (name + version) stored in index meta; compared at query time. Embedder drift → hard fail (exit 41/42); reranker drift → warn. | `src/index/meta.rs`, `src/commands/query.rs::check_drift()` |
| **Content-Hash Smart Re-Embedding** | Skill text (name + "\n\n" + description) is hashed; hash is compared at reindex time. Unchanged hash → skip embedder, reuse vector. Used by both `enable` (first-time, always embed) and `reindex` (smart fast-path). | `src/index/skills.rs::content_hash()`, `reindex_plugin_atomic()` |
| **Health Classification** | Pre-computed per-subsystem status (embedder, reranker, index, drift) classified into `OverallHealth` enum (Ok / Degraded / Unhealthy). | `src/commands/status.rs::classify_*` helpers |
| **Version Output Override** | Pre-parse hook in `main.rs` intercepts `--version` / `-V` before clap to include model identities and honour `--json`. | `src/main.rs`, `src/commands/status.rs::print_version` |
| **Schema Migrations** | Forward-only registration framework; write paths call `apply_pending` with pre-computed target version; read paths use legacy 52 for backward compat. Tests inject synthetic migrations via `MIGRATIONS_OVERRIDE` thread-local. | `src/index/migrations.rs`, `src/index/db.rs` (bootstrap + write-lock), `tests/schema_migrations.rs` |

---

## What Does NOT Belong Here

- Directory structure details → STRUCTURE.md
- Technology versions → STACK.md
- External service configs → INTEGRATIONS.md
- Code style rules → CONVENTIONS.md

---

*This document describes HOW the system is organized. Keep focus on patterns and relationships.*

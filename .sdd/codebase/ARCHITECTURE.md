# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13 (Phase 3 User Story 1) + 2026-05-13 (Phase 4 User Story 2 — interactive browse) + 2026-05-13 (Phase 5 User Story 3 — plugin disable subcommand) + 2026-05-13 (Phase 6 User Story 4 slice 1 — models commands) + 2026-05-13 (Phase 7 User Stories 5–7 — reindex orchestrator, catalog-update cascade, explicit CLI) + 2026-05-13 (Phase 8 User Story 6 — health diagnostics) + 2026-05-14 (Phase 9 User Story 7 — catalog remove cascade) + 2026-05-14 (Foundational F7 + F8 — schema migrations framework, MCP async island) + 2026-05-14 (Phase 3 User Story 1 — MCP server wired) + 2026-05-14 (Phase 3 User Story 2 — workspace context, `tome workspace info/init`) + 2026-05-14 (Phase 3 User Story 3 — per-command scope honouring, reference-counted catalog clone cleanup) + 2026-05-14 (Phase 3 User Story 4 — `tome doctor` diagnostic layer with auto-fix)

## Architecture Overview

Tome is a **dual-mode Rust application** (CLI + MCP server) organized around a **capability-driven** modular architecture. The CLI follows a classic **parse → dispatch → execute → map-errors → exit** pipeline. The MCP server shares library-shaped logic (embedding, index, plugin metadata) but dispatches via `rmcp::serve_server(stdio())` in an async island under `src/mcp/`. Error handling is centralized in a closed `TomeError` enum that enforces exhaustive exit-code mapping at compile time. Signal handling (SIGINT) is global and atomic for the CLI, async-aware for the MCP server (via `tokio::signal::ctrl_c()`). A forward-only schema migration framework governs index evolution with three dedicated exit codes (51 for integrity failures, 73 for schema-version-too-new on write, 74 for migration application errors). The MCP async island under `src/mcp/` (Phase 3 US1 now filled) provides stdio transport, tool registration via rmcp macros, lazy reranker loading per FR-109, and preflight validation. Two log paths coexist: CLI stderr (tracing-subscriber) and MCP file log (JSON-lines, size-based rotation) — they are mutually exclusive per FR-221 (stdout is the MCP protocol channel). **Phase 3 US2** (workspace context) introduces per-project workspaces: a `.tome/` directory marking a workspace root, with scope resolution walking upward from CWD, optional `TOME_WORKSPACE` env var, `--workspace` / `--global` CLI flags, and two new read-only commands `tome workspace info` (scope diagnostics) and `tome workspace init` (atomic `.tome/` creation with optional catalog inheritance). **Phase 3 US3** (per-command scope honouring) gates all commands through scope resolution and reference-counts catalog cache directories across scopes, enabling safe concurrent use of shared catalog clones. **Phase 3 US4** (`tome doctor`) adds a comprehensive diagnostic layer: `assemble_report(scope, paths, home)` reads all subsystems (models, index, catalogs, harnesses, drift) without mutation, classifies overall health (Ok / Degraded / Unhealthy), detects problems (missing files, manifest corruption, version mismatches), and proposes safe auto-fixes (`--fix` re-downloads models, re-clones broken catalogs, applies schema migrations). Repairs that cannot be automated (manual plugin reconciliation, corrupted index reconstruction) are flagged with exit 75 (`DoctorFixNotSafe`).

## Architecture Pattern

| Pattern | Description |
|---------|-------------|
| **Dual-mode: CLI + MCP** | CLI (sync, ~20 MB binary) and MCP server (async, ~22 MB including rmcp) coexist. CLI dispatch skips logging/signals; MCP is routed differently from other top-level commands (Phase 3 US1). |
| **Sync CLI, Async MCP** | All CLI code is sync. MCP lives in `src/mcp/` with `tokio` scoped exclusively here. Structural test `tests/sync_boundary.rs` enforces. Shared library functions (`query::pipeline`, `lifecycle::enable`, index CRUD) are sync; both CLI and MCP call them. |
| **Workspace + Global Scope** | Dual-install model (Phase 3 US2): per-project `.tome/` directory (workspace scope) or global XDG (global scope). `--workspace <path>` or `TOME_WORKSPACE=<path>` or CWD walk override the default (global fallback). Every command honours scope resolution via `ResolvedScope(Scope, ScopeSource)` threaded from pre-dispatch. `tome workspace info/init` are read-only + atomic scope-management commands. |
| **Scope-Parametrized Paths** | `src/paths.rs::Paths` now has accessor methods like `config_file_for(&Scope)` and `index_db_for(&Scope)` that compute per-scope locations. Workspace uses `${workspace_root}/.tome/` for config and index; global uses `${XDG_CONFIG_HOME}/tome/` and `${XDG_DATA_HOME}/tome/`. Single `Paths` struct parametrized at call sites (Phase 3 US2). |
| **Content-Addressed Shared Catalog Cache** | Catalog Git clones are cached at `${XDG_DATA_HOME}/tome/catalogs/<sha256(url)>/`, shared across scopes (Phase 3 US3). Per-scope config references the cached URL; multiple scopes can reference the same cached clone. `store::reference_count(url, paths)` walks scope configs to enumerate all references (TOCTOU-benign: one winner on concurrent remove, other no-ops). Catalog cache directory is only deleted when ref count reaches zero (Phase 3 US3). |
| **Closed Error Set** | All failure paths map to a single `TomeError` enum with explicit exit codes; no `Other` or `Unknown` arms. Adding a failure mode requires specification, error type, and test updates. Phase 3 US1 adds `McpStartupFailed` and `McpProtocolIo`. Phase 3 US2 adds `WorkspaceNotFound` (71), `WorkspaceMalformed` (75). Phase 3 US4 adds `DoctorFixNotSafe` (75, different context). |
| **Atomic Writes** | Registry mutations, cache operations, and index writes use `tempfile` + rename for POSIX atomicity; SQLite WAL provides the index concurrency contract. Interruptions cannot corrupt state. **Workspace init pattern**: `tempfile::Builder::tempdir_in(workspace_root)` creates a sibling `.tome.tmp.XXXX/`, stages all files inside, then `std::fs::rename(staged, .tome)` lands atomically. On failure, `.tome.old/` rollback restores the previous state (Phase 3 US2). **Catalog add pattern** (Phase 3 US3): reuse existing cache if present (cheap manifest check, skip git clone), else clone as before (Phase 1 behavior). |
| **Capability-Organized Modules** | Modules group related functionality: `catalog/` (manifest + Git + store), `commands/` (CLI handlers), `config/` (manifest deserialization), `paths/` (XDG resolution + scope-parametrized accessors), `logging/` (tracing setup), `output/` (human/JSON formatting), `plugin/` (metadata parsing + lifecycle), `index/` (SQLite skills DB + KNN + migrations), `embedding/` (fastembed wrapper + model registry + download), `presentation/` (tables / progress / colour / prompts), `workspace/` (scope resolution + workspace info/init), `doctor/` (diagnostic orchestration, state reporting, auto-repairs), `mcp/` (async server boundary + stdio transport + preflight). |
| **Credential Scrubbing at Boundary** | All captured `git` and `reqwest` output passes through credential scrubbing before reaching logging, error display, or structured output. |
| **Trait-based Embedding Abstraction** | `Embedder` and `Reranker` are seam interfaces; `FastembedEmbedder` wraps `fastembed-rs`, and a deterministic `StubEmbedder` (unit-test only) provides testability without model files. |
| **Plugin-Dir Resolution: Manifest-First** | `lifecycle::resolve_plugin_dir` reads `tome-catalog.toml`, looks up `id.plugin` in the declared `plugins[].name`, joins with the source; falls back to flat `entry.path.join(&id.plugin)` for backward compat when manifest is absent. Single shared function across `enable`, `disable`, `list`, `show` fixes inconsistency. |
| **Interactive Three-Level Loop Pattern** | Bare `tome plugin` (no subcommand) enters an interactive flow: `catalog_loop` → `plugin_loop` → `view_loop`, each with a `LoopExit` enum to handle Back/Quit unwinds and error propagation (Phase 4, User Story 2). |
| **Per-Plugin Atomic Reindex** | `lifecycle::reindex_plugin` mirrors `lifecycle::enable` atomicity: each plugin's reindex is one SQLite transaction under one advisory lock. Batch operations (`tome catalog update`, `tome reindex`) loop per-plugin, committing each before moving to the next. SIGINT between plugins leaves earlier plugins committed (per-plugin boundary). |
| **Lazy Embedder Loading** | Heavy embedder (~345 MB ONNX) is loaded only when reindexing will actually call it. `tome catalog update` and `tome reindex` defer load until the first enabled plugin is encountered; a sync with zero enabled plugins never touches model files. MCP preflight (FR-110) eager-loads the embedder; reranker is lazy-loaded on first `search_skills` call per FR-109. |
| **Health Diagnostics (Read-Only)** | `status` command reads subsystem state without mutation: models via manifest checks, index via read-only connection + `PRAGMA integrity_check`, drift via stored identity comparison. Never acquires advisory lock, never downloads models. Exits 0 on Ok, 1 on Degraded/Unhealthy. |
| **Comprehensive Diagnostic & Auto-Repair** | `doctor` command extends health diagnostics with harness detection (6 known agentic coding harnesses) and automated repairs (`--fix`). `doctor::assemble_report(scope, paths, home)` classifies subsystems (models, index, catalogs, harness presence), proposes safe auto-fixes (re-download, re-clone, migrate), and flags non-automatable issues. Exit codes: 0 (healthy), 1 (degraded/unhealthy), 75 (`--fix` attempted but manual fixes remain). Harness probe reads `$HOME`-relative paths; threaded as parameter so tests can isolate (no env mutation). |
| **Single-Lock-Per-Batch Cascade** | `lifecycle::cascade_disable_for_catalog` acquires the advisory lock once, disables + drops all enabled plugins for a catalog, then releases. Different from per-plugin operations; chosen to match the contract in `specs/002-phase-2-plugins-index/contracts/catalog-extensions.md` §"tome catalog remove". |
| **Forward-Only Schema Migrations** | `src/index/migrations.rs` enforces a registration framework: `apply_pending(conn, current, target)` applies registered steps within a read lock. Three exit codes govern the migration domain: 51 (`IndexIntegrityCheckFailure` — unknown state post-migration), 73 (`SchemaVersionTooNew` — write path refuses newer-on-disk schemas), 74 (`SchemaMigrationFailed` — registered step apply error). Read path via `open_read_only` keeps legacy 52 (`SchemaTooNew`) for backward compat. Phase 2 ships zero registered migrations; Phase 3 tests inject synthetic migrations. |
| **MCP Async Island Boundary** | `src/mcp/` directory is a tokio-scoped async boundary (research §R-2 pinned to current-thread runtime per Phase 3 plan). Phase 3 US1 fills with actual server loop: `mod::run(scope, paths)` builds runtime, installs file-log subscriber, runs preflight on blocking pool, constructs `McpState`, drives `rmcp::serve_server(stdio())`, and `tokio::select!`s over graceful shutdown vs SIGINT. Preflight validates scope-resolved index state before handoff: schema gate (emits 73), drift detect, SHA-256 verify, eager-load embedder. `log.rs` wires size-based rotation (FR-227) + JSON-lines tracing registry (FR-226) + stderr-only error layer (FR-220). Structural test `tests/sync_boundary.rs` enforces the exemption. |
| **MCP Tool Registration & Dispatch** | `server.rs` impl `rmcp::ServerHandler` with `#[tool_router]` + `#[tool_handler]` macros. Routes `list_tools` / `call_tool` through the generated `ToolRouter`. Two tools advertised: `search_skills` and `get_skill`. Each tool method delegates to a free function in `mcp::tools::{search_skills,get_skill}::handle` so per-tool logic stays modular. Input/output schemas derived from `Deserialize` / `Serialize` / `JsonSchema`. |
| **Lazy Reranker in MCP** | `McpState` carries `Arc<dyn Embedder>` (eager, from preflight) and `OnceCell<Arc<dyn Reranker>>` (lazy). First `search_skills` call checks `OnceCell`; if empty, loads reranker synchronously on the blocking pool and stores it. Subsequent calls reuse the cached instance. Per FR-109, embedder is eager but reranker deferred. |
| **MCP Query Pipeline Reuse** | `search_skills` handler validates input against scope config (rmcp error codes per contract), then dispatches to `commands::query::pipeline()` (extracted as a silent compute path). Reuses all KNN + rerank logic without duplication. Returns `SkillMatch` records with absolute paths and opaque scores. |
| **Helper Promotion for Shared State** | When two diagnostic surfaces must agree on the same subsystem reading (status + doctor on models/index/drift), the only safe way is to share the implementation. Promote the helpers to `pub` (e.g., `status::check_model()`, `status::check_index()`, `status::check_drift()`) rather than duplicating compute. Single source of truth prevents drift. Risk mitigated by documenting as "read-only diagnostic helpers" (no side-effects, no internal mutations). |
| **Library-API + Emit-Wrapper Split with Home Parameter** | `doctor::assemble_report` takes `home: &Path` so the harness probe is testable without env mutation. CLI wrapper (`commands/doctor.rs`) resolves `$HOME` and passes it through. Same idiom is reusable for any future read-from-env operation that tests need to isolate. Decouples IO from logic, enables dependency injection for testing. |
| **Re-Assemble After Partial Repairs** | After `--fix` applies repairs, re-compute suggested_fixes + overall health without re-probing catalogs or harnesses. Each repair already re-ran its affected check function (model re-download re-ran the model check, catalog re-clone re-ran the catalog state check, schema migrate re-ran the index check). Only suggested_fixes + overall need recomputing. Avoids doubling FS cost for catalog enumeration + harness probing. Pattern is efficient + composable when multiple repairs are applied. |

## Core Components

### Catalog Cache & Reference Counting (`src/catalog/store.rs`, Phase 3 US3)

- **Purpose**: Implement reference-counted catalog cache directories shared across scopes.
- **Location**: `src/catalog/store.rs` (new `reference_count()` function).
- **Public Interface**: `pub fn reference_count(url: &str, paths: &Paths) -> Vec<Scope>` — walks global config and all workspace registries, returns list of scopes that reference the given catalog URL.
- **Design**:
  - Catalog Git clones stored at content-addressed path `${XDG_DATA_HOME}/tome/catalogs/<sha256(url)>/` (Phase 1).
  - Per-scope config.toml references the URL; multiple scopes can reference the same cached clone (Phase 3 US3).
  - `reference_count()` is the ad-hoc reference table: unlocked walk of all scope configs (TOCTOU-benign per concurrency contract).
  - `store::save()` is responsible for persisting config; callers (e.g., `commands/catalog/add.rs`) own the rollback path.
- **TOCTOU Profile**: Concurrent remove operations race benignly — one caller wins, other sees empty list and no-ops (cache dir already deleted). Concurrent add-then-remove leaves a dangling cache directory (recoverable by `tome catalog update` which re-clones if needed).
- **Workspace Integration**: Optional `${XDG_STATE_HOME}/tome/workspaces.txt` registry enables efficient workspace discovery. Without it, reference_count falls back to reading global config only (documented degradation, not a bug).

### Catalog Add with Cache Reuse (`src/commands/catalog/add.rs`, Phase 3 US3)

- **Purpose**: Register a catalog, reusing existing cache clone if URL already cached elsewhere.
- **Location**: `src/commands/catalog/add.rs`.
- **Flow**:
  1. Compute cache directory from catalog URL (sha256).
  2. If cache exists: reuse it (read manifest from cache, skip git clone). Rollback only deletes cache on error if we cloned.
  3. Else: clone as before (Phase 1 behavior).
  4. Persist catalog entry to scope-specific config.toml.
  5. Check display-name collision within the scope (rejects same-alias-within-one-scope per Phase 1).
- **Optimization**: Reuse existing clone avoids redundant network I/O when the same catalog URL is registered in multiple scopes. Cheap manifest check determines if cache is usable.

### Catalog Remove with Reference Counting (`src/commands/catalog/remove.rs`, Phase 3 US3)

- **Purpose**: Unregister catalog; conditionally delete cache based on reference count.
- **Location**: `src/commands/catalog/remove.rs`.
- **Flow**:
  1. Check if enabled plugins exist (exit 53 if not `--force`).
  2. If enabled and `--force`, cascade-disable all plugins (Phase 9 logic).
  3. Remove config entry from scope-specific config.toml (atomic).
  4. Call `reference_count(&entry.url, paths)` to check remaining references.
  5. If ref count is zero, delete cache directory. Otherwise log remaining references at debug level.
- **Composition**: Cascade-disable is orthogonal to refcount; both run within the same remove flow. TOCTOU-safe: pre-check for enabled plugins runs without lock; the worst outcome is cascading an extra plugin (still correct).

### Catalog Management (`src/catalog/`, `src/commands/catalog/`)

- **Purpose**: Orchestrate catalog registration, refresh, removal, and inspection; manage Git cloning and credential scrubbing.
- **Location**: `src/catalog/` (core logic: git, manifest, store), `src/commands/catalog/` (subcommand handlers).
- **Dependencies**: `git` (shell-outs), `manifest` (TOML parsing + validation), `store` (atomic writes + reference counting), `config` (registry persistence).
- **Dependents**: Main CLI, integration tests.
- **Key Invariants**:
  - Catalogs are cached at `~/.local/share/tome/catalogs/<sha256(url)>/` (content-addressed, shared across scopes).
  - Config is persisted per scope atomically.
  - Git operations capture stderr and pass it through credential scrubbing before error display.
- **Phase 7 Change**: `tome catalog update` now reindexes enabled plugins in each catalog after a Git refresh. Per-plugin atomicity: each `lifecycle::reindex_plugin` call owns its own lock. Auto-disable cascade on `PluginNotFound` / `PluginManifestParseError` via `lifecycle::auto_disable_orphan()`.
- **Phase 9 Change**: `tome catalog remove` now refuses with exit 53 (`CatalogHasEnabledPlugins`) if enabled plugins exist, unless `--force` is passed. On `--force`, calls `lifecycle::cascade_disable_for_catalog()` to drop all enabled plugin rows in one lock window, then proceeds with Phase 1 removal logic.
- **Phase 3 US3 Change**: Catalog cache is now reference-counted across scopes. `add` reuses existing clones; `remove` deletes cache only when ref count reaches zero.

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
- **Public Interface**: `pub fn run(args: PluginDisableArgs, scope: &ResolvedScope, mode: output::Mode) -> Result<(), TomeError>`.
- **Flow**:
  1. Parse `PluginId` from args.
  2. Load config (scope-specific), verify plugin exists (fail fast before prompt).
  3. If not `--force`, check TTY (non-TTY → emit pointer line to stderr, return `NotATerminal` exit 54).
  4. TTY: prompt with default "no" per spec. User decline → clean exit Ok(()) + optional stderr note.
  5. User accept or `--force`: call `lifecycle::disable()` (returns `DisableOutcome`).
  6. Emit human or JSON output.
- **Error Semantics**: Same exit codes as `lifecycle::disable` (exit 32 if already disabled). Non-TTY without `--force` → exit 54 (`NotATerminal`).
- **Pattern**: Mirrors `enable.rs` in structure (validate → prompt → call library → emit). No embedder construction — index-only UPDATE. Cheap re-enable tested via `tests/plugin_enable.rs::cheap_reenable_after_disable_invokes_embedder_zero_times`.

### Interactive Browse Flow (`src/commands/plugin/interactive.rs`)

- **Purpose**: Bare `tome plugin` (no subcommand) — provide an interactive catalog → plugin → action browse loop (Phase 4, User Story 2).
- **Location**: `src/commands/plugin/interactive.rs` (~515 lines).
- **Public Interface**: `pub fn run_interactive(scope: &ResolvedScope, mode: output::Mode) -> Result<(), TomeError>`.
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
- **TTY Enforcement** (FR-051): Non-TTY invocation via `presentation::prompt::select()` will refuse with `NotATerminal`, propagating as exit code 54.
- **Phase 3 US3 Change**: Receives scope-resolved `ResolvedScope` from pre-dispatch; uses scope-parametrized config/index paths.

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
  - `pub fn run(args: StatusArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError>` — CLI entry; emits report, exits 0 on Ok, 1 on Degraded/Unhealthy.
  - `pub fn assemble_report(scope: &ResolvedScope, paths: &Paths, verify: bool) -> Result<StatusReport, TomeError>` — library entry (used by tests); produces report without exit side-effect.
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

### Doctor Diagnostic Layer (`src/doctor/`, `src/commands/doctor.rs`, Phase 3 US4)

- **Purpose**: Comprehensive read-only diagnostics of all subsystems (models, index, catalogs, harnesses) plus safe automated repairs (`--fix`). Detects problems without mutating state; proposes solutions. Flags non-automatable issues for manual intervention.
- **Location**: `src/doctor/` (library: orchestration, checks, repairs, reporting), `src/commands/doctor.rs` (CLI: arg parsing, home resolution, emit).
- **Public Interfaces**:
  - `pub fn assemble_report(scope: &ResolvedScope, paths: &Paths, home: &Path, verify: bool) -> Result<DoctorReport, TomeError>` — library entry; produces report without side-effects. Threads `home` parameter so tests can isolate the harness probe.
  - `pub fn fixes::apply(report: &mut DoctorReport, paths: &Paths, scope: &Scope) -> Result<u32, TomeError>` — apply safe repairs (re-download, re-clone, migrate). Each repair re-runs the affected check function in place. Returns count of repairs attempted.
  - `pub fn fixes::re_assemble(report: &mut DoctorReport)` — rebuild suggested_fixes + overall health after repairs (cheap, no re-probing).
  - `pub fn fixes::has_remaining_manual_fixes(report: &DoctorReport) -> bool` — predicate for exit 75 (`DoctorFixNotSafe`).
- **Report Structure** (`DoctorReport`):
  - `tome_version: String` — current version.
  - `workspace: WorkspacePresence { scope, source, path, catalogs, plugins_total, plugins_enabled }` — scope diagnostics.
  - `embedder, reranker: ModelSnapshot { name, version, state }` — model health (Ok / Missing / Corrupt).
  - `index: IndexSnapshot { present, integrity_ok, plugins_enabled, skills_indexed, schema_version, size_bytes }` — index state.
  - `drift: DriftStatus` — embedder/reranker identity mismatches.
  - `catalogs: Vec<CatalogCacheHealth>` — per-catalog state (Ok / Missing / NotARepo / ManifestInvalid).
  - `harnesses: Vec<HarnessPresence>` — probed harnesses (6 known: Claude Code, Cursor, Codex, Gemini CLI, OpenCode, Continue).
  - `overall: DoctorClassification` — health verdict (Ok / Degraded / Unhealthy).
  - `suggested_fixes: Vec<SuggestedFix>` — repair recommendations (subsystem, diagnosis, command, auto_fixable).
- **Modules** (Phase 3 US4):
  - `mod.rs` (~120 lines): Orchestrator `assemble_report()` — calls check functions, classifies overall health, dispatches to fixes.
  - `report.rs` (~130 lines): `DoctorReport`, `CatalogCacheState`, `HarnessPresence`, `DoctorClassification`, `SuggestedFix` — emit-only Serialize types.
  - `checks.rs` (~200 lines): `check_catalogs(paths, scope)` — enumerates registered catalogs, classifies each on-disk clone (existence + `.git/` + manifest-parses). Reuses `status::check_model()`, `status::check_index()`, `status::check_drift()` for single source of truth.
  - `harness_detect.rs` (~100 lines): `probe(home)` — detects harness directories (Claude Code, Cursor, Codex, Gemini, OpenCode, Continue) in well-known locations under `$HOME`. Returns vec of `HarnessPresence` in fixed order.
  - `fixes.rs` (~200 lines): `apply(&mut report, paths, scope)` — applies three safe repair classes: (1) model re-download (clears dir, calls `embedding::download::download_model`), (2) catalog re-clone (removes broken cache, calls `Git::clone_shallow`), (3) schema forward-migrate (acquires lock, calls `index::migrations::apply_pending`). Each repair re-runs the affected check function via `check_catalogs()`, `check_model()`, `check_index()`, etc. in place. `re_assemble()` rebuilds suggested_fixes + overall without re-probing. `has_remaining_manual_fixes()` checks for non-automatable issues.
- **Design Invariants**:
  - Read-only on report assembly: no index mutations, no model downloads (unless `--fix` is explicitly passed).
  - Home as parameter: `assemble_report` takes `home: &Path` so harness probe is testable without env mutation (dependency injection pattern). CLI wrapper resolves `$HOME` and passes through.
  - Single source of truth: reuses model/index/drift checks from `status::` module (shared helper promotion pattern).
  - Per-repair re-check: each repair inside `fixes::apply` re-runs the affected check function, so post-repair state is fresh per field. Only suggested_fixes + overall need recomputing after all repairs are applied.
  - Subsystem-keyed dispatch: each suggested fix carries a `subsystem` string (`"embedder"` / `"reranker"` / `"catalog:name"` / `"schema"`). Repairs match on subsystem prefix to determine action.
  - Exit semantics: 0 (overall Ok), 1 (overall Degraded/Unhealthy, or `--fix` attempted but non-automatable issues remain [actually exit 75 per spec]), 75 (`--fix` ran but `has_remaining_manual_fixes()` is true).
- **Harness Detection** (`harness_detect.rs`):
  - Probes six well-known harness directories: Claude Code (`~/.claude/`), Cursor (`~/.cursor/`), Codex (`~/.codex/`), Gemini CLI (`~/.gemini-cli/`), OpenCode (`~/.opencode/`), Continue (`~/.continue/`).
  - Returns vec of `HarnessPresence { name, path, present }` in fixed order (for stable NDJSON output).
  - Non-blocking: if a directory doesn't exist, `present = false`. No errors, always returns a result.

### MCP Server (`src/mcp/`, Phase 3 US1)

- **Purpose**: Provide an async-native stdio MCP server (complementary to the CLI) that shares library-shaped plugin/embedding/index logic.
- **Location**: `src/mcp/` (six files: `mod.rs`, `server.rs`, `state.rs`, `tools/mod.rs`, `tools/search_skills.rs`, `tools/get_skill.rs`, plus foundational `runtime.rs`, `log.rs`, `preflight.rs`).
- **Files** (Phase 3 US1):
  - `mod.rs` (~140 lines): Sync entry point `pub fn run(scope, paths) -> Result<(), TomeError>`. Opens file log appender, builds tokio runtime, runs preflight on blocking pool, constructs `McpState`, drives `rmcp::serve_server(stdio())`, and `tokio::select!`s over graceful shutdown vs SIGINT.
  - `server.rs` (~90 lines): `rmcp::ServerHandler` impl with `#[tool_router]` + `#[tool_handler]` macros. Routes `list_tools` / `call_tool` through the generated `ToolRouter`. Advertises two tools: `search_skills` and `get_skill`. Each tool method delegates to a free function in `mcp::tools::{search_skills,get_skill}::handle()`.
  - `state.rs` (~30 lines): `McpState` struct carrying `Arc<dyn Embedder>` (eager), `OnceCell<Arc<dyn Reranker>>` (lazy), `ResolvedScope`, `Paths`, and registry entries. Lazy reranker per FR-109; enables concurrent `search_skills` calls with synchronized first-load.
  - `tools/mod.rs` (~15 lines): Tool module aggregation.
  - `tools/search_skills.rs` (~150 lines): Input/output schemas (`#[derive(Deserialize, JsonSchema)]` / `#[derive(Serialize, JsonSchema)]`). `Input`: query text, top_k (1..=100, default 10), optional catalog filter, optional plugin filter. `Output`: `Vec<SkillMatch>` with catalog, plugin, name, description, plugin_version, path (absolute), and opaque score. Handler validates filters against scope config (rmcp error codes per contract), lazy-loads reranker on first call, dispatches to `commands::query::pipeline()` for silent KNN + rerank, returns results.
  - `tools/get_skill.rs` (~100 lines): Input/output schemas. `Input`: catalog, plugin, skill name (triple). `Output`: skill body (frontmatter stripped) + absolute paths of sibling resource files. Handler reads skill body from disk, enumerates resources in skill directory, returns structured result.
  - `runtime.rs` (~50 lines): Current-thread tokio runtime initialization. Per research §R-2, single-threaded reactor suitable for I/O-bound work.
  - `log.rs` (~100 lines): File-log appender wiring. Size-based rotation (FR-227): closes and rotates `${XDG_STATE_HOME}/tome/mcp.log` if exceeds 10 MiB. JSON-lines `tracing-subscriber` registry (FR-226) for structured diagnostics. Stderr-only error layer (FR-220) for fatal startup errors.
  - `preflight.rs` (~120 lines): Pre-flight validation (FR-110) — scope-resolved index read-only open → schema gate (emits 73 on too-new) → drift detect → SHA-256 verify primary file → eager-load `FastembedEmbedder`. Reranker deferred per FR-109. Returns `PreflightReport` with embedder + reranker identities + handle to eager embedder.
- **Async Boundary**:
  - Only files under `src/mcp/` use `tokio`. All other modules remain sync. Shared library functions (index CRUD, plugin lifecycle, embedding) are sync; both CLI and MCP call them.
  - Entry point is sync: `commands::mcp::run(scope, paths)` calls `mcp::run(scope, paths)` which blocks on `runtime.block_on(async { ... })`.
  - Structural test `tests/sync_boundary.rs` enforces: exempts every file under `src/mcp/`, gates every other module for `tokio` imports. Detects leakage.
- **Design Invariants**:
  - No MCP modules import from `commands/` (except `commands::query::pipeline` for reuse). Query logic is library-shaped; no need to duplicate.
  - MCP does NOT import from `cli.rs` or other CLI-specific modules. Scope resolution is handled upstream; MCP receives a `ResolvedScope`.
  - Preflight is defensive: errors during validation do not crash the server; they surface as exit codes to the harness.
  - Log rotation aware of MCP's continuous nature (file size cap, not line count).
  - Lazy reranker per FR-109: embedder eager (loaded at startup), reranker deferred (loaded on first `search_skills`). `OnceCell` ensures thread-safe idempotent initialization.
  - File logging only: stdout is the MCP protocol channel (FR-221), stderr is reserved for fatal startup errors only (FR-222). Diagnostics go to `${XDG_STATE_HOME}/tome/mcp.log`.

### MCP Dispatcher (`src/commands/mcp.rs`, Phase 3 US1)

- **Purpose**: Thin CLI dispatcher for `tome mcp` command. Routes to `mcp::run(scope, paths)`.
- **Location**: `src/commands/mcp.rs` (~20 lines).
- **Public Interface**: `pub fn run(_args: McpArgs, scope: &ResolvedScope, _mode: Mode) -> Result<(), TomeError>`.
- **Flow**:
  1. Resolve paths.
  2. Call `mcp::run(scope, &paths)`.
  3. Return result directly (no output formatting — MCP protocol is the structured output).
- **Design Note**: Unlike every other command, this one does NOT honour `--json` at the top level. The MCP protocol itself is the structured output; `--json` would muddle the stdio transport channel (FR-221).

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

- **Purpose**: Embed the user's query text, perform KNN over enabled skills, optionally rerank, filter, and render results. Reused by both CLI (`tome query`) and MCP (`search_skills`).
- **Location**: `src/commands/query.rs` (CLI entry and result presentation), `src/index/query.rs` (KNN SQL + filter logic).
- **Dependencies**: `index::` (open read-only, knn), `embedding::` (embedder + reranker), `catalog::manifest` (filter validation).
- **Public Entry Point** (Phase 3 US1): `pub fn pipeline(args, deps) -> Result<QueryOutcome, TomeError>` — silent compute path reused by both CLI and MCP. Returns `QueryOutcome` with ranked results; caller handles formatting/output.
- **Flow**:
  1. Parse query text and filter flags (`--catalog`, `--plugin`, `--no-rerank`, `--min-score`, `--strict`).
  2. Validate filter flags against registered catalogs (cheap manifest reads).
  3. Open index read-only (scope-specific).
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
- **Phase 3 US2 additions** (deferred to Phase 10 for general refactoring):
  - `config_file_for(&Scope)`, `index_db_for(&Scope)`, `index_lock_for(&Scope)`: scope-parametrized accessors.

### Logging (`src/logging.rs`)

- **Purpose**: Initialize `tracing-subscriber` with stderr-only output, orthogonal to `--json`.
- **Location**: `src/logging.rs`.
- **Verbosity**: `-v` = info, `-vv` = debug; env vars `TOME_LOG`, `RUST_LOG` supported.
- **Key Invariant**: Logs go to stderr; primary command output (--json or human) goes to stdout. This keeps structured data uncontaminated by debug output.
- **Phase 3 US1 Note**: Not initialized for `tome mcp`; MCP manages its own logging via `mcp/log.rs` (file appender + JSON-lines subscriber).

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
- **Variants** (24+ total, Phase 3 US1 additions):
  - Phase 1: `Internal`, `Usage`, `CatalogNotFound`, `CatalogAlreadyExists`, `ManifestInvalid`, `GitFailed`, `Io`, `Interrupted`.
  - Phase 2: `IndexIntegrityCheckFailure` (51), `IndexBusy` (50), `ModelMissing` (30), `PluginNotFound` (20), `SkillFrontmatterParseError` (23).
  - Phase 3: `PluginAlreadyInState` (31/32), `QueryNoResultsStrict`, drift checks (exit 41/42).
  - Phase 3 US1: `McpStartupFailed`, `McpProtocolIo` (new for MCP-specific failures).
  - Phase 3 US2: `WorkspaceNotFound` (71), `WorkspaceMalformed` (75), `WorkspaceConflict` (72).
  - Phase 9: `CatalogHasEnabledPlugins` (exit 53).
  - Phase 3 US4: `DoctorFixNotSafe` (exit 75).
  - **Foundational F7** (schema migration domain): `SchemaVersionTooNew` (73), `SchemaMigrationFailed` (74). Legacy `SchemaTooNew` (52) retained for read-path.
- **Compile-Time Enforcement**: The `TomeError::exit_code()` method is exhaustive; adding a variant forces edits to `tests/exit_codes.rs`, the spec, and the PRD.

## Data Flow

### Diagnostic Doctor Flow: `tome doctor [--fix] [--verify]`

```
CLI parse (--fix, --verify)
       ↓
resolution → ResolvedScope
       ↓
resolve $HOME from environment
       ↓
dispatch to commands::doctor::run(args, scope, mode)
       ↓
assemble_report(scope, paths, &home, verify):
  → call checks::check_catalogs(scope, paths):
      → load config
      → for each catalog: check existence, .git/, manifest validity
      → return Vec<CatalogCacheHealth>
  → call harness_detect::probe(&home):
      → check 6 well-known harness dirs under $HOME
      → return Vec<HarnessPresence>
  → reuse status::check_model() for embedder/reranker states
  → reuse status::check_index() for index health
  → reuse status::check_drift() for embedder/reranker drift
  → classify overall health (Ok / Degraded / Unhealthy)
  → propose suggested_fixes based on detected issues
  → return DoctorReport
       ↓
if args.fix:
  → fixes::apply(&mut report, paths, scope):
      ↓
      for each suggested fix in report:
        → match fix.subsystem:
            "embedder" | "reranker":
              → call embedding::download::download_model()
              → call status::check_model() to update report field
            "catalog:<name>":
              → extract catalog name from "catalog:XXX"
              → call Git::clone_shallow(url, cache_dir)
              → call checks::check_catalogs(scope, paths) to re-classify
            "schema":
              → acquire advisory lock
              → call index::migrations::apply_pending()
              → call status::check_index() to re-check
       ↓
  → fixes::re_assemble(&mut report):
      → rebuild suggested_fixes based on current report state
      → reclassify overall health
       ↓
emit(report, mode):
  → human: formatted multiline with glyphs, section headers
  → JSON: single NDJSON record
       ↓
if overall == Ok:
  exit(0)
else if args.fix && has_remaining_manual_fixes(report):
  exit(75) with DoctorFixNotSafe error
else:
  exit(1)
```

### Workspace Resolution & Scope Threading

```
main() pre-dispatch:
  → parse Cli { scope: GlobalScopeArgs { workspace, global }, … }
  → call workspace::resolution::resolve(workspace_flag, global_flag, env::var("TOME_WORKSPACE"), cwd)
       ↓
       resolution::resolve(workspace_opt, global_opt, env_opt, cwd):
         → if both workspace_opt and global_opt: return Err(WorkspaceConflict) exit 72
         → if workspace_opt: verify path exists && contains .tome/ → return Workspace(path), source=Flag
         → if global_opt: return Global, source=GlobalFlag
         → if env_opt: verify path exists && contains .tome/ → return Workspace(path), source=Env
         → walk cwd upward looking for .tome/ → return Workspace(found_path), source=CwdWalk
         → return Global, source=GlobalFallback
       ↓
       ResolvedScope { scope, source } produced
       ↓
  → thread ResolvedScope into every command invocation: run(args, scope, mode)
       ↓
dispatch to commands::{catalog,plugin,query,models,reindex,status,doctor,workspace}::run(args, scope, mode)
  ↓
  → each command calls Paths::resolve()
  → uses scope-parametrized accessors: paths.config_file_for(&scope), paths.index_db_for(&scope), etc.
  ↓
  → workspace-scoped config and index accessed
```

### Health Report Flow: `tome status [--verify]`

```
CLI parse (--verify flag)
       ↓
resolution → ResolvedScope
       ↓
dispatch to status::run()
       ↓
assemble_report(scope, paths, verify=flag):
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
resolution → ResolvedScope
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

*This document describes the system design and component relationships. For directory layout details, see STRUCTURE.md.*

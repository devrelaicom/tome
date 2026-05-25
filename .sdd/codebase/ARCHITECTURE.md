# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-25
> **Last Updated**: 2026-05-25

## Architecture Overview

Tome is a Rust CLI tool and MCP server that manages plugin ecosystems across coding harnesses (Claude Code, Cursor, Gemini CLI, Codex, OpenCode). It provides a centralized index for skill discovery and reranking, multi-workspace support with per-project bindings, harness composition management, and workspace-scoped plugin enablement.

The architecture is **monolithic with layered structure** split across two execution contexts:
- **CLI layer** — sync command dispatcher
- **MCP layer** — async stdio server (Phase 3+)

The central nervous system is a **single SQLite database** (`<home>/.tome/index.db`) that centralizes all state: plugin metadata, embeddings, workspace bindings, project bindings, and enabled skills. Per-workspace composition settings and summaries live in separate TOML files (`<root>/workspaces/<name>/settings.toml`) and central RULES.md. Project markers (`<project>/.tome/config.toml`) are thin binding pointers, not databases.

Phase 4 / US1–US2 adds **harness synchronization** and **workspace lifecycle management** — when a project is bound to a workspace, Tome automatically syncs harness configurations (rules files + MCP configs) across the five supported harnesses. Full workspace create/rename/remove/sync/info/list surface ships with atomic per-harness writes and shared-path deduplication.

## Architecture Pattern

| Pattern | Description |
|---------|-------------|
| Layered (capability-based) | Commands → Business Logic (Lifecycle, Embedding, Workspace, Harness) → Data Access (Index, Catalog, Config) → Persistence (SQLite, Filesystem, Git) |
| Hexagonal (ports & adapters) | Trait boundaries for `Embedder`/`Reranker`/`Summariser`/`HarnessModule` allow swappable implementations (production vs stub for tests) |
| Trait-driven | Core abstractions decouple policy from mechanism; composition via struct fields rather than factory functions |
| Phase 4 — Harness abstraction + workspace binding + full lifecycle | Five `HarnessModule` impls + sync orchestrator + comprehensive workspace surface (init/list/info/rename/remove/sync + regen-summary) enable multi-workspace projects with atomic per-harness configuration |

## Core Components

### CLI Entry Point (`src/main.rs`)

- **Purpose**: Parse arguments, resolve workspace context, dispatch to subcommands
- **Location**: `src/main.rs`
- **Key flow**:
  1. Pre-parse `--version` flag (before clap) to include embedder/reranker/summariser identities
  2. Resolve `Paths` once from `$HOME/.tome/` (Phase 4 single root)
  3. Resolve workspace via `workspace::resolution::resolve()` (consults central DB)
  4. Route command dispatch; translate TomeError to exit codes
  5. Special-case MCP: skip stderr logging init + ctrlc handler (uses tokio signal)

### Path Resolution (`src/paths.rs`)

- **Purpose**: Resolve all Tome-owned paths from `$HOME/.tome/` root (Phase 4 consolidated)
- **Location**: `src/paths.rs`
- **Phase 4 changes**: Dropped XDG split; everything under single `<home>/.tome/` root (constitution v1.3.0 §Paths amendment)
- **Public fields**:
  - `root` — `<home>/.tome/`
  - `index_db`, `index_lock` — central database
  - `catalogs_dir`, `models_dir` — on-disk resources
  - `workspaces_dir` — per-workspace settings
  - `logs_dir`, `mcp_log`, `mcp_log_prev` — diagnostics
- **Invariant**: All path joins happen here; no string literals elsewhere (enforced by test guards)

### Workspace Scope Resolution (`src/workspace/`)

- **Purpose**: Determine active workspace from CLI flag, env var, project marker, or default
- **Location**: `src/workspace/{name,scope,resolution}.rs`
- **Phase 4 changes**:
  - `Scope` → tuple struct `Scope(pub WorkspaceName)` (was: enum `Scope::Global | Scope::Workspace(PathBuf)`)
  - `ResolvedScope` gains `project_root: Option<PathBuf>` field
  - `--workspace <NAME>` flag (was: `--workspace <PATH>`); no more `--global` flag
  - Privileged `"global"` workspace is silent default
- **Resolution algorithm**:
  1. Check `--workspace <NAME>` CLI flag (validate against central `workspaces` table)
  2. Check `TOME_WORKSPACE` env var
  3. Walk project hierarchy for `.tome/config.toml` marker (read `workspace` field)
  4. Fall back to `"global"` workspace (always exists)
  5. Emit `WorkspaceConflict` (72) if multiple markers found; `WorkspaceNotFound` (13) if name not in registry

### Project-to-Workspace Binding (`src/workspace/binding.rs`)

- **Purpose**: Phase 4 / US1.a — Bind a project to a workspace; land atomic project marker
- **Location**: `src/workspace/binding.rs`
- **Key entry point**: `bind_project(project_root, workspace_name, force, deps) -> Result<BindOutcome, TomeError>`
- **Algorithm**:
  1. Dangerous CWD check (refuse `$HOME`, `/` unless `--force`)
  2. Acquire central DB advisory lock
  3. UPSERT into `workspace_projects` table (project_path PK, workspace_id FK, bound_at timestamp)
  4. Bump workspace `last_used_at` timestamp
  5. Land `<project>/.tome/config.toml` with `[workspace] = <name>` atomically via tempfile + rename
  6. Release lock; return `BindOutcome` with project_root, workspace name, and sync-outcome placeholder
- **Atomicity**: If DB commits but marker landing fails, doctor's Binding subsystem detects orphan; re-running recovers
- **Phase B** (harness sync): Runs outside this module, outside the lockfile (see `harness::sync`)

### Workspace Lifecycle (`src/workspace/{init,rename,remove,sync,regen_summary}.rs`)

- **Purpose**: Phase 4 / US2 — Complete workspace management surface
- **Location**: `src/workspace/{init,rename,remove,sync,regen_summary}.rs`
- **`init(target_root, workspace_name, inherit_global, force)` entry point**:
  - Atomic directory landing for `<root>/workspaces/<name>/` (settings.toml + RULES.md skeleton)
  - Creates row in central `workspaces` table
  - Optional catalog inheritance from global workspace (enrolment only; enablement not copied per FR-415)
  - Atomicity via `tempfile::Builder::tempdir_in` + `TempDir::keep()` + `std::fs::rename`
- **`rename(old_name, new_name, paths, workspace_name)`**:
  - Validates neither side is reserved `global` (exit 15)
  - Per-project marker rewrite (loop project_path/workspace_projects, read + replace workspace name, persist atomically per-project)
  - Filesystem rename of `<root>/workspaces/<old>/` → `<new>/`
  - Central DB UPDATE to `workspaces.name` (single transaction)
  - Drift detection post-rename; emits `RenameOutcome` with project_count, manifest_hash, summary cache state
- **`remove(workspace_name, force, paths)`**:
  - Refuses reserved `global` (exit 15)
  - Refuses non-empty bind list unless `--force` (exit 16 `WorkspaceHasBoundProjects`); returns list of bound project paths
  - 5-step cascade per FR-405:
    1. Per-project teardown: for each bound project, read marker, resolve effective harness list, per-harness cleanup (respect shared paths)
    2. Per-project marker removal: delete `<project>/.tome/config.toml`
    3. Single DB transaction: delete `workspace_skills`, `workspace_catalogs`, `workspace_projects`, `workspaces` rows
    4. Delete central `<root>/workspaces/<name>/` directory
    5. Refcount cleanup: for each catalog URL once-referenced only by removed workspace, `remove_dir_all` cache clone
- **`regen(workspace_name, paths)`**:
  - Call summariser to generate short + long summaries from enabled plugins (Phase 4 skeleton invokes StubSummariser)
  - Write to workspace settings `[summaries]` section
  - Rewrite central `<root>/workspaces/<name>/RULES.md`
  - Per-project marker copy (RULES.md only; not atomically because RULES.md is read-only to projects)
- **`sync_one(workspace_name, paths)` + `list_workspace_names(paths)`**:
  - `sync_one`: Copy central RULES.md to every bound project's marker copy (idempotent, skip if already match)
  - `list_workspace_names`: Enumerate `<root>/workspaces/` and return Vec<WorkspaceName>

### Harness Abstraction (`src/harness/`)

- **Purpose**: Trait-driven dispatch to five supported harnesses (Claude Code, Codex, Cursor, Gemini, OpenCode)
- **Location**: `src/harness/{mod,claude_code,codex,cursor,gemini,opencode,rules_file,mcp_config,sync}.rs`
- **Phase 4 NEW**: Complete harness abstraction layer with per-harness `HarnessModule` impls + sync orchestrator
- **`HarnessModule` trait methods**:
  - Identity — `name()`, `description()`
  - Detection — `detect(home) -> bool` (existence-only per FR-167)
  - Rules integration — `rules_file_target()`, `rules_file_strategy()`, `block_body_style()`
  - MCP config — `mcp_config_path()`, `mcp_config_format()`, `mcp_parent_key()`
- **Key decisions** (per research §R-8):
  - Each harness owns a file under `src/harness/`; no per-harness subdirs in commands/
  - Rules strategies: block-in-file (Claude, Codex, Gemini, OpenCode) vs standalone (Cursor)
  - MCP config: JSON for most, TOML for Codex; stored per-project (Claude, Cursor, OpenCode) or global (Codex, Gemini)
- **Registry**: `SUPPORTED_HARNESSES` static + test override hook (`HARNESS_MODULES_OVERRIDE`)

### Harness Synchronization Orchestrator (`src/harness/sync.rs`)

- **Purpose**: Phase 4 / US1.b-c — Compute effective harness list, dispatch per-harness writes, run cleanup
- **Location**: `src/harness/sync.rs`
- **Key entry point**: `sync_project(project_root, sync_deps) -> Result<SyncOutcome, TomeError>`
- **Algorithm** (mirrors `contracts/sync-algorithm.md`):
  1. **Phase B0** (locked read, caller's responsibility): Project marker landed, DB UPSERT committed, lock released
  2. **Phase B1** (unlocked filesystem reads): Compose effective harness list from project marker + workspace settings + global settings (via `settings::resolve_effective_list`)
  3. **Phase B2** (unlocked filesystem writes): Dispatch per-harness rules-file and MCP-config writes with dedup on target path
  4. **Phase B3** (unlocked cleanup): For harnesses no longer in effective list, remove their on-disk config/entries (respecting shared-path dedup)
- **Multi-harness sharing** (FR-482/483): When two harnesses target same rules-file path or MCP config path, dedup the write (first touch records the harness name); cleanup pass respects shared paths
- **Forward progress on clash** (FR-403): If user-owned `tome` entry blocks an MCP write without `--force`, record the error but keep processing; first clash wins for overall `Result::Err` (exit 19), but rules-file writes for unaffected harnesses still happen
- **Dedup logic**: `BTreeMap<PathBuf, effective_harness_name>` for rules files; same for MCP configs; per-path FIFO on first writer

### Settings & Composition (`src/settings/`)

- **Purpose**: Parse and resolve layered harness selections across project/workspace/global scopes
- **Location**: `src/settings/{mod,composition,parser,resolver}.rs`
- **Phase 4 NEW**: Complete settings layer with composition reference support
- **Layers** (priority order; first match wins):
  1. Project marker — `<project>/.tome/config.toml` (`ProjectMarkerConfig`)
  2. Workspace settings — `<root>/workspaces/<name>/settings.toml` (`WorkspaceSettings`)
  3. Global settings — `<root>/settings.toml` (`GlobalSettings`)
- **Composition references**: `[workspace]`, `[global]`, `[workspaces.<name>]` — one level deep (not recursive)
- **Resolution**: `resolve_effective_list(project_root, workspace_name, paths, home) -> EffectiveHarnessList`
  - Reads project marker (if present), workspace settings (if present), global settings
  - Collects composition references and resolves them once (one-deep; no recursion)
  - Returns merged harness names with priority order
- **All types**: `#[serde(deny_unknown_fields)]` — Tome-owned inputs are strict per FR-013a boundary

### Commands Dispatcher (`src/commands/`)

- **Purpose**: Execute 11 CLI subcommands (catalog, plugin, models, query, reindex, status, workspace, harness, mcp, doctor)
- **Location**: `src/commands/{catalog,plugin,models,query,reindex,status,workspace,harness,mcp,doctor}.rs`
- **Pattern**: Most commands have:
  - `pub fn run(args, scope, mode)` — CLI entry with emit/exit
  - `pub fn pipeline(args, deps)` or `run_with_deps(...)` — silent compute for library reuse by MCP/tests
- **Phase 4 NEW**: `commands/harness/` thin seam (US1.a stub, US1.b real) dispatches to harness sync orchestrator
- **Phase 4 NEW**: `commands/workspace/` expands from 2 to 8 subcommands: `info/init/list/use_/rename/remove/regen_summary/sync`
- **Key invariant**: Lazy model loading (embedder/reranker not loaded on status/doctor/workspace unless needed)

### Workspace Command Suite (`src/commands/workspace/`)

- **Purpose**: Workspace management — full lifecycle from creation through removal
- **Location**: `src/commands/workspace/{info,init,list,use_,rename,remove,regen_summary,sync}.rs`
- **`info`** — `tome workspace info [<name>]` — read-only report of workspace details, plugin/skill counts, bound projects, summary cache state
  - Accepts optional `<name>` argument; defaults to resolved scope
  - New Phase 4 fields: `ScopeKind`, `project_count`, `summary_cache_state`, bound_project_list
- **`init`** — `tome workspace init <name> [--inherit-global] [--force]` — create new workspace in central registry
  - Lands `<root>/workspaces/<name>/settings.toml` + RULES.md atomically
  - Inserts row in central `workspaces` table
  - Optional catalog inheritance from global (enablement not copied)
  - Phase 4 NEW
- **`list`** — `tome workspace list` — enumerate every workspace with catalog/plugin/skill/project counts, last_used_at
  - Phase 4 NEW
- **`use_`** — `tome workspace use <name> [--force]` — bind current project to workspace, sync harnesses (Phase 4 / US1.a-c)
  - Calls `binding::bind_project` for Phase A (lock → DB → marker)
  - Calls `commands::harness::sync_for_project_root` for Phase B (harness writes)
  - Emits combined `BindOutcome` + `SyncOutcome` in JSON or human format
- **`rename`** — `tome workspace rename <old> <new>` — rename workspace, update all bound projects atomically
  - Refuses both `global` (exit 15)
  - Per-project marker rewrite + filesystem rename + DB update
  - Phase 4 NEW
- **`remove`** — `tome workspace remove <name> [--force]` — delete workspace with 5-step cascade
  - Refuses `global` (exit 15)
  - Refuses when projects bound unless `--force` (exit 16)
  - Per-project teardown, per-project marker cleanup, DB cascade, dir removal, refcount cleanup
  - Phase 4 NEW
- **`regen_summary`** — `tome workspace regen-summary <name>` — force regeneration of workspace summaries
  - Invokes summariser (StubSummariser in Phase 4 foundational)
  - Writes to workspace settings + central RULES.md
  - Copies RULES.md to every bound project marker
  - Phase 4 NEW
- **`sync`** — `tome workspace sync [<name>]` — copy central RULES.md to every bound project
  - Omit `<name>` to sync every workspace (idempotent, skip if bytes match)
  - Phase 4 NEW

### Central Index Database (`src/index/`)

- **Purpose**: Single SQLite database indexing all plugins, skills, embeddings, and workspace state
- **Location**: `src/index/{db,schema,skills,query,migrations}.rs`
- **Schema Version**: 2 (Phase 4)
- **Core tables**:
  - `meta` (STRICT) — embedder/reranker/summariser identity + drift detection
  - `workspaces` — registry of workspace names (id, name UNIQUE, created_at, last_used_at)
  - `skills` — catalog/plugin/skill metadata (id, catalog, plugin, name UNIQUE triple, content_hash, indexed_at)
  - `skill_embeddings` — sqlite-vec virtual table (skill_id PK, embedding FLOAT[384])
  - `workspace_skills` — junction table (workspace_id, skill_id) — enablement is presence of row
  - `workspace_catalogs` — junction table (workspace_id, catalog_name, url, pinned_ref)
  - `workspace_projects` — project-to-workspace bindings (project_path PK, workspace_id FK, bound_at)
- **Phase 4 changes**:
  - Moved from per-workspace DBs to single central DB
  - `skills.enabled` column dropped (enablement = presence of `workspace_skills` row)
  - New workspace/project tables for multi-workspace support
  - Catalog metadata now derived from filesystem + junction rows (not stored)
- **Concurrency**: Single advisory lockfile (`index.lock`) per-Paths; readers never acquire lock; schema migration + writes both acquire lock
- **Dependencies**: `rusqlite` + `sqlite-vec` extension (vendored C code)

### Embedding Pipeline (`src/embedding/`)

- **Purpose**: 384-dimensional text embedding + cross-encoder reranking for skill search
- **Location**: `src/embedding/{mod,fastembed,stub,registry,download,runtime}.rs`
- **Trait boundaries**:
  - `Embedder` trait — `embed(text) -> Vec<f32>` + identity (model name/version)
  - `Reranker` trait — `rerank(query, candidates) -> Vec<Scored>` + identity
- **Implementations**:
  - **Production**: `FastembedEmbedder` (ort-wrapped `fastembed-rs`; CPU-only), `FastembedReranker`
  - **Test**: `StubEmbedder`, `StubReranker` — deterministic, model-free
- **Model Registry**: Pinned BGE-small INT8 (45 MB, MIT), BGE-reranker INT8 (280 MB, MIT) with SHA-256 checksums
- **Download**: Atomic `reqwest::blocking` + `tempfile` + SHA-256 verify; sparse-file fixtures in tests

### Plugin Lifecycle (`src/plugin/lifecycle.rs`)

- **Purpose**: Enable/disable/reindex orchestrator composing manifest parse → embedding → index writes
- **Location**: `src/plugin/lifecycle.rs`
- **LifecycleDeps struct**: Input bundle wrapping `&Embedder`, config, scope, paths, seeds
- **Phase 4 changes**: Scope parameter is `&Scope` (not path-based); workspace_name resolved via `scope.name()`
- **Key invariants**:
  - Cheap re-enable: if `content_hash` matches, embedder not invoked; row updated with `UPDATE ... SET enabled = 1`
  - Per-plugin atomicity: each `enable_plugin_atomic` acquires its own advisory lock
  - Auto-disable on manifest-missing or plugin-not-found (reuses `CatalogNotFound` error per FR-602)

### Summariser (`src/summarise/`)

- **Purpose**: Generate short/long workspace summaries from enabled plugins via Qwen2.5-0.5B-Instruct GGUF
- **Location**: `src/summarise/{mod,llama,stub,registry,download,prompts}.rs`
- **Phase 4 NEW**: Skeleton shipped with placeholder registry entry
- **Architecture**:
  - `Summariser` trait — `summarise(PluginSummariesInput) -> Result<SummariserOutput, TomeError>`
  - **Production**: `LlamaSummariser` via `llama-cpp-2` + process-wide `LlamaBackend` singleton (OnceLock + mutex)
  - **Test**: `StubSummariser` — deterministic, no model load
- **Model**: Qwen2.5-0.5B-Instruct GGUF (placeholder SHA-256 in F6; real weight lands in US4.a)
- **Singleton pattern**: First `backend()` call initializes via mutex-gated OnceLock; subsequent calls hit lock-free path
- **Invocation**: Per-workspace, triggered by enable/disable/reindex/catalog-update; output cached in workspace settings

### Doctor Diagnostics (`src/doctor/`)

- **Purpose**: Broad health check + auto-repair for embedder/reranker/catalogs/schema/drift/bindings
- **Location**: `src/doctor/{mod,checks,fixes}.rs`
- **Key entry point**: `assemble_report(scope, paths, home, verify) -> DoctorReport`
- **Report fields**: Embedder health, reranker health, index integrity, drift, catalog cache state, harness presence, binding state, suggested fixes, overall classification
- **Classification**:
  - Unhealthy — embedder missing/corrupt, integrity fail, embedder drift
  - Degraded — reranker missing/corrupt, reranker drift, any catalog cache != Ok
  - Ok — everything passes
- **Auto-fixes** (routed by `subsystem` string):
  - `"embedder"` — `embedding::download::download_model`
  - `"reranker"` — same
  - `"catalog:<name>"` — `Git::clone_shallow`
  - `"schema"` — `index::migrations::apply_pending` under advisory lock
- **Binding subsystem** (Phase 4 NEW): Detects orphaned project markers (DB row missing but `.tome/config.toml` exists) + cross-workspace project markers (marker workspace != resolved workspace)
- **No side effects** on `assemble`; `fixes::apply` mutates in place; `re_assemble` rebuilds derived state

### MCP Server (`src/mcp/`)

- **Purpose**: Async stdio MCP server advertising two tools: `search_skills`, `get_skill`
- **Location**: `src/mcp/{mod,runtime,log,preflight,server,state,tools}.rs`
- **Async boundary**: Only module allowed to use `tokio` (enforced by `tests/sync_boundary.rs`)
- **Concurrency model**: Single-threaded tokio runtime per research §R-2
- **Key components**:
  - `runtime.rs` — entry point; builds `tokio::runtime::Runtime`, installs file log, runs preflight, blocks on `rmcp::serve_server`
  - `preflight.rs` — FR-110 pipeline: schema-version gate → drift detection → embedder SHA-256 verify → eager-load FastembedEmbedder
  - `log.rs` — 10 MiB atomic-rotate file log (JSON lines); stderr reserved for fatal startup errors only (FR-222)
  - `state.rs` — `McpState { embedder, reranker (OnceLock), scope, paths, ... }`
  - `tools/search_skills.rs`, `tools/get_skill.rs` — handlers with spawn_blocking for sync work
- **Tool handlers**: Validate input, lazy-load reranker via `OnceLock::get_or_try_init`, dispatch work inside `spawn_blocking`
- **Signal handling**: `tokio::signal::ctrl_c()` triggers graceful shutdown; 5 s timeout before hard shutdown

### Catalog Management (`src/catalog/`)

- **Purpose**: Register/list/update/remove external plugin catalogs from git repos
- **Location**: `src/catalog/{manifest,store,git}.rs`
- **Key invariants**:
  - On-disk clone cache at `<root>/catalogs/<sha256>/` (content-addressed)
  - Reference counting: `catalog::store::reference_count(url, paths) -> Vec<Scope>` determines cleanup eligibility
  - Credential scrubbing in git errors + model URLs (regex `[A-Za-z][A-Za-z0-9+.-]*://.*@`)
- **Manifest parsing**: `tome-catalog.toml` (strict, deny unknown fields)

### Configuration (`src/config.rs`)

- **Purpose**: Parse global `<root>/config.toml` — backward-compat layer for Phase 3 catalog enrolments (now moved to central DB junction)
- **Location**: `src/config.rs`
- **Type**: `Config` struct with `[catalogs]` table (read on commands that need catalog list)
- **Strictness**: `#[serde(deny_unknown_fields)]`

## Data Flow

### Primary User Flow: Bind a Project (Phase 4 / US1)

```
CLI: tome workspace use <workspace-name>
     ↓
Paths::resolve() — read $HOME, construct <home>/.tome/ paths
     ↓
Dangerous CWD check (refuse $HOME / / unless --force)
     ↓
index::open() with lock — acquire advisory lock
     ↓
workspace::binding::bind_project() — UPSERT into workspace_projects table
     ↓
Land <project>/.tome/config.toml with [workspace] = <name> atomically
     ↓
Release advisory lock
     ↓
commands::harness::sync_for_project_root() — PHASE B (unlocked)
  ↓
settings::resolve_effective_list(project, workspace, paths, home)
  ↓
harness::sync::sync_project() — per-harness rules-file + MCP-config writes
  ↓
Dedup on target paths; respect shared-path cleanup; forward-progress on clash
     ↓
CLI: print BindOutcome + SyncOutcome (added/updated/removed counts)
```

### Workspace Lifecycle Flow (Phase 4 / US2)

```
CLI: tome workspace init <name> | list | rename <old> <new> | remove <name> | regen-summary <name> | sync [<name>]
     ↓
Load central index (read-only for info/list/sync; write for init/rename/remove/regen-summary)
     ↓
Acquire lock if mutation (init/rename/remove create/update/delete in workspaces table)
     ↓
PHASE A (locked for mutations):
  - init: create workspace dir + settings skeleton + insert workspaces row
  - rename: per-project marker rewrites (unlocked after marker-time, before DB rename) + DB row update
  - remove: per-project teardown (unlocked per-project) → marker removal → DB cascade delete
  - regen-summary: invoke summariser → update workspace settings + central RULES.md + per-project copies
     ↓
PHASE B (unlocked):
  - sync: copy central RULES.md to every bound project marker (idempotent byte-match skip)
     ↓
Release lock; emit outcome (counts, project paths, summary cache state)
```

### Primary User Flow: Enable a Skill

```
CLI: tome plugin enable <catalog>/<plugin>
     ↓
Paths::resolve() — read $HOME, construct <home>/.tome/ paths
     ↓
workspace::resolution::resolve() — consult CLI flag / env / project marker / default
     ↓
index::open() — acquire advisory lock, check schema, load embedder/reranker identities from meta
     ↓
plugin::lifecycle::enable() — read plugin.json + SKILL.md frontmatter, compute embeddings
     ↓
index::skills::enable_plugin_atomic() — INSERT/UPDATE skills, skill_embeddings, workspace_skills junction rows
     ↓
Release advisory lock
     ↓
CLI: print summary (added/modified/unchanged skill counts)
```

### Search Flow: Query Skills

```
CLI: tome query "find a plugin that does X"
     ↓
workspace::resolution::resolve() → Scope(WorkspaceName)
     ↓
index::open_read_only() — open DB, don't take lock (readers ≠ writers)
     ↓
embedding::Embedder::embed(query) → Vec<f32> (384-dim)
     ↓
index::query::knn() — sqlite-vec KNN search filtered by workspace_skills junction (default k=20)
     ↓
embedding::Reranker::rerank(query, candidates) — cross-encoder scoring
     ↓
Sort by reranker score; emit human-readable or JSON
```

### MCP Tool Flow: search_skills Handler

```
MCP harness: POST {"jsonrpc": "2.0", "method": "tools/call", "params": {"name": "search_skills", ...}}
     ↓
preflight (one-time on MCP startup):
  - Schema version gate
  - Drift detection
  - SHA-256 verify embedder artefact
  - Eager-load FastembedEmbedder
     ↓
Handler: validate plugin_without_catalog / unknown_catalog
     ↓
Lazy-load reranker via OnceLock::get_or_try_init() in spawn_blocking
     ↓
Call query::pipeline() — compute embedding, KNN, rerank
     ↓
Map TomeError to MCP error envelope; emit JSON
```

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| CLI (commands/) | HTTP/prompt/exit handling | All business logic | Direct DB access (uses index:: API) |
| Business Logic (plugin/, embedding/, doctor/, workspace/, harness/) | Orchestration + decisions | Data access APIs (index::, catalog::, settings::) | CLI context (prompts, colors) |
| Index API (index/) | Query + CRUD on skills/meta/workspace tables | Database (rusqlite) | Business logic |
| Catalog/Config (catalog/, config.rs) | Manifest parsing, git operations | Filesystem, git CLI | Index operations |
| Harness (harness/) | Per-harness path/config strategy | Filesystem (existence probes only) | Nothing else |
| Settings (settings/) | Parse + compose harness lists | Filesystem (read TOML), workspace names | Database |
| Workspace (workspace/) | Scope resolution, binding, lifecycle | Catalog, config, index (read-only), settings | Commands |
| Output (output.rs) | JSON/human formatting, error display | Error enum | Any command logic |
| MCP (mcp/) | Async stdio protocol, tool dispatch | All other modules via spawn_blocking | Direct CLI context |
| Util (util/) | Shared helpers (atomic directories, etc.) | Filesystem, standard library | Any domain logic |

## Dependency Rules

- **Higher → Lower only**: Commands depend on business logic; business logic depends on data access; no upward references
- **Cross-layer**: `Paths` can be accessed anywhere (it's pure path construction); `TomeError` exported everywhere
- **Async island**: Only `src/mcp/` may import `tokio` (enforced by `tests/sync_boundary.rs`)
- **Lazy model loading**: Embedder/reranker loaded only when needed (not on status, doctor, query if cache hit)
- **Harness/Settings**: Settings composes harness names but never imports harness module (harness registry accessed via `with_effective_modules` callback)

## Key Interfaces & Contracts

| Interface | Purpose | Implementations |
|-----------|---------|-----------------|
| `Embedder` | Text → 384-dim embedding | `FastembedEmbedder` (prod), `StubEmbedder` (test) |
| `Reranker` | Query + candidates → scored results | `FastembedReranker` (prod), `StubReranker` (test) |
| `Summariser` | Plugin list → (short, long) summary | `LlamaSummariser` (prod), `StubSummariser` (test) |
| `HarnessModule` | Per-harness rules/MCP paths | `ClaudeCode`, `Codex`, `Cursor`, `Gemini`, `OpenCode`, `StubHarness` |
| `LifecycleDeps` | Input bundle for plugin enable/disable | Struct wrapping embedder, config, scope, paths |
| `BindDeps` | Input bundle for project binding | Struct wrapping paths, home_root |
| `SyncDeps` | Input bundle for harness sync | Struct wrapping paths, home_root, workspace_name, force |

## State Management

| State Type | Location | Pattern |
|------------|----------|---------|
| Central index | `<home>/.tome/index.db` | SQLite with WAL + advisory lockfile |
| Workspace registry | Central DB `workspaces` table | Primary key on `name` UNIQUE |
| Project bindings | Central DB `workspace_projects` table | project_path PRIMARY KEY → workspace_id FK |
| Catalog cache | `<home>/.tome/catalogs/<sha256>/` | Content-addressed, reference-counted cleanup |
| Models | `<home>/.tome/models/` | Atomic downloads via tempfile + SHA-256 |
| Project marker | `<project>/.tome/config.toml` | Thin TOML; workspace binding + project-scope harnesses |
| Workspace settings | `<home>/.tome/workspaces/<name>/settings.toml` | Layered harness list + cached summaries |
| Workspace RULES.md | `<home>/.tome/workspaces/<name>/RULES.md` | Central workspace rules, copied to projects |
| Global settings | `<home>/.tome/settings.toml` | Global harness fallback |
| MCP log | `<home>/.tome/mcp.log` | 10 MiB atomic-rotate JSON lines |
| Per-harness config | Harness-specific paths (e.g. `<project>/.claude/tools.md`, `<home>/.codex/`) | Rules files + MCP config, written by sync orchestrator |

## Cross-Cutting Concerns

| Concern | Implementation | Location |
|---------|----------------|----------|
| Logging | `tracing` + `tracing-subscriber` (stderr only; MCP uses file) | `src/logging.rs` |
| Error handling | Closed `TomeError` enum + exit codes (1–80+) | `src/error.rs` |
| Credential scrubbing | Regex remove `scheme://...@host` from strings | `src/catalog/git.rs` |
| Signal handling | `ctrlc` on CLI (SIGINT → exit 8); tokio signal on MCP (graceful shutdown) | `src/main.rs`, `src/mcp/runtime.rs` |
| Output formatting | `--json` mode (serde + anstream) vs human (colors, tables, spinner) | `src/output.rs`, `src/presentation/` |
| Advisory locking | Single lockfile per Paths; database transaction per critical section | `src/index/lock.rs` |
| Atomic directory landing | Tempfile staging dir → rename on success | `src/util/atomic_dir.rs` |
| Atomic file writes | `catalog::store::write_atomic` — harness rules/MCP config writes + all workspace TOML updates | `src/catalog/store.rs` |

## Testing Architecture

- **Unit tests**: Embedded in modules (e.g., `src/error.rs`, `src/settings/mod.rs`)
- **Integration tests**: Under `tests/` — access library via public API, no `#[cfg(test)]` visibility
- **Test fixtures**: Synthetic DB builders, sparse-file models, git repos via `git init`
- **Stub implementations**: `StubEmbedder` / `StubReranker` / `StubSummariser` / `StubHarness` with deterministic output for reproducible tests
- **Test injection**: Thread-local `MIGRATIONS_OVERRIDE`, `HARNESS_MODULES_OVERRIDE` via RAII guards
- **Sync boundary**: `tests/sync_boundary.rs` enforces no `tokio` outside `src/mcp/`

---

*This document describes HOW the system is organized. Keep focus on patterns and relationships.*

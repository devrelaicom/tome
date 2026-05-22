# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-14
> **Last Updated**: 2026-05-14

## Architecture Overview

Tome is a synchronous CLI application (with an async MCP server island) designed to make Claude Code's plugin ecosystem work across other agentic coding harnesses. It provides:

- **Catalog management**: register and update third-party plugin catalogs (git repositories)
- **Plugin lifecycle**: enable/disable plugins and reindex skills for vector search
- **Skill search**: query-time KNN + reranking across enabled plugins
- **Workspace isolation**: per-project and global plugin/model state
- **Diagnostics**: health checks and automated repair via `tome doctor`
- **MCP interoperability**: stdio MCP server for non-Claude harnesses to query skills

The codebase is organized into **capability modules** (not layered): each module is responsible for a distinct piece of functionality, with clear dependency boundaries and a synchronous foundation except for the MCP server which lives in a structurally-enforced async island at `src/mcp/`.

## Architecture Pattern

| Pattern | Description |
|---------|-------------|
| **Capability-organized modules** | `catalog/`, `plugin/`, `index/`, `embedding/`, `workspace/`, `doctor/`, `mcp/` — each module owns its domain without bleed |
| **Closed error set** | `TomeError` enum with 26+ variants (codes 1–8, 20–23, 30–37, 40–42, 50–54, 60–61, 70–75) — adds exit-code specificity; new variants force test + PRD updates |
| **Sync-first with async island** | All work is synchronous; `src/mcp/` is the single place where `tokio::`, `.await`, and `async fn` are permitted — enforced by `tests/sync_boundary.rs` |
| **Silent compute + emit wrapper** | Command pipelines separate pure compute (`fn pipeline(args, deps) -> Result<Outcome>`) from I/O side-effects (emit, exit) — enables library reuse by non-CLI surfaces (MCP, RPC) |
| **Per-scope config + shared index** | Workspaces and global scope each maintain their own `config.toml`; the index DB is scoped by `${state_dir}/index.db` matching the resolved scope |
| **Content-addressed shared resources** | On-disk catalog clones are SHA256-addressed; reference counting via `catalog::store::reference_count` prevents orphan cleanup when multiple scopes reference the same URL |
| **Advisory lock + SQLite WAL** | All index DB mutations acquire an advisory lockfile (`index.lock`); WAL mode enables concurrent readers |
| **Test-only `thread_local!` injection** | Framework modules expose `#[doc(hidden)] pub static MIGRATIONS_OVERRIDE: thread_local!` for synthesis; production code ignores the override |

## Core Components

### `catalog/` — Catalog Registry

- **Purpose**: Register, update, and cache third-party plugin catalogs (git repositories). Catalog metadata lives in `${state_dir}/catalogs/` as TOML files; actual plugin trees are cloned to `${cache_dir}/catalogs/` by content hash.
- **Location**: `src/catalog/`
- **Dependencies**: `git` (shell), `serde`/`toml`, `tempfile` (atomic writes)
- **Key Types**:
  - `CatalogManifest` (`tome-catalog.toml` — strict parsing)
  - `CatalogEntry` (in-memory representation with URL + ref)
- **Key Functions**:
  - `store::save_atomic` — atomic TOML write via tempfile
  - `store::reference_count(url, paths) -> Vec<Scope>` — walk every scope's config to find who references a URL
  - `git::clone_shallow` — fetch a specific ref, credential-scrub errors
  - `git::scrub_credentials` — redact secrets from error chains
- **Dependents**: Every command that lists/enables plugins, catalog-update workflows

### `plugin/` — Plugin Metadata & Lifecycle

- **Purpose**: Parse plugin manifests (`plugin.json`), skill frontmatter (SKILL.md YAML headers), enumerate plugin components (skills, agents, commands, hooks), and orchestrate enable/disable transitions.
- **Location**: `src/plugin/`
- **Dependencies**: `serde` (lenient), `time` (timestamps)
- **Key Types**:
  - `PluginId` (parsed `<catalog>/<plugin>`)
  - `PluginManifest` (lenient)
  - `SkillFrontmatter` (lenient + fallback defaults)
  - `PluginRecord` (aggregated view: manifest + index state)
  - `ComponentCounts` (skills/agents/commands/hooks tally)
- **Key Functions**:
  - `lifecycle::enable(plugin_id, deps)` → embedder call + index insert
  - `lifecycle::disable(plugin_id, deps)` → index delete
  - `lifecycle::reindex_plugin(id, deps, force)` → re-embed modified skills
  - `lifecycle::cascade_disable_for_catalog(catalog_id, deps)` → disable all enabled plugins in one lock window
  - `frontmatter::parse_skill_frontmatter(path)` → extract title/description from SKILL.md
- **Dependents**: `commands/plugin/`, `index/skills`, MCP tools

### `index/` — Vector Search Index (SQLite + sqlite-vec)

- **Purpose**: Persist indexed skills with embeddings, reranker scores, and metadata. Schema versioning + forward-only migrations.
- **Location**: `src/index/`
- **Dependencies**: `rusqlite`, `sqlite-vec` C extension (vendored), `sha2`/`hex` (content hash), `tempfile` (atomic lock), `serde`
- **Key Types**:
  - `SkillRecord` (persisted: plugin, skill name, enabled, embedding, reranker score)
  - `MetaKey` (config keys: `embedder_name`, `reranker_name`, `reranker_version`)
  - `DriftStatus` (embedder/reranker mismatch detection)
  - `Candidate` (KNN result: plugin id, skill name, similarity)
  - `Migration { from, to, name, apply }` (forward-only schema change)
- **Key Functions**:
  - `db::open(paths, scope)` → acquire advisory lock, forward-migrate, return handle
  - `db::open_read_only(paths, scope)` → open without lock (readers don't block writers)
  - `migrations::apply_pending(conn, current, target)` → apply sequence of registered migrations
  - `skills::enable_plugin_atomic(plugin_id, skills, embedder)` → single transaction: insert/update rows, detect content-hash-unchanged for cheap re-enable
  - `skills::reindex_plugin_atomic(plugin_id, embedder, reranker)` → classify each skill as Added/Modified/Removed, update selectively
  - `query::knn(conn, query_embedding, filters, reranker)` → search by vector + optional rerank
  - `integrity::check(conn)` → `PRAGMA integrity_check`
  - `meta::detect_drift(conn, embedder_name, reranker_name)` → compare stored vs configured identities
- **Dependents**: Plugin lifecycle, query, status/doctor, MCP tools

### `embedding/` — Model Management & Inference

- **Purpose**: Download, verify, and invoke BGE embedder (bge-small-en-v1.5) and reranker (bge-reranker-base). Single embedder; lazy reranker load.
- **Location**: `src/embedding/`
- **Dependencies**: `fastembed-rs` (wrapping `ort`/ONNX), `reqwest::blocking`, `sha2`, `indicatif` (progress), `serde`
- **Key Types**:
  - `Embedder` trait (sync `fn embed(&self, text: &str) -> Result<Vec<f32>>`)
  - `Reranker` trait (sync `fn rerank(&self, query: &str, candidates: &[&str]) -> Result<Vec<Score>>`)
  - `FastembedEmbedder` (production impl via fastembed-rs)
  - `StubEmbedder` (deterministic test impl, zero models)
  - `ModelRegistry` entry (name, version, url, sha256)
- **Key Functions**:
  - `download::download_model(model_name, paths)` → fetch + verify via SHA-256 + atomic persist
  - `download::sha256_file(path)` → streaming chunked hash (used by `models list --verify`)
  - `fastembed::new() -> Result<Arc<dyn Embedder>>` → initialize ONNX model at `${XDG_DATA_HOME}/tome/models/`
  - `stub::new() -> Arc<dyn Embedder>` (cfg test)
- **Dependents**: Plugin enable/reindex, query, MCP preflight + tools

### `workspace/` — Scope & Context Resolution (Phase 3)

- **Purpose**: Represent local (`.tome/`) vs global plugin/model state; resolve which scope applies to a command; manage opt-in workspace registry at `${state_dir}/workspaces.txt`.
- **Location**: `src/workspace/`
- **Dependencies**: `serde`/`toml`, `tempfile`, `directories`
- **Key Types**:
  - `Scope` (enum: `Global` | `Workspace(PathBuf)`)
  - `ResolvedScope` (determined scope + source: `--workspace` flag, `--global` flag, env, cwd walk, fallback)
  - `ScopeSource` (enum variants serialized for JSON: `Flag`, `GlobalFlag`, `Env`, `CwdWalk`, `GlobalFallback`)
  - `Paths` (updated with per-scope accessors: `config_file_for(&scope)`, `index_db_for(&scope)`, etc.)
  - `WorkspaceInfo` (read-only report: catalog count, model state, index facts, schema version)
  - `InitOutcome` (result of `tome workspace init`)
- **Key Functions**:
  - `resolution::resolve(scope_args) -> Result<ResolvedScope>` → check `--workspace`/`--global` mutual exclusivity (exit 72), walk cwd for `.tome/`, fallback global
  - `init::init(target, inherit_global, force, paths)` → atomic directory landing via sibling staging + rename
  - `info::assemble(scope, paths)` → pure compute (no I/O side-effects) for `workspace info` command + JSON
  - `inventory::append_if_registry_exists(workspaces_txt_path, item)` → append if file exists (opt-in)
- **Dependents**: `main.rs` (resolution happens first), every command (threads `ResolvedScope` through), workspace commands

### `doctor/` — Diagnostic & Auto-Repair (Phase 3 US4)

- **Purpose**: Comprehensive health check across models, index, catalogs, harness presence, drift; suggest fixes; apply auto-fixable repairs.
- **Location**: `src/doctor/`
- **Dependencies**: Everything (reads every subsystem)
- **Key Types**:
  - `DoctorReport` (embedder/reranker health, index integrity, drift, catalogs, harnesses, suggested_fixes, overall classification)
  - `CatalogCacheState` (enum: `Ok` | `Missing` | `NotARepo` | `ManifestInvalid` | `Orphan` — new in P3 Polish)
  - `CatalogCacheHealth` (per-catalog result)
  - `HarnessPresence` (existence check for `.claude/`, `.codex/`, `.cursor/`, `.gemini/`, `.opencode/`, `.continue/`)
  - `DoctorClassification` (enum: `Ok` | `Degraded` | `Unhealthy`)
  - `SuggestedFix` (auto_fixable bool, subsystem string: `"embedder"`, `"reranker"`, `"catalog:<name>"`, `"schema"`)
- **Key Functions**:
  - `assemble_report(scope, paths, home, verify) -> DoctorReport` — silent compute (no emit)
  - `checks::check_catalogs(paths, scope)` → walk `${cache_dir}/catalogs/` on disk
  - `checks::check_workspace_registry(paths)` → validate 1 MiB size cap, 10k entry cap
  - `harness_detect::probe(home)` → existence-only check for 6 harness dirs
  - `fixes::apply(report, paths, scope)` → run each auto_fixable fix, re-run check inline
  - `re_assemble(report)` → recompute suggested_fixes + overall without re-probing
- **Dependents**: `commands/doctor`, `mcp/preflight`

### `mcp/` — MCP Server (Phase 3 US1, async island)

- **Purpose**: Stdio MCP server exposing two tools: `search_skills` (KNN+rerank) and `get_skill` (metadata + components).
- **Location**: `src/mcp/` — **only place async/await is allowed**
- **Dependencies**: `rmcp`, `tokio` (single-threaded), `tracing` (file subscriber), `tempfile`
- **Key Types**:
  - `McpState` (Arc: embedder, reranker `OnceCell`, scope, paths, embedder_entry, reranker_entry)
  - `Server` (rmcp `ToolRouter` impl)
  - `SearchSkillsInput` / `GetSkillInput` (request schemas via `schemars`)
- **Submodules**:
  - `runtime.rs` — single-threaded tokio builder
  - `log.rs` — 10 MiB atomic-rotate JSON-lines file logger + `ContractEventFormat` (custom tracing event format pins field names to contract: `ts` not `timestamp`, `msg` not `message`)
  - `preflight.rs` — FR-110 startup checks (schema version, embedder load + hash, reranker deferred)
  - `server.rs` — rmcp server loop + graceful shutdown (5 s timeout on SIGTERM/SIGINT)
  - `state.rs` — `McpState` definition
  - `tools/search_skills.rs` — tool handler (validates plugin/catalog, lazy-loads reranker, invokes `query::pipeline`)
  - `tools/get_skill.rs` — tool handler (looks up plugin, reads SKILL.md, walks skill dir recursively)
- **Key Functions**:
  - `run(scope, paths) -> Result<()>` — sync entry from CLI, builds runtime, calls async loop
  - `preflight::run(scope, paths) -> Result<EmbedderHandle>` — sync checks on blocking pool
- **Dependents**: `commands/mcp`, `main.rs` (special-case: skip logging init, skip ctrlc handler)

### `commands/` — CLI Entry Points

- **Purpose**: Dispatch every user-facing command; apply per-command argument parsing; wire library functions to I/O (stdout, exit codes).
- **Location**: `src/commands/`
- **Submodules**:
  - `catalog.rs` — `add`, `remove`, `list`, `update`, `show`; Phase 2 foundational + Phase 3 US3 refcount
  - `plugin/` — `enable`, `disable`, `list`, `show`, `interactive`; bare `tome plugin` → three-level TUI
  - `models/` — `download`, `list`, `remove`; Phase 2 US4
  - `query.rs` — `tome query` with KNN + reranker + `--strict`; `pipeline` export for MCP reuse (Phase 3 US1.b)
  - `reindex.rs` — `tome reindex [<scope>] [--force]`; `run_with_deps` library entry (Phase 3 US5)
  - `status.rs` — `tome status [--verify] [--json]`; read-only pre-flight; `--version` hook
  - `workspace/` — `info`, `init` (Phase 3 US2)
  - `doctor.rs` — `tome doctor [--fix] [--verify]`; thin wrapper over doctor library (Phase 3 US4)
  - `mcp.rs` — `tome mcp` (Phase 3 US1)
- **Pattern**: Each command typically has `pub fn run(args, scope, mode) -> Result<Outcome>` (emit-path) + `pub fn run_with_deps(args, scope, deps, mode) -> Result<Outcome>` or `pub fn pipeline(args, deps) -> Result<Outcome>` (silent-compute for library reuse)

### `presentation/` — Output Formatting & TUI

- **Purpose**: Tables, progress spinners, colored output, TTY-aware prompts.
- **Location**: `src/presentation/`
- **Submodules**:
  - `tables.rs` — `comfy-table` helpers (plugin lists, skill results)
  - `progress.rs` — `indicatif` spinners for download/embed
  - `colour.rs` — `owo-colors` + `NO_COLOR` detection
  - `prompt.rs` — `inquire` select/confirm/multi-select (refuse on non-TTY)
- **Dependents**: Every command that outputs or prompts

### `output.rs` — JSON / Human Modes

- **Purpose**: Single write_* interface supporting both human text and machine-readable JSON.
- **Key Types**:
  - `OutputMode` (enum: `Human` | `Json`)
- **Key Functions**:
  - `write(mode, value)` where value is `Serialize`
  - `write_error(mode, error)`

### Other Core Modules

- **`config.rs`** — `config.toml` (strict `#[serde(deny_unknown_fields)]`)
- **`paths.rs`** — XDG paths (refactored Phase 3 F1 to support per-scope accessors)
- **`logging.rs`** — `tracing-subscriber` stderr setup (skipped on MCP path)
- **`cli.rs`** — `clap` derive defs for all commands + global `--json`, `-v`, `--workspace`, `--global`
- **`error.rs`** — Closed `TomeError` enum with exit codes (26+ variants)
- **`catalog/git.rs`** — Shell-out to system `git`, credential scrubbing

## Data Flow

### `tome plugin enable` Flow

```
CLI args → scope resolution
          ↓
plugin::identity::parse() → PluginId
          ↓
catalog::store::load() → resolve catalog + plugin dir
          ↓
plugin::manifest::parse_plugin_manifest() → PluginManifest
          ↓
plugin::components::walk() → list of skills
          ↓
embedding::fastembed::new() [lazy load, cached] → Arc<dyn Embedder>
          ↓
index::db::open() [acquire advisory lock, migrate]
          ↓
plugin::frontmatter::parse_skill_frontmatter() × N → SkillFrontmatter array
          ↓
for each skill:
  embedder.embed(text) → Vec<f32>
  ↓
index::skills::enable_plugin_atomic() [one transaction]
  insert into skills, skill_embeddings
  ↓
return EnableOutcome (count_enabled, count_had_error)
          ↓
emit human or JSON output
```

### `tome query` Flow

```
CLI args (query text, --catalog filter, --strict threshold)
          ↓
embedding::fastembed::new() [lazy load]
          ↓
embedder.embed(query_text) → Vec<f32>
          ↓
index::db::open_read_only() [no lock needed]
          ↓
index::query::knn(embedding, filters) → Vec<Candidate>
          ↓
embedding::reranker = lazy::load() [on demand]
          ↓
reranker.rerank(query, skill_texts) → Vec<(candidate, score)>
          ↓
--strict threshold filter
          ↓
emit as table or JSON
```

### `tome workspace info` Flow

```
CLI arg (optional --workspace <path>)
          ↓
workspace::resolution::resolve() → ResolvedScope
          ↓
workspace::info::assemble(scope, paths) [pure compute, no emit]
  catalog count from resolved config.toml
  model state from embedding registry + disk
  index state from read-only DB open
  schema version from meta table
          ↓
emit WorkspaceInfo (human or JSON) → exit 0
```

### `tome doctor --fix` Flow

```
doctor::assemble_report(scope, paths, home, verify) → DoctorReport
  check models, index, drift, catalogs, harnesses
  build suggested_fixes by classification
  classify overall (Ok / Degraded / Unhealthy)
          ↓
if --fix:
  for each suggested_fix where auto_fixable:
    fixes::apply_one()
    re-check that subsystem inline
    update report in place
  ↓
  doctor::re_assemble() [recompute suggested_fixes + overall]
          ↓
emit DoctorReport + exit (0 / 1 / 75)
```

### MCP `search_skills` Tool Flow

```
rmcp receive JSON-RPC call → SearchSkillsInput
          ↓
validate plugin_without_catalog / unknown_catalog
          ↓
state.reranker.get_or_try_init() [lazy load via spawn_blocking]
          ↓
spawn_blocking {
  query::pipeline(args, deps) [pure compute]
    embed query
    KNN search
    rerank
    return Vec<Candidate>
  }
          ↓
emit JSON array of results
```

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| **CLI (`commands/`)** | Parse args, emit output, exit | Libraries, library helpers | Nothing reaches back to CLI |
| **Library (core modules)** | Silent compute, state mutation | Database, embedder, reranker, catalog | Output/emit, process exit (return Result instead) |
| **Index (`index/`)** | Query, schema, lock, skills table | SQLite, migrations | Commands, embedding |
| **Embedding (`embedding/`)** | Model download, inference, reranking | File I/O, `ort`, `fastembed` | Index, plugin, catalog |
| **Plugin (`plugin/`)** | Metadata parsing, lifecycle | Index, embedding, catalog | CLI output |
| **Catalog (`catalog/`)** | Registry persistence, git | Git shell, file I/O | Index directly, plugin enable |
| **Workspace (`workspace/`)** | Scope resolution, context | Catalog, config, paths | Commands (except scope resolution in main.rs) |
| **MCP (`src/mcp/`, async)** | Server loop, tool dispatch | Everything via library APIs | Process management (return error to harness) |

**Dependency Rule**: No circular dependencies. Higher-level modules (CLI commands) depend on lower-level modules (core libraries); reverse never happens.

## Dependency Rules

- **Higher can depend on lower**: Commands depend on libraries; libraries don't depend on commands.
- **No circular imports**: Each module explicitly lists its dependencies in the submodule tree.
- **Sync-only except `src/mcp/`**: The boundary is structurally enforced — `tests/sync_boundary.rs` fails the build if any file outside `src/mcp/` uses `tokio::` or `.await`.
- **Loose coupling across domains**: `catalog/`, `plugin/`, `index/` are independently testable; integration happens at the command layer.

## Key Interfaces & Contracts

| Interface | Purpose | Implementations |
|-----------|---------|-----------------|
| `Embedder` trait | Embed text → Vec<f32> | `FastembedEmbedder` (production), `StubEmbedder` (tests) |
| `Reranker` trait | Rerank candidates | `FastembedReranker` (production), `StubReranker` (tests) |
| `PluginId` | Parsed `<catalog>/<plugin>` | Parse via `identity::parse`, normalize, validate |
| `TomeError` enum | Closed error set with exit codes | 26+ variants, one-to-one mapping to exit codes |
| `ResolvedScope` | Resolved workspace vs global | Determined at CLI entry, threaded through every command |
| `DoctorReport` | Diagnostic output | Built by `assemble_report`, mutated by `fixes::apply`, re-assembled by `re_assemble` |

## State Management

| State Type | Location | Lifetime | Pattern |
|------------|----------|----------|---------|
| **Plugin registry** | `${state_dir}/catalogs/` (TOML) | Persistent | Write via `catalog::store::save_atomic` |
| **Catalog clones** | `${cache_dir}/catalogs/<sha256>/` | Persistent until refcount→0 | Reference-counted by `store::reference_count` |
| **Index DB** | `${state_dir}/index.db` | Persistent | Advisory lock + WAL |
| **Models** | `${XDG_DATA_HOME}/tome/models/` | Persistent | Atomic persist + SHA-256 verify |
| **MCP log** | `${state_dir}/tome/mcp.log` | Persistent, 10 MiB rotation | Atomic rename on rotate |
| **Workspace registry** | `${state_dir}/workspaces.txt` | Persistent, opt-in | Append-only, dedupe by canonicalize |
| **Embedder (in-process)** | `Arc<dyn Embedder>` in `McpState` | Per-MCP-server-lifetime | Loaded once on preflight |
| **Reranker (lazy)** | `OnceCell<Arc<dyn Reranker>>` in `McpState` | Lazily loaded on first query | `get_or_try_init` + `spawn_blocking` |

## Cross-Cutting Concerns

| Concern | Implementation | Location |
|---------|----------------|----------|
| **Logging** | `tracing-subscriber` stderr (`info!`, `error!`) + MCP file log | `src/logging.rs`, `src/mcp/log.rs` |
| **Error handling** | Closed `TomeError` enum, `thiserror` at module level, `anyhow` at CLI | `src/error.rs` + per-module error types |
| **Signal handling** | `ctrlc` SIGINT (CLI), `tokio::signal` SIGINT/SIGTERM (MCP) | `src/catalog/git.rs`, `src/mcp/mod.rs` |
| **Atomic writes** | `tempfile::NamedTempFile` → persist or rollback | `catalog/store.rs`, `workspace/init.rs`, `mcp/log.rs` |
| **Concurrency** | Advisory lockfile (CLI) + SQLite WAL (readers don't block writers) | `index/lock.rs`, `index/db.rs` |
| **Credential scrubbing** | Regex redaction of secrets in error chains | `catalog/git.rs`, `mcp/mod.rs` (workspace_path, error_message fields) |
| **Test isolation** | `StubEmbedder`, `MIGRATIONS_OVERRIDE` thread_local, `home: &Path` parameter | `embedding/stub.rs`, `index/migrations.rs`, `doctor/mod.rs` |

---

## What Does NOT Belong Here

- Directory structure details → STRUCTURE.md
- Technology versions → STACK.md
- External service configs → INTEGRATIONS.md
- Code style rules → CONVENTIONS.md

---

*This document describes HOW the system is organized. Keep focus on patterns and relationships.*

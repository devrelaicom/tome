# Project Structure

> **Purpose**: Document directory layout, module boundaries, and where to add new code.
> **Generated**: 2026-05-25
> **Last Updated**: 2026-05-25

## Directory Layout

```
tome/
├── src/                                # Rust library + binary source
│   ├── main.rs                         # CLI entry: scope resolution, command dispatch, error mapping
│   ├── lib.rs                          # Public exports
│   ├── cli.rs                          # clap derive defs (all commands + global flags)
│   ├── error.rs                        # Closed TomeError enum (30+ variants → exit codes)
│   ├── config.rs                       # config.toml parsing (strict; legacy Phase 3 shape)
│   ├── paths.rs                        # Phase 4: consolidated <home>/.tome/ paths (no XDG split)
│   ├── logging.rs                      # tracing-subscriber wiring
│   ├── output.rs                       # JSON / human output mode dispatcher
│   │
│   ├── catalog/                        # Catalog registry + git ops
│   │   ├── mod.rs                      # Public API
│   │   ├── manifest.rs                 # tome-catalog.toml parsing (strict)
│   │   ├── store.rs                    # Registry persistence + reference counting + write_atomic
│   │   └── git.rs                      # Shell git ops + credential scrubbing
│   │
│   ├── plugin/                         # Plugin metadata + lifecycle
│   │   ├── mod.rs                      # PluginRecord, PluginStatus
│   │   ├── manifest.rs                 # plugin.json parsing (lenient)
│   │   ├── frontmatter.rs              # SKILL.md YAML frontmatter parser
│   │   ├── identity.rs                 # PluginId: <catalog>/<plugin> parsing
│   │   ├── components.rs               # Walk skill/agent/command/hook dirs
│   │   └── lifecycle.rs                # enable/disable/reindex orchestration (per-scope)
│   │
│   ├── index/                          # Vector search index (SQLite + sqlite-vec)
│   │   ├── mod.rs                      # Public API exports
│   │   ├── db.rs                       # Open, WAL config, schema version check
│   │   ├── schema.rs                   # CREATE TABLE statements + bootstrap (schema v2)
│   │   ├── migrations.rs               # Forward-only schema migrations + framework
│   │   ├── vec_ext.rs                  # sqlite-vec extension loader
│   │   ├── skills.rs                   # Skills table CRUD + content-hash diffing
│   │   ├── query.rs                    # KNN search (workspace-filtered) + optional reranking
│   │   ├── meta.rs                     # Model identity metadata + drift detection
│   │   ├── integrity.rs                # PRAGMA integrity_check wrapper
│   │   ├── lock.rs                     # Advisory lockfile acquisition
│   │   └── workspace_catalogs.rs       # Phase 4: junction table CRUD (workspace → catalogs)
│   │
│   ├── embedding/                      # Model management + inference
│   │   ├── mod.rs                      # Embedder/Reranker/Scored traits
│   │   ├── fastembed.rs                # FastembedEmbedder impl via fastembed-rs
│   │   ├── stub.rs                     # StubEmbedder (cfg test)
│   │   ├── registry.rs                 # Pinned MODEL_REGISTRY (URLs + SHA-256)
│   │   ├── download.rs                 # Model fetch + verify + atomic persist
│   │   └── runtime.rs                  # ort Environment singleton setup
│   │
│   ├── workspace/                      # Scope + context resolution + binding + lifecycle (Phase 3-4)
│   │   ├── mod.rs                      # Public API exports
│   │   ├── scope.rs                    # Phase 4: Scope(WorkspaceName) tuple struct
│   │   ├── name.rs                     # WorkspaceName validation + parsing
│   │   ├── resolution.rs               # Workspace vs global determination
│   │   ├── binding.rs                  # Phase 4: Project binding + marker landing (US1.a)
│   │   ├── info.rs                     # WorkspaceInfo report assembly
│   │   ├── init.rs                     # Atomic workspace creation via tempfile
│   │   ├── regen_summary.rs            # Phase 4 NEW: Summariser invocation (US2)
│   │   ├── rename.rs                   # Phase 4 NEW: Workspace rename with project updates (US2)
│   │   ├── remove.rs                   # Phase 4 NEW: Workspace removal with 5-step cascade (US2)
│   │   └── sync.rs                     # Phase 4 NEW: Central RULES.md sync to projects (US2)
│   │
│   ├── doctor/                         # Diagnostic + auto-repair (Phase 3 US4)
│   │   ├── mod.rs                      # assemble_report + re_assemble entry
│   │   ├── checks.rs                   # check_catalogs, check_index, check_drift
│   │   ├── harness_detect.rs           # Probe ~/.claude/, ~/.codex/, ~/.cursor/, etc.
│   │   └── fixes.rs                    # apply + auto-fix dispatch (subsystem routing)
│   │
│   ├── harness/                        # Phase 4: Per-harness trait + sync orchestrator
│   │   ├── mod.rs                      # HarnessModule trait, SUPPORTED_HARNESSES registry
│   │   ├── claude_code.rs              # Claude Code harness impl
│   │   ├── codex.rs                    # Codex harness impl
│   │   ├── cursor.rs                   # Cursor harness impl
│   │   ├── gemini.rs                   # Gemini CLI harness impl
│   │   ├── opencode.rs                 # OpenCode harness impl
│   │   ├── rules_file.rs               # Block-in-file + standalone strategies + atomic_write
│   │   ├── mcp_config.rs               # JSON + TOML MCP config read/write primitives
│   │   ├── sync.rs                     # Phase 4: Sync orchestrator (per-project harness writes)
│   │   └── stub.rs                     # StubHarnessModule for test injection
│   │
│   ├── settings/                       # Phase 4: Layered harness composition
│   │   ├── mod.rs                      # Type defs (ProjectMarkerConfig, WorkspaceSettings, GlobalSettings)
│   │   ├── parser.rs                   # TOML deserialization (strict)
│   │   ├── composition.rs              # CompositionRef + reference parsing
│   │   └── resolver.rs                 # Resolve effective harness list (priority walk + composition refs)
│   │
│   ├── summarise/                      # Phase 4: Workspace summariser (skeleton)
│   │   ├── mod.rs                      # Summariser trait + input/output types
│   │   ├── llama.rs                    # LlamaSummariser (production, llama-cpp-2)
│   │   ├── stub.rs                     # StubSummariser (deterministic test impl)
│   │   ├── registry.rs                 # Pinned summariser model (Qwen2.5-0.5B)
│   │   ├── prompts.rs                  # Prompt templates + length constraints
│   │   └── download.rs                 # Model fetch (stub-only in F6)
│   │
│   ├── commands/                       # CLI command entry points
│   │   ├── mod.rs                      # Public API exports
│   │   ├── catalog.rs                  # `tome catalog {add,remove,list,update,show}`
│   │   ├── plugin/                     # `tome plugin` subcommands
│   │   │   ├── mod.rs                  # Dispatcher + shared helpers
│   │   │   ├── enable.rs               # `tome plugin enable <id>`
│   │   │   ├── disable.rs              # `tome plugin disable <id> [--force]`
│   │   │   ├── list.rs                 # `tome plugin list`
│   │   │   ├── show.rs                 # `tome plugin show <id>`
│   │   │   └── interactive.rs          # Bare `tome plugin` → three-level TUI
│   │   ├── models/                     # `tome models` subcommands
│   │   │   ├── mod.rs                  # Dispatcher + shared helpers
│   │   │   ├── download.rs             # `tome models download [<name>]`
│   │   │   ├── list.rs                 # `tome models list [--verify]`
│   │   │   └── remove.rs               # `tome models remove <name> [--force]`
│   │   ├── query.rs                    # `tome query [<text>]` + --catalog, --strict, --plain
│   │   ├── reindex.rs                  # `tome reindex [<scope>] [--force]`
│   │   ├── status.rs                   # `tome status [--verify]` + --version hook
│   │   ├── workspace/                  # `tome workspace` subcommands (Phase 4 US2)
│   │   │   ├── mod.rs                  # Dispatcher (8 subcommands)
│   │   │   ├── info.rs                 # `tome workspace info [<name>]` — read-only report
│   │   │   ├── init.rs                 # `tome workspace init <name> [--inherit-global] [--force]`
│   │   │   ├── list.rs                 # `tome workspace list` — enumerate all workspaces
│   │   │   ├── use_.rs                 # `tome workspace use <name> [--force]` (bind + sync)
│   │   │   ├── rename.rs               # `tome workspace rename <old> <new>` — rename with project updates
│   │   │   ├── remove.rs               # `tome workspace remove <name> [--force]` — cascade delete
│   │   │   ├── regen_summary.rs        # `tome workspace regen-summary <name>` — regenerate summaries
│   │   │   └── sync.rs                 # `tome workspace sync [<name>]` — sync RULES.md to projects
│   │   ├── harness/                    # Phase 4: Harness sync command wrapper
│   │   │   └── mod.rs                  # Thin seam to harness::sync orchestrator
│   │   ├── doctor.rs                   # `tome doctor [--fix] [--verify]`
│   │   └── mcp.rs                      # `tome mcp` entry point
│   │
│   ├── presentation/                   # Output formatting + TUI
│   │   ├── mod.rs                      # Public API exports
│   │   ├── tables.rs                   # comfy-table wrappers
│   │   ├── progress.rs                 # indicatif spinner helpers
│   │   ├── colour.rs                   # owo-colors + NO_COLOR detection
│   │   ├── prompt.rs                   # inquire select/confirm/multiselect (TTY-only)
│   │   └── format.rs                   # Numeric formatting (MiB, etc.)
│   │
│   ├── util/                           # Phase 4: Shared utilities
│   │   ├── mod.rs                      # Public API exports
│   │   └── atomic_dir.rs               # Atomic directory landing (tempfile + rename)
│   │
│   └── mcp/                            # MCP server (async island, Phase 3)
│       ├── mod.rs                      # Sync entry point: run()
│       ├── runtime.rs                  # Single-threaded tokio builder
│       ├── log.rs                      # 10 MiB rotate JSON file logger + ContractEventFormat
│       ├── preflight.rs                # FR-110 startup checks (schema, drift, embedder hash)
│       ├── server.rs                   # rmcp server loop + graceful shutdown
│       ├── state.rs                    # McpState definition (embedder, reranker OnceLock)
│       └── tools/                      # MCP tool handlers
│           ├── mod.rs                  # Tool registration
│           ├── search_skills.rs        # search_skills tool (KNN+rerank, workspace-filtered)
│           └── get_skill.rs            # get_skill tool (metadata + components)
│
├── tests/                              # Integration tests (access library as external crate)
│   ├── catalog_*.rs                    # Catalog add/remove/update tests
│   ├── plugin_*.rs                     # Plugin enable/disable/list/show/interactive
│   ├── models_*.rs                     # Model download/list/remove
│   ├── query.rs                        # Query + strict mode + rerank
│   ├── reindex.rs                      # Reindex all/per-catalog/per-plugin
│   ├── status.rs                       # Status command + health checks
│   ├── workspace_*.rs                  # Workspace info/init/binding/sync/list/rename/remove tests (US1–US2)
│   ├── doctor.rs                       # Doctor assembly + fixes + harness detect
│   ├── mcp_*.rs                        # MCP server lifecycle + tools
│   ├── exit_codes.rs                   # Exit code matrix validation
│   ├── manifest_strictness.rs          # Strict/lenient parsing guards
│   ├── atomicity.rs                    # Interrupt-injection tests (SIGINT mid-op)
│   ├── concurrency.rs                  # Two-process index contention
│   ├── schema_migration_e2e.rs         # Forward migration via MIGRATIONS_OVERRIDE
│   ├── sync_boundary.rs                # Structural test: no async outside src/mcp/
│   ├── common/
│   │   ├── mod.rs                      # Test utilities (StubEmbedder, StubHarness, fixtures, Fixture builder)
│   │   └── stub_*.rs                   # Stub implementations for test injection
│   └── fixtures/
│       └── sample-plugin-catalog/      # Real plugin tree for integration tests
│
├── vendor/                             # Vendored C dependencies
│   └── sqlite-vec/                     # sqlite-vec extension (built via build.rs)
│
├── .githooks/                          # Git hooks (versioned, no external manager)
│   ├── pre-commit                      # fmt, clippy, typos
│   └── pre-push                        # cargo test
│
├── .sdd/                               # SDD codebase documentation
│   └── codebase/
│       ├── STACK.md                    # Technologies + versions
│       ├── INTEGRATIONS.md             # External APIs + services
│       ├── ARCHITECTURE.md             # System design + patterns
│       ├── STRUCTURE.md                # Directory layout (this file)
│       ├── CONVENTIONS.md              # Naming + code style
│       ├── TESTING.md                  # Test strategy + patterns
│       ├── SECURITY.md                 # Auth + authorization
│       └── CONCERNS.md                 # Tech debt + risks
│
├── specs/                              # Design docs + contracts
│   ├── 001-phase-1-foundations/
│   │   └── spec.md
│   ├── 002-phase-2-plugins-index/
│   │   ├── spec.md
│   │   ├── plan.md
│   │   ├── research.md
│   │   ├── data-model.md
│   │   ├── contracts/
│   │   └── quickstart.md
│   ├── 003-phase-3-mcp-workspaces/
│   │   ├── spec.md
│   │   ├── plan.md
│   │   ├── research.md
│   │   ├── data-model.md
│   │   ├── contracts/
│   │   └── quickstart.md
│   └── 004-phase-4-refactor-harnesses/       # Phase 4 (F1–F11 shipped; US1–US2 shipped)
│       ├── spec.md
│       ├── plan.md
│       ├── research.md (19 R-decisions)
│       ├── data-model.md (schema v2, Scope reshape, HarnessModule, settings layers)
│       ├── contracts/ (13 contracts: paths-and-layout, harness-modules, settings-composition, sync-algorithm, workspace-commands, etc.)
│       ├── retro/ (P2.md: F1–F11; later US1–US2)
│       └── quickstart.md
│
├── PRDs/                               # Product requirement documents
│   ├── phase-1.md
│   ├── phase-2.md
│   ├── phase-3.md
│   └── phase-4.md
│
├── retro/                              # Phase retrospectives
│   ├── P2.md (phase 2 polish)
│   ├── P3.md (phase 2 feature complete)
│   ├── P4.md (workspace lifecycle)
│   ├── P5.md (refcount)
│   ├── P6.md (doctor)
│   ├── P7.md (schema migration)
│   ├── P8.md (phase 3 polish)
│   └── P10.md (phase 2 polish / feature complete)
│
├── Cargo.toml                          # Package definition (MSRV 1.93, v0.4.0+)
├── Cargo.lock                          # Dependency lock
├── build.rs                            # sqlite-vec C extension compilation
├── CONSTITUTION.md                     # v1.3.0 — constraints + trade-offs (Phase 4 §Paths amendment)
├── CLAUDE.md                           # Project context for Claude Code
└── CHANGELOG.md                        # Version history (v0.1.0–v0.3.0, Phase 4 in flight)
```

## Key Directories

### `src/` — Source Code

| Directory | Purpose | Key Files | When to Add Code |
|-----------|---------|-----------|------------------|
| `main.rs` | CLI entry, scope resolution, command dispatch | — | CLI bootstrap only |
| `cli.rs` | Command-line argument parsing (clap derive) | — | New subcommands or global flags |
| `error.rs` | Closed TomeError enum + exit code mapping | — | New failure classes only (rare) |
| `catalog/` | Catalog registry, git ops, reference counting | `git.rs`, `store.rs` | New catalog features |
| `plugin/` | Plugin metadata, lifecycle orchestration | `lifecycle.rs` | Plugin enable/disable/reindex logic |
| `index/` | SQLite index, schema, migrations, KNN query | `schema.rs`, `skills.rs` | Schema changes, new queries |
| `embedding/` | Text embedding, reranking, model management | `registry.rs` | Model updates, embedding features |
| `workspace/` | Scope resolution, binding, lifecycle management | `binding.rs`, `resolution.rs`, `init.rs`, `rename.rs`, `remove.rs`, `sync.rs` | Multi-workspace features, binding logic, workspace operations |
| `harness/` | Phase 4: Per-harness trait impls + sync orchestrator | `sync.rs`, per-harness files, `rules_file.rs`, `mcp_config.rs` | New harness integrations, sync logic, rules/MCP strategies |
| `settings/` | Phase 4: Layered composition resolver | `resolver.rs` | Composition logic changes |
| `commands/` | CLI command implementations | `catalog.rs`, `plugin/`, `workspace/`, `harness/` | New commands or command logic |
| `presentation/` | Output formatting, TUI, colors | `tables.rs`, `prompt.rs` | Output enhancements |
| `mcp/` | MCP server (async island) | `tools/`, `runtime.rs` | MCP tool handlers, server features |
| `util/` | Shared utilities (atomic directories, etc.) | `atomic_dir.rs` | Common helper functions |

### `tests/` — Test Files

| Directory | Purpose | Pattern | When to Add |
|-----------|---------|---------|------------|
| `catalog_*.rs` | Catalog lifecycle tests | `#[test] fn catalog_add_updates_refcount()` | Catalog feature changes |
| `plugin_*.rs` | Plugin enable/disable tests | `#[test] fn plugin_enable_embeds_skills()` | Plugin feature changes |
| `workspace_*.rs` | Workspace binding + info + sync + lifecycle tests | `#[test] fn workspace_binding_lands_marker()` | Workspace feature changes (US1–US2) |
| `query.rs` | KNN + rerank tests | `#[test] fn query_with_rerank_sorts_by_score()` | Query logic changes |
| `common/mod.rs` | Test utilities + fixtures | `fn build_test_db()`, `StubEmbedder` | Shared test helpers |
| `sync_boundary.rs` | Structural: no async outside `src/mcp/` | Build-time path scan | Architecture enforcement |

## Module Boundaries

### Feature Modules

Each capability module is self-contained:

- **`catalog/`** — Manages registry persistence + git cloning
  - Can call: `config`, `error`, `paths`, `serde`, `util::atomic_dir` (new: write_atomic)
  - Cannot call: `plugin`, `index`, `commands`
- **`plugin/`** — Orchestrates enable/disable
  - Can call: `catalog`, `embedding`, `index`, `config`, `error`
  - Cannot call: `commands`, `output` (returns Result)
- **`index/`** — Persists skills + embeddings
  - Can call: `embedding::registry` (for model identity), `error`, `paths`, `rusqlite`
  - Cannot call: `plugin`, `commands` (except integration tests)
- **`embedding/`** — Wraps ML models
  - Can call: `error`, `paths`, `serde`, `ort`, `fastembed-rs`
  - Cannot call: `index` (except trait bounds for output), `plugin`
- **`workspace/`** — Scope resolution + binding + lifecycle (Phase 4 expanded)
  - Can call: `catalog`, `config`, `paths`, `index` (read-only via public API), `settings`, `plugin`, `util::atomic_dir`
  - Cannot call: `commands` (except scope passes through)
- **`harness/`** — Per-harness trait + sync orchestrator (Phase 4)
  - Can call: `paths` (filesystem checks only, existence probes), `settings` (resolver callback), `error`, `util::atomic_dir` (mcp_config writes)
  - Cannot call: Any business logic except `settings::resolver::StubScope` for composition
- **`settings/`** — Composition resolver (Phase 4)
  - Can call: `serde`, `error`, `workspace::name`, `paths` (readonly)
  - Cannot call: `index`, `harness` (directly; harness registry accessed via callback)
- **`summarise/`** — Workspace summariser (Phase 4)
  - Can call: `error`, `paths`, `serde`, `llama-cpp-2` (production), `plugin`
  - Cannot call: `index`, `commands`
- **`doctor/`** — Health check + auto-repair
  - Can call: `embedding`, `index`, `catalog`, `commands::plugin`, `paths`, `harness` (detect)
  - Cannot call: None; very broad reading access

### Command Modules

Commands are thin wrappers:

```
commands/{catalog,plugin,models,query,reindex,status,workspace,harness,mcp,doctor}.rs
  ↓
Resolve dependencies (config, index lock, embedder)
  ↓
Call library function (plugin::lifecycle, embedding, doctor::assemble_report, workspace::*, harness::sync, etc.)
  ↓
Emit output (presentation + output.rs)
```

Never put business logic inside `commands/` — extract to `plugin/`, `embedding/`, `index/`, `workspace/`, `harness/`, etc.

## Where to Add New Code

| If you're adding... | Put it in... | Example |
|---------------------|--------------|---------|
| New catalog feature | `src/catalog/` | `pub fn list_catalogs_by_workspace()` |
| Plugin enable/disable logic | `src/plugin/lifecycle.rs` | `pub fn auto_disable_orphan_skills()` |
| New search filter | `src/index/query.rs` | `pub fn knn_with_plugin_filter()` |
| Model download feature | `src/embedding/download.rs` | `pub fn download_model_with_retry()` |
| Workspace feature | `src/workspace/{init,rename,remove,sync,regen_summary}.rs` | New binding validators, workspace operations |
| Project binding logic | `src/workspace/binding.rs` | New binding validators |
| Harness integration | `src/harness/{harness_name}.rs` | New `HarnessModule` impl |
| Harness sync logic | `src/harness/sync.rs` | Orchestration changes |
| Rules file strategy | `src/harness/rules_file.rs` | New strategy type + handlers |
| MCP config logic | `src/harness/mcp_config.rs` | New format support |
| Settings composition | `src/settings/resolver.rs` | New composition reference types |
| Summariser feature | `src/summarise/` | New prompt, backend changes |
| MCP tool | `src/mcp/tools/` | New file with tool handler |
| CLI command | `src/commands/{feature}.rs` or `src/commands/{feature}/mod.rs` | New `pub fn run(args, scope, mode)` |
| Output format | `src/presentation/` | New `comfy-table` wrapper |
| Test fixture | `tests/common/mod.rs` | `fn build_workspace_db()` |
| New dependency feature | `build.rs` | C extension compilation |

## Import Paths

There are no custom path aliases (e.g., `@/`). Use absolute paths from crate root:

```rust
use tome::plugin::lifecycle::enable;
use tome::index::{open, open_read_only};
use tome::embedding::{Embedder, Reranker};
use tome::harness::HarnessModule;
use tome::workspace::binding::bind_project;
use tome::workspace::{init, rename, remove, sync_one};
use tome::settings::resolver::resolve_effective_list;
use tome::util::atomic_dir::land_directory;
```

## Entry Points

| File | Purpose | Called by | Calls |
|------|---------|-----------|-------|
| `src/main.rs` | CLI bootstrap | Binary | `workspace::resolution`, `commands::*` |
| `src/mcp/mod.rs::run()` | MCP server bootstrap | Binary via `commands::mcp` | `tokio`, `rmcp::serve_server` |
| `src/lib.rs` | Library re-exports | Integration tests | All public modules |
| `src/workspace/binding.rs::bind_project()` | Project binding | `commands::workspace::use_` | `index`, `util::atomic_dir` |
| `src/harness/sync.rs::sync_project()` | Harness orchestration | `commands::harness::sync_for_project_root()` | `settings`, per-`HarnessModule` |
| `src/workspace/init.rs::init()` | Workspace creation | `commands::workspace::init` | `util::atomic_dir`, `catalog`, `index` |
| `src/workspace/rename.rs::rename()` | Workspace rename | `commands::workspace::rename` | `index`, per-project marker writes |
| `src/workspace/remove.rs::remove()` | Workspace removal | `commands::workspace::remove` | `index`, `harness`, `catalog::store` |
| `src/workspace/regen_summary.rs::regen()` | Summary regeneration | `commands::workspace::regen_summary` | `summarise`, `util::atomic_dir` |
| `src/workspace/sync.rs::sync_one()` | RULES.md sync | `commands::workspace::sync` | `util::atomic_dir` |
| `build.rs` | Build-time setup | Cargo | sqlite-vec C compiler |

## Generated Files

None — all code is hand-written. `index.db`, `index.lock`, catalog cache, project markers, and workspace directories are generated at runtime but not tracked in git.

## Phase 4 Structural Changes (F1–F11 + US1–US2 Shipped)

### Foundational (F1–F11)
- **Paths**: Consolidated under `<home>/.tome/` (no XDG split) — `src/paths.rs`
- **Central database**: Single `index.db` per Paths (was: per-workspace) — `src/index/schema.rs` schema v2
- **Scope type**: `Scope(WorkspaceName)` tuple struct (was: `Scope::Global | Workspace(Path)`) — `src/workspace/scope.rs`
- **Error variants**: 8 new variants (13, 19, 70, 71, 72, 73, 74, 75) for workspace + binding + clash + schema + exit-codes — `src/error.rs`
- **WorkspaceName newtype**: Validation + parsing, RFC-952 DNS-label rules — `src/workspace/name.rs`
- **Workspace junction tables**: `workspace_catalogs`, `workspace_skills`, `workspace_projects` — `src/index/schema.rs`
- **Per-command scope threading**: Every command accepts resolved `Scope` parameter — `src/commands/`
- **Harness abstraction**: Five `HarnessModule` impls + registry — `src/harness/`
- **Settings layers**: Project / workspace / global with composition refs — `src/settings/`
- **Summariser skeleton**: `src/summarise/` with stub + llama impls

### User Stories (US1–US2 Shipped)
- **US1 (Phase 4 / F1–F11)**: `tome workspace use` — bind project to workspace + harness sync
  - **US1.a** (binding): `src/workspace/binding.rs`, `src/commands/workspace/use_.rs`
  - **US1.b-c** (harness sync): `src/harness/sync.rs`, `src/commands/harness/mod.rs`
- **US2 (Phase 4 / PRs #82–#86)**: Full workspace lifecycle management
  - **US2.a** (info/list): `src/commands/workspace/info.rs`, `src/commands/workspace/list.rs`
  - **US2.b** (init): `src/workspace/init.rs`, `src/commands/workspace/init.rs`
  - **US2.c** (rename/remove): `src/workspace/rename.rs`, `src/workspace/remove.rs`, `src/commands/workspace/rename.rs`, `src/commands/workspace/remove.rs`
  - **US2.d** (regen-summary/sync): `src/workspace/regen_summary.rs`, `src/workspace/sync.rs`, `src/commands/workspace/regen_summary.rs`, `src/commands/workspace/sync.rs`
  - **US2.d-1** (atomic writes): `catalog::store::write_atomic` lifted to public, used by workspace/harness writers

---

*This document shows WHERE code lives. Update when directory structure changes.*

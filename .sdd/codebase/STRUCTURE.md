# Project Structure

> **Purpose**: Document directory layout, module boundaries, and where to add new code.
> **Generated**: 2026-05-14
> **Last Updated**: 2026-05-14

## Directory Layout

```
tome/
├── src/                                # Rust library + binary source
│   ├── main.rs                         # CLI entry: scope resolution, command dispatch, error mapping
│   ├── lib.rs                          # Public exports
│   ├── cli.rs                          # clap derive defs (all commands + global flags)
│   ├── error.rs                        # Closed TomeError enum (26+ variants → exit codes)
│   ├── config.rs                       # config.toml parsing (strict)
│   ├── paths.rs                        # XDG paths + per-scope accessors
│   ├── logging.rs                      # tracing-subscriber wiring
│   ├── output.rs                       # JSON / human output mode dispatcher
│   │
│   ├── catalog/                        # Catalog registry + git ops
│   │   ├── mod.rs                      # Public API
│   │   ├── manifest.rs                 # tome-catalog.toml parsing (strict)
│   │   ├── store.rs                    # Registry persistence + refcount
│   │   └── git.rs                      # Shell git ops + credential scrubbing
│   │
│   ├── config/                         # (Optional future: config schema enhancements)
│   │
│   ├── plugin/                         # Plugin metadata + lifecycle
│   │   ├── mod.rs                      # PluginRecord, PluginStatus
│   │   ├── manifest.rs                 # plugin.json parsing (lenient)
│   │   ├── frontmatter.rs              # SKILL.md YAML frontmatter parser
│   │   ├── identity.rs                 # PluginId: <catalog>/<plugin> parsing
│   │   ├── components.rs               # Walk skill/agent/command/hook dirs
│   │   └── lifecycle.rs                # enable/disable/reindex orchestration
│   │
│   ├── index/                          # Vector search index (SQLite + sqlite-vec)
│   │   ├── mod.rs                      # Public API exports
│   │   ├── db.rs                       # Open, WAL config, schema version check
│   │   ├── schema.rs                   # CREATE TABLE statements + bootstrap
│   │   ├── migrations.rs               # Forward-only schema migrations + framework
│   │   ├── vec_ext.rs                  # sqlite-vec extension loader
│   │   ├── skills.rs                   # Skills table CRUD + content-hash diffing
│   │   ├── query.rs                    # KNN search + optional reranking
│   │   ├── meta.rs                     # Model identity metadata + drift detection
│   │   ├── integrity.rs                # PRAGMA integrity_check wrapper
│   │   └── lock.rs                     # Advisory lockfile acquisition
│   │
│   ├── embedding/                      # Model management + inference
│   │   ├── mod.rs                      # Embedder/Reranker traits
│   │   ├── fastembed.rs                # FastembedEmbedder impl via fastembed-rs
│   │   ├── stub.rs                     # StubEmbedder (cfg test)
│   │   ├── registry.rs                 # Pinned MODEL_REGISTRY (URLs + SHA-256)
│   │   ├── download.rs                 # Model fetch + verify + atomic persist
│   │   └── runtime.rs                  # ort Environment singleton setup
│   │
│   ├── workspace/                      # Scope + context resolution (Phase 3)
│   │   ├── mod.rs                      # Public API exports
│   │   ├── scope.rs                    # Scope enum + ResolvedScope
│   │   ├── resolution.rs               # Workspace vs global determination
│   │   ├── info.rs                     # WorkspaceInfo report assembly
│   │   ├── init.rs                     # Atomic .tome/ directory creation
│   │   └── inventory.rs                # Opt-in workspaces.txt registry
│   │
│   ├── doctor/                         # Diagnostic + auto-repair (Phase 3 US4)
│   │   ├── mod.rs                      # assemble_report + re_assemble entry
│   │   ├── report.rs                   # DoctorReport, CatalogCacheState enum
│   │   ├── checks.rs                   # check_catalogs, check_workspace_registry
│   │   ├── harness_detect.rs           # Probe ~/.claude/, ~/.codex/, etc.
│   │   └── fixes.rs                    # apply + auto-fix dispatch
│   │
│   ├── commands/                       # CLI command entry points
│   │   ├── mod.rs                      # Public API exports
│   │   ├── catalog.rs                  # `tome catalog {add,remove,list,update,show}`
│   │   ├── plugin/                     # `tome plugin` subcommands
│   │   │   ├── mod.rs                  # Dispatcher + shared helpers
│   │   │   ├── enable.rs               # `tome plugin enable <id>`
│   │   │   ├── disable.rs              # `tome plugin disable <id>`
│   │   │   ├── list.rs                 # `tome plugin list`
│   │   │   ├── show.rs                 # `tome plugin show <id>`
│   │   │   └── interactive.rs          # Bare `tome plugin` → three-level TUI
│   │   ├── models/                     # `tome models` subcommands
│   │   │   ├── mod.rs                  # Dispatcher + shared helpers
│   │   │   ├── download.rs             # `tome models download [<name>]`
│   │   │   ├── list.rs                 # `tome models list [--verify]`
│   │   │   └── remove.rs               # `tome models remove <name>`
│   │   ├── query.rs                    # `tome query [<text>]` + --catalog, --strict
│   │   ├── reindex.rs                  # `tome reindex [<scope>] [--force]`
│   │   ├── status.rs                   # `tome status [--verify]` + --version hook
│   │   ├── workspace/                  # `tome workspace` subcommands
│   │   │   ├── mod.rs                  # Dispatcher
│   │   │   ├── info.rs                 # `tome workspace info`
│   │   │   └── init.rs                 # `tome workspace init [--inherit-global]`
│   │   ├── doctor.rs                   # `tome doctor [--fix] [--verify]`
│   │   └── mcp.rs                      # `tome mcp` (Phase 3 US1)
│   │
│   ├── presentation/                   # Output formatting + TUI
│   │   ├── mod.rs                      # Public API exports
│   │   ├── tables.rs                   # comfy-table wrappers
│   │   ├── progress.rs                 # indicatif spinner helpers
│   │   ├── colour.rs                   # owo-colors + NO_COLOR detection
│   │   ├── prompt.rs                   # inquire select/confirm/multiselect
│   │   └── format.rs                   # Numeric formatting (MiB, etc.)
│   │
│   └── mcp/                            # MCP server (async island, Phase 3)
│       ├── mod.rs                      # Sync entry point: run()
│       ├── runtime.rs                  # Single-threaded tokio builder
│       ├── log.rs                      # 10 MiB rotate JSON file logger
│       ├── preflight.rs                # FR-110 startup checks
│       ├── server.rs                   # rmcp server loop + graceful shutdown
│       ├── state.rs                    # McpState definition
│       └── tools/                      # MCP tool handlers
│           ├── mod.rs                  # Tool registration
│           ├── search_skills.rs        # search_skills tool (KNN+rerank)
│           └── get_skill.rs            # get_skill tool (metadata + components)
│
├── tests/                              # Integration tests
│   ├── catalog_*.rs                    # Catalog add/remove/update tests
│   ├── plugin_*.rs                     # Plugin enable/disable/list/show/interactive
│   ├── models_*.rs                     # Model download/list/remove
│   ├── query.rs                        # Query + strict mode + rerank
│   ├── reindex.rs                      # Reindex all/per-catalog/per-plugin
│   ├── status.rs                       # Status command + health checks
│   ├── workspace_*.rs                  # Workspace info/init/commands integration
│   ├── doctor.rs                       # Doctor assembly + fixes + harness detect
│   ├── mcp_*.rs                        # MCP server lifecycle + tools
│   ├── exit_codes.rs                   # Exit code matrix validation
│   ├── manifest_strictness.rs          # Strict/lenient parsing guards
│   ├── atomicity.rs                    # Interrupt-injection tests (SIGINT mid-op)
│   ├── concurrency.rs                  # Two-process index contention
│   ├── schema_migration_e2e.rs         # Forward migration via MIGRATIONS_OVERRIDE
│   ├── sync_boundary.rs                # Structural test: no async outside src/mcp/
│   ├── common/mod.rs                   # Test utilities (StubEmbedder, fixtures)
│   └── fixtures/
│       └── sample-plugin-catalog/      # Real plugin tree for integration tests
│
├── vendor/                             # Vendored C dependencies
│   └── sqlite-vec/                     # sqlite-vec extension (built via build.rs)
│
├── .githooks/                          # Git hooks (versioned)
│   ├── pre-commit                      # fmt, clippy, typos
│   └── pre-push                        # cargo test
│
├── specs/                              # Design docs + contracts
│   ├── 001-phase-1-foundations/
│   │   └── spec.md
│   ├── 002-phase-2-plugins-index/
│   │   ├── spec.md
│   │   ├── plan.md
│   │   ├── research.md
│   │   ├── data-model.md
│   │   ├── contracts/                  # Protocol specs
│   │   │   ├── plugin-commands.md
│   │   │   ├── query.md
│   │   │   ├── models-commands.md
│   │   │   ├── reindex.md
│   │   │   ├── status.md
│   │   │   ├── catalog-extensions.md
│   │   │   ├── version-output.md
│   │   │   ├── exit-codes.md
│   │   │   └── index-schema.sql
│   │   └── quickstart.md
│   └── 003-phase-3-mcp-workspaces/
│       ├── spec.md
│       ├── plan.md
│       ├── research.md
│       ├── data-model.md
│       ├── contracts/
│       │   ├── mcp-server.md
│       │   ├── mcp-tools.md
│       │   ├── workspace-resolution.md
│       │   ├── workspace-init.md
│       │   ├── workspace-info.md
│       │   ├── doctor.md
│       │   ├── schema-migration.md
│       │   ├── catalog-extensions-p3.md
│       │   ├── exit-codes-p3.md
│       │   └── log-format.md
│       └── quickstart.md
│
├── .sdd/
│   └── codebase/
│       ├── STACK.md                    # Technologies + versions
│       ├── INTEGRATIONS.md             # External APIs + services
│       ├── ARCHITECTURE.md             # System design + patterns (this file's parent)
│       ├── STRUCTURE.md                # Directory layout (this file)
│       ├── CONVENTIONS.md              # Naming + code style
│       ├── TESTING.md                  # Test strategy + patterns
│       ├── SECURITY.md                 # Auth + authorization
│       └── CONCERNS.md                 # Tech debt + risks
│
├── Cargo.toml                          # Package definition (MSRV 1.93)
├── Cargo.lock                          # Dependency lock
├── CONSTITUTION.md                     # v1.2.0 — constraints + trade-offs
├── CLAUDE.md                           # Project context for Claude Code
├── CHANGELOG.md                        # Version history (v0.1.0, v0.2.0, v0.3.0)
├── PRDs/                               # Product requirement documents
│   ├── phase-1.md
│   ├── phase-2.md
│   └── phase-3.md
├── review/                             # Post-review findings + triage
│   ├── findings.md
│   └── disposition.md
├── retro/                              # Phase retro summaries
│   ├── P1.md
│   ├── P2.md
│   ├── ... (P3 → P8)
│   └── P8.md
└── README.md                           # Project overview
```

## Key Directories

### `src/` — Source Code

| Directory | Purpose | Entry Point | Key Pattern |
|-----------|---------|-------------|-------------|
| `main.rs` | CLI entry | Scope resolution first, then dispatch | Catches all errors, maps to exit codes |
| `catalog/` | Registry management | `store::save_atomic` | Atomic TOML writes, git shell-outs |
| `plugin/` | Metadata + lifecycle | `lifecycle::enable` | Parse manifests, orchestrate embedder calls |
| `index/` | Vector search DB | `db::open` | Advisory lock + WAL, migrations on open |
| `embedding/` | Model inference | `fastembed::new` or `stub::new` | Trait-based, lazy-load reranker |
| `workspace/` | Scope resolution | `resolution::resolve` | Happens at CLI entry, threads through all commands |
| `doctor/` | Diagnostics | `assemble_report` | Silent compute, pure functions for testability |
| `mcp/` | MCP server | `run(scope, paths)` | Single-threaded tokio, all async confined here |
| `commands/` | CLI dispatch | Per-subcommand `run(args, scope, mode)` | Silent compute + emit wrapper pattern |

### `tests/` — Test Files

| File/Directory | Test Type | Pattern | Coverage |
|---|---|---|---|
| `catalog_*.rs` | Integration | CLI binary + library API | Add/remove/list/update/show + refcount |
| `plugin_*.rs` | Integration | CLI binary ± `StubEmbedder` | Enable/disable/list/show/interactive/repeated |
| `models_*.rs` | Integration | CLI binary ± sparse file fixtures | Download/list/remove + hash verify |
| `query.rs` | Integration | Library API + `StubEmbedder` | KNN + rerank + --strict + no-rerank |
| `reindex.rs` | Integration | CLI binary + library API | Per-plugin atomicity, per-catalog batching |
| `status.rs` | Integration | Library API (no binary) | Health checks, model/index/drift state |
| `workspace_*.rs` | Integration | CLI binary + library API | Info/init + resolution + cross-command scope |
| `doctor.rs` | Integration | Library API (no binary) | Report assembly + fixes + harness detect |
| `mcp_*.rs` | Integration | Library API via `StubEmbedder` | Server lifecycle, tool handlers, log format |
| `exit_codes.rs` | Unit | Direct enum dispatch | Matrix coverage for all 26+ codes |
| `manifest_strictness.rs` | Unit | Direct parse calls | Strict Tome-owned vs lenient third-party |
| `atomicity.rs` | Integration | Interrupt-injection (closure-level `Err`) | SIGINT mid-transaction rollback |
| `concurrency.rs` | Integration | Two-process subprocess | Advisory lock contention |
| `schema_migration_e2e.rs` | Integration | Synthetic `MIGRATIONS_OVERRIDE` | Forward migrate, mid-sequence failures |
| `sync_boundary.rs` | Structural | Grep-based path check | Fail build if `tokio::` outside `src/mcp/` |
| `common/mod.rs` | Fixture utility | Reusable builders | `StubEmbedder`, `ToolEnv`, test DB bootstrap |
| `fixtures/sample-plugin-catalog/` | Real fixture | Git repo structure | Real manifest + SKILL.md files |

## Module Boundaries

### Adding a New Catalog Subcommand

1. Add CLI variant to `src/cli.rs::CatalogCommand` enum
2. Add handler to `src/commands/catalog.rs::run()`
3. Library logic lives in `src/catalog/` (separate concern)
4. Add integration test file: `tests/catalog_<subcommand>.rs`
5. Update exit code matrix in `tests/exit_codes.rs` if adding new `TomeError` variant

### Adding a New Plugin Lifecycle Phase

1. New phase (enable/disable/reindex) adds a variant to `src/cli.rs::PluginCommand`
2. Handler goes in `src/commands/plugin/<phase>.rs`
3. Library orchestration in `src/plugin/lifecycle.rs`
4. Index mutations via `src/index/skills.rs`
5. Add integration test: `tests/plugin_<phase>.rs`

### Adding a New Index Subsystem Check

1. Add check function to `src/doctor/checks.rs` or `src/commands/status.rs`
2. `doctor::assemble_report` calls it
3. Add corresponding `SuggestedFix` dispatch in `src/doctor/fixes.rs` if auto-fixable
4. Add unit test in test file covering the check + fix

### Adding a New MCP Tool

1. Add schema + handler to `src/mcp/tools/<tool_name>.rs`
2. Register in `src/mcp/server.rs` via `#[tool_handler]` macro
3. Reuse library compute (e.g., `query::pipeline`) inside handler via `spawn_blocking`
4. Add test in `tests/mcp_server.rs` covering input validation + output envelope

## Where to Add New Code

| If you're adding... | Put it in... | Example Path |
|---------------------|--------------|---|
| New CLI command | `src/commands/<domain>/` | `src/commands/plugin/enable.rs` |
| New catalog operation | `src/catalog/` | `src/catalog/store.rs` |
| New plugin metadata parser | `src/plugin/` | `src/plugin/components.rs` |
| New index query type | `src/index/query.rs` | `src/index/query.rs::knn()` variant |
| New model download feature | `src/embedding/download.rs` | `embedding::download::verify_checksum()` |
| New scope resolution rule | `src/workspace/resolution.rs` | `resolution::resolve()` match arm |
| New diagnostic check | `src/doctor/checks.rs` | `doctor::checks::check_new_subsystem()` |
| New presentation formatter | `src/presentation/` | `src/presentation/tables.rs` or `format.rs` |
| New MCP tool | `src/mcp/tools/<name>.rs` | `src/mcp/tools/search_skills.rs` |
| Shared library helper | `tests/common/mod.rs` | `common::StubEmbedder` trait impl |

## Naming Conventions

### Rust Files

| Pattern | Usage | Example |
|---------|-------|---------|
| `mod.rs` | Module root + public API | `src/plugin/mod.rs` re-exports `PluginRecord` |
| `{feature}.rs` | Single-feature module | `src/index/migrations.rs` |
| `{domain}/{subfeature}.rs` | Multi-level domain | `src/mcp/tools/search_skills.rs` |

### Functions

| Pattern | Usage | Example |
|---------|-------|---------|
| `pub fn run(args, scope, mode) -> Result<>` | CLI dispatcher | `commands::plugin::enable::run()` |
| `pub fn run_with_deps(..., mode) -> Result<>` | Library + test entry | `commands::reindex::run_with_deps()` |
| `pub fn pipeline(args, deps) -> Result<>` | Silent compute (no emit) | `commands::query::pipeline()` |
| `pub fn assemble_*(scope, paths) -> Result<>` | Report builders | `doctor::assemble_report()` |
| `pub fn check_*(paths, scope) -> Result<>` | Diagnostic checks | `doctor::checks::check_catalogs()` |
| `pub fn apply_*(report, paths, scope)` | Repair executors | `doctor::fixes::apply()` |

### Test Files

| Pattern | Usage | Example |
|---------|-------|---------|
| `tests/{domain}_{feature}.rs` | Feature + domain integration | `tests/plugin_enable.rs` |
| `tests/{feature}_e2e.rs` | End-to-end multi-command | `tests/schema_migration_e2e.rs` |
| `tests/{feature}_json.rs` | JSON output pinning | `tests/workspace_info_json.rs` |

## Generated Files

Files auto-generated and should NOT be manually edited:

| Location | Generator | Regenerate | Notes |
|----------|-----------|-----------|-------|
| `target/` | `cargo build` | Rebuild | Ignored in `.gitignore` |
| `Cargo.lock` | Cargo | `cargo update` (rarely) | Committed for reproducibility |
| `.sdd/codebase/*.md` | `/sdd:map` skill | Manual refresh | Not auto-generated; documented in CLAUDE.md |

## Entry Points

| File | Purpose | Exit Path |
|------|---------|-----------|
| `src/main.rs` | Binary entry; scope resolve → command dispatch | `std::process::exit(code)` |
| `src/lib.rs` | Library exports (used by `tests/`, MCP) | Result types returned |
| `src/mcp/mod.rs::run()` | Sync MCP entry from `main.rs` | `tokio::runtime::block_on` → Result |
| `src/commands/*/run()` | Per-command CLI entry | `output::write*()` + implicit exit 0, or error |
| `tests/common/` | Test fixture builders | Helper returns `Result` |

## Build Configuration

- **Cargo.toml**: `rust-version = "1.93"` (MSRV pinned)
- **Profile**: `lto = "thin"`, `panic = "abort"`, `strip = "symbols"` (binary size)
- **Dependencies**: 
  - Sync: `clap`, `serde`, `rusqlite`, `fastembed-rs`, `time`, etc.
  - Async (MCP only): `tokio`, `rmcp`, `schemars`
  - Vendored: `sqlite-vec` (C extension, compiled via `build.rs`)

## Testing Strategy

| Level | Tool | Pattern | Location |
|-------|------|---------|----------|
| **Unit** | `#[test]` in modules | Direct function calls | Within source files or `tests/` |
| **Integration** | `cargo test --test` | CLI binary or library API | `tests/{feature}_*.rs` |
| **E2E** | Multiple commands | Real file I/O | `tests/*_e2e.rs` |
| **Structural** | Grep + path checks | Enforce invariants | `tests/sync_boundary.rs`, `manifest_strictness.rs` |

## Import Paths

No special alias resolution (no `tsconfig.json` equivalent). Rust uses the module tree directly:

```rust
use tome::plugin::PluginId;              // From library
use tome::commands::status::assemble;    // From library
use common::StubEmbedder;                // From tests/common/
```

---

## What Does NOT Belong Here

- Architecture patterns → ARCHITECTURE.md
- Technology choices → STACK.md
- Code style rules → CONVENTIONS.md
- Test patterns → TESTING.md

---

*This document shows WHERE code lives. Update when directory structure changes.*

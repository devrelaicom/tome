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
│   ├── harness/                        # Phase 4: Per-harness trait + sync orchestrator + composition
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
│   │   ├── resolver.rs                 # Resolve effective harness list (priority walk + composition refs + ScopeProvider trait)
│   │   └── edit.rs                     # Phase 4 US3: Surgical TOML edits for harness use/remove
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
│   │   ├── harness/                    # Phase 4 US3: Complete harness command surface
│   │   │   ├── mod.rs                  # Dispatcher (6 subcommands) + CentralDbScopeProvider impl
│   │   │   ├── bare.rs                 # `tome harness` (no subcommand) — list all supported harnesses
│   │   │   ├── list.rs                 # `tome harness list [workspace]` — resolve effective harness list
│   │   │   ├── use_.rs                 # `tome harness use <name> [--scope {project|workspace|global}]`
│   │   │   ├── remove.rs               # `tome harness remove <name> [--scope]` — delete from settings
│   │   │   ├── info.rs                 # `tome harness info` — per-harness details + detection
│   │   │   └── sync.rs                 # `tome harness sync [--force]` — reconcile filesystem
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
│   ├── harness_*.rs                    # Phase 4 US3: Harness list/use/remove/info/sync/composition tests
│   ├── doctor.rs                       # Doctor assembly + fixes + harness detect
│   ├── mcp_*.rs                        # MCP server lifecycle + tools
│   ├── exit_codes.rs                   # Exit code matrix validation
│   ├── manifest_strictness.rs          # Strict/lenient parsing guards
│   ├── atomicity.rs                    # Interrupt-injection tests (SIGINT mid-op)
│   ├── concurrency.rs                  # Two-process index contention
│   ├── schema_migration_e2e.rs         # Forward migration via MIGRATIONS_OVERRIDE
│   ├── sync_boundary.rs                # Structural test: no async outside src/mcp/
│   ├── common/
│   │   ├── mod.rs                      # Test utilities (HOME_MUTEX, HarnessModulesGuard, fixtures)
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
│   └── 004-phase-4-refactor-harnesses/       # Phase 4 (F1–F11 shipped; US1–US3 shipped)
│       ├── spec.md
│       ├── plan.md
│       ├── research.md (19 R-decisions)
│       ├── data-model.md (schema v2, Scope reshape, HarnessModule, settings layers)
│       ├── contracts/ (13 contracts: paths-and-layout, harness-modules, settings-composition, sync-algorithm, workspace-commands, etc.)
│       ├── retro/ (P2.md: F1–F11; P3.md: US1; P4.md–P5.md: US2–US3)
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

| Directory | Purpose | Key Files |
|-----------|---------|-----------|
| `catalog/` | Catalog registry, git ops | `manifest.rs`, `store.rs`, `git.rs` |
| `plugin/` | Plugin metadata, lifecycle | `manifest.rs`, `frontmatter.rs`, `lifecycle.rs` |
| `index/` | SQLite + sqlite-vec index | `db.rs`, `schema.rs`, `skills.rs`, `query.rs` |
| `embedding/` | Text embedding + reranking | `fastembed.rs`, `stub.rs`, `download.rs` |
| `workspace/` | Scope resolution, binding, lifecycle | `scope.rs`, `binding.rs`, `init.rs`, `rename.rs`, `remove.rs` |
| `harness/` | Phase 4: Harness abstraction + sync | `mod.rs` (trait), 5 harness impls, `sync.rs`, `rules_file.rs`, `mcp_config.rs` |
| `settings/` | Phase 4: Layered composition | `parser.rs`, `resolver.rs` (composition engine), `edit.rs` |
| `summarise/` | Phase 4: Workspace summariser | `llama.rs`, `stub.rs`, `prompts.rs` |
| `doctor/` | Health check + auto-repair | `checks.rs`, `fixes.rs`, `harness_detect.rs` |
| `commands/` | CLI subcommand entry points | Per-command modules + dispatchers |
| `presentation/` | Output formatting + TUI | `tables.rs`, `prompt.rs`, `colour.rs` |
| `util/` | Shared utilities | `atomic_dir.rs` (tempfile + rename) |
| `mcp/` | Async MCP server (Phase 3) | `runtime.rs`, `server.rs`, `tools/` |

### `src/harness/` — Harness Module Details

Phase 4 / US3 adds complete harness command surface (6 subcommands) backed by production composition resolver.

| File | Purpose |
|------|---------|
| `mod.rs` | `HarnessModule` trait, `SUPPORTED_HARNESSES` registry, `MCP_CONFIG_KEY` static, test injection hook (`HARNESS_MODULES_OVERRIDE`) |
| `claude_code.rs` | Claude Code harness: block-in-file rules, JSON MCP config (per-project), description + detection |
| `codex.rs` | Codex harness: block-in-file rules, TOML MCP config (global), description + detection |
| `cursor.rs` | Cursor harness: standalone rules file, JSON MCP config (per-project), description + detection |
| `gemini.rs` | Gemini CLI harness: block-in-file rules, JSON MCP config (global), description + detection |
| `opencode.rs` | OpenCode harness: block-in-file rules (inline strategy), JSON MCP config (per-project), description + detection |
| `rules_file.rs` | Block-in-file + standalone strategies, atomic write helpers, `<!-- tome:begin/end -->` marker management, `@` include directive handling |
| `mcp_config.rs` | Read/write helpers for JSON (preserve_order) + TOML (toml_edit), strict/lenient boundaries |
| `sync.rs` | Sync orchestrator: resolve effective harness list, dispatch per-harness writes, dedup shared paths, cleanup pass, forward-progress on clash |
| `stub.rs` | `StubHarnessModule` for test injection + parallelism coordination via `HarnessModulesGuard` |

### `src/settings/` — Settings & Composition Details

Phase 4 / US3 wires composition resolver into production paths via `CentralDbScopeProvider`.

| File | Purpose |
|------|---------|
| `mod.rs` | Type definitions: `ProjectMarkerConfig`, `WorkspaceSettings`, `GlobalSettings` (all `#[serde(deny_unknown_fields)]`) |
| `parser.rs` | TOML deserialization for all three types; workspace/global files are optional |
| `composition.rs` | `CompositionRef` parsing: bare names vs `[scope]` / `[workspaces.<name>]` references |
| `resolver.rs` | `resolve_effective_list()` pure-compute engine; `ScopeProvider` trait (test: `StubScope`, production: `CentralDbScopeProvider`); cycle detection via DFS |
| `edit.rs` | Phase 4 US3: Surgical TOML edits — `open_settings()`, `add_harness()`, `remove_harness()`, `save_settings()` for project/workspace/global files |

### `src/commands/harness/` — Harness Command Surface

Phase 4 / US3 implements full subcommand dispatcher + production `ScopeProvider` impl.

| File | Purpose |
|------|---------|
| `mod.rs` | Dispatcher, `sync_for_project_root()` entry (called by workspace use), `CentralDbScopeProvider` impl (consults workspaces table + reads .toml files) |
| `bare.rs` | `tome harness` (no subcommand) — tabular list of 5 supported harnesses + detection status |
| `list.rs` | `tome harness list [workspace]` — resolve effective harness list via ScopeProvider + composition resolver |
| `use_.rs` | `tome harness use <name> [--scope]` — append harness to settings file via `settings::edit`, run sync on change |
| `remove.rs` | `tome harness remove <name> [--scope]` — delete harness from settings file, run cleanup on change |
| `info.rs` | `tome harness info [--json]` — per-harness detection, targets, source-of-scope annotation |
| `sync.rs` | `tome harness sync [--force]` — idempotent reconciliation; thin wrapper over `harness::sync::sync_project` |

### `tests/` — Integration Tests

| File Pattern | Purpose |
|-----|---------|
| `catalog_*.rs` | Catalog add/remove/update/refcount tests |
| `plugin_*.rs` | Plugin enable/disable/list/show/interactive tests |
| `models_*.rs` | Model download/list/remove tests |
| `workspace_*.rs` | Workspace info/init/binding/sync/list/rename/remove tests (US1–US2) |
| `harness_*.rs` | Phase 4 US3: Harness list/use/remove/info/sync tests; composition resolver; ScopeProvider fixture tests |
| `query.rs` | Query + strict mode + reranking tests |
| `reindex.rs` | Reindex all/per-catalog/per-plugin tests |
| `status.rs` | Status command + health checks |
| `doctor.rs` | Doctor assembly + fixes + harness detect + binding subsystem |
| `mcp_*.rs` | MCP server lifecycle + tools + log rotation |
| `exit_codes.rs` | Exit code matrix validation |
| `manifest_strictness.rs` | Strict/lenient parsing guards |
| `atomicity.rs` | Interrupt-injection tests (SIGINT mid-op) |
| `concurrency.rs` | Two-process index contention |
| `schema_migration_e2e.rs` | Forward migration via MIGRATIONS_OVERRIDE |
| `sync_boundary.rs` | Structural test: no async outside src/mcp/ |
| `common/mod.rs` | Test utilities: `HOME_MUTEX`, `HarnessModulesGuard`, fixtures, stub implementations |

## Module Boundaries

### Where to Add New Code

| If you're adding... | Put it in... | Pattern |
|---------------------|--------------|---------|
| New harness | `src/harness/{name}.rs` + register in `mod.rs` | Impl `HarnessModule` trait (5 methods) |
| New harness subcommand | `src/commands/harness/{cmd}.rs` | Pattern: `run(args, scope, paths, mode)` + `assemble_*(...)` |
| New workspace command | `src/commands/workspace/{cmd}.rs` | Pattern: `run(args, scope, paths, mode)` + `assemble_*(...)` or `run_with_deps(...)` |
| New catalog command | `src/commands/catalog.rs` | Add to existing dispatcher + orchestrator |
| New skill search filter | `src/index/query.rs` | Add to `QueryFilters` struct + SQL |
| New embedder impl | `src/embedding/{name}.rs` | Impl `Embedder` trait + register in tests |
| New workspace scope hook | `src/workspace/` + `src/settings/` | Add to scope resolution + composition |
| New settings layer | `src/settings/{layer}.rs` | Add type def to `mod.rs`, parser to `parser.rs` |
| New composition ref type | `src/settings/composition.rs` | Extend `CompositionRef` enum |
| New diagnostic check | `src/doctor/checks.rs` | Add `pub fn check_*(...)` + classification logic |
| Surgical TOML edit | `src/settings/edit.rs` | Add helper using `toml_edit::DocumentMut` |

### Key Patterns

#### CLI Command Pattern (most commands follow this)

```rust
// src/commands/thing/mod.rs or src/commands/thing.rs

pub fn run(args: ThingArgs, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError> {
    let paths = Paths::resolve()?;
    let outcome = assemble_thing(&args, scope, &paths)?;  // silent compute
    output::write(mode, &outcome);  // emit (human / JSON)
    Ok(())
}

pub fn assemble_thing(args: &ThingArgs, scope: &ResolvedScope, paths: &Paths) -> Result<ThingOutcome, TomeError> {
    // Business logic here; no I/O side effects
    Ok(ThingOutcome { ... })
}
```

#### Harness Module Pattern

```rust
// src/harness/{name}.rs
pub static THE_HARNESS: HarnessModule = HarnessModule { ... };

impl HarnessModule for ... {
    fn name(&self) -> &'static str { ... }
    fn detect(&self, home: &Path) -> bool { ... }
    fn rules_file_target(&self, home: &Path) -> PathBuf { ... }
    fn rules_file_strategy(&self) -> RulesFileStrategy { ... }
    fn block_body_style(&self) -> BlockBodyStyle { ... }
    fn mcp_config_path(&self, home: &Path) -> PathBuf { ... }
    fn mcp_config_format(&self) -> McpConfigFormat { ... }
    fn mcp_parent_key(&self) -> &'static str { ... }
}
```

#### Composition Reference Pattern (Phase 4 US3)

```rust
// In settings TOML files, harnesses list can contain:
// - Bare names: "claude-code", "codex"
// - References: "[workspace]", "[global]", "[workspaces.prod]"
// The resolver recursively expands references into a merged effective list,
// detecting cycles and preserving source chain for debugging.
```

#### Test Fixture Pattern

```rust
// tests/common/mod.rs
lazy_static! {
    pub static ref HOME_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
}

#[must_use]
pub struct HomeGuard {
    _guard: std::sync::MutexGuard<'static, ()>,
    _temp: TempDir,
}

impl HomeGuard {
    pub fn new() -> Result<(Self, PathBuf), Box<dyn std::error::Error>> {
        let _guard = HOME_MUTEX.lock().unwrap();
        let temp = TempDir::new()?;
        let temp_home = temp.path().to_path_buf();
        // Set $HOME to temp_home for test duration; drop HomeGuard to restore
        Ok((Self { _guard, _temp: temp }, temp_home))
    }
}
```

## Generated Files

No auto-generated files in src/; test fixtures are synthesized at runtime (e.g., sparse-file models, synthetic DBs).

---

## What Does NOT Belong Here

- Architecture patterns → ARCHITECTURE.md
- Technology choices → STACK.md
- Code style rules → CONVENTIONS.md
- Test patterns → TESTING.md

---

*This document shows WHERE code lives. Update when directory structure changes.*

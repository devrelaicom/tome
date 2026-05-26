# Project Structure

> **Purpose**: Document directory layout, module boundaries, and where to add new code.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-26

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
│   │   └── lifecycle.rs                # enable/disable/reindex orchestration (per-scope) + trigger regenerate
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
│   │   ├── regen_summary.rs            # Phase 4 NEW: Summariser invocation (US2/US4.b)
│   │   ├── rename.rs                   # Phase 4 NEW: Workspace rename with project updates (US2)
│   │   ├── remove.rs                   # Phase 4 NEW: Workspace removal with 5-step cascade (US2)
│   │   └── sync.rs                     # Phase 4 NEW: Central RULES.md sync to projects (US2)
│   │
│   ├── doctor/                         # Diagnostic + auto-repair (Phase 3 US4 + Phase 4 US5)
│   │   ├── mod.rs                      # assemble_report + re_assemble entry
│   │   ├── checks.rs                   # check_catalogs, check_index, check_drift, check_workspace_registry
│   │   ├── harness_detect.rs           # Probe ~/.claude/, ~/.codex/, ~/.cursor/, ~/.gemini/, ~/.opencode/
│   │   ├── report.rs                   # DoctorReport + Subsystem (typed 11-variant enum) + SubsystemHealth
│   │   ├── fixes.rs                    # apply + apply_one (subsystem routing) + per-subsystem repair handlers
│   │   ├── binding.rs                  # Phase 4 US5 NEW: check_binding (T366) — marker well-formedness + RULES.md drift
│   │   ├── harness_integration.rs      # Phase 4 US5 NEW: check_harness_integration (T367) — per-harness rules/mcp checks
│   │   └── orphan_cleanup.rs           # Phase 4 US5 NEW: cleanup_stale_staging_dirs (FR-410) — 1-hour age gate
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
│   ├── summarise/                      # Phase 4: Workspace summariser (US4)
│   │   ├── mod.rs                      # Summariser trait + input/output types
│   │   ├── llama.rs                    # LlamaSummariser (production, llama-cpp-2, model cached on self)
│   │   ├── stub.rs                     # StubSummariser (deterministic test impl)
│   │   ├── trigger.rs                  # Phase 4 US4.b: regenerate_for_trigger + SummariserOverrideGuard
│   │   ├── registry.rs                 # Pinned summariser model (Qwen2.5-0.5B-Instruct)
│   │   ├── prompts.rs                  # Prompt templates (SHORT_PROMPT, LONG_PROMPT) + length constraints
│   │   └── download.rs                 # Model fetch
│   │
│   ├── commands/                       # CLI command entry points
│   │   ├── mod.rs                      # Public API exports
│   │   ├── catalog.rs                  # `tome catalog {add,remove,list,update,show}`
│   │   ├── plugin/                     # `tome plugin` subcommands
│   │   │   ├── mod.rs                  # Dispatcher + shared helpers
│   │   │   ├── enable.rs               # `tome plugin enable <id>` + trigger regenerate
│   │   │   ├── disable.rs              # `tome plugin disable <id> [--force]` + trigger regenerate
│   │   │   ├── list.rs                 # `tome plugin list`
│   │   │   ├── show.rs                 # `tome plugin show <id>`
│   │   │   └── interactive.rs          # Bare `tome plugin` → three-level TUI
│   │   ├── models/                     # `tome models` subcommands
│   │   │   ├── mod.rs                  # Dispatcher + shared helpers
│   │   │   ├── download.rs             # `tome models download [<name>]`
│   │   │   ├── list.rs                 # `tome models list [--verify]`
│   │   │   └── remove.rs               # `tome models remove <name> [--force]`
│   │   ├── query.rs                    # `tome query [<text>]` + --catalog, --strict, --plain
│   │   ├── reindex.rs                  # `tome reindex [<scope>] [--force]` + trigger regenerate
│   │   ├── status.rs                   # `tome status [--verify]` + --version hook
│   │   ├── workspace/                  # `tome workspace` subcommands (Phase 4 US2/US4)
│   │   │   ├── mod.rs                  # Dispatcher (8 subcommands)
│   │   │   ├── info.rs                 # `tome workspace info [<name>]` — read-only report
│   │   │   ├── init.rs                 # `tome workspace init <name> [--inherit-global] [--force]`
│   │   │   ├── list.rs                 # `tome workspace list` — enumerate all workspaces
│   │   │   ├── use_.rs                 # `tome workspace use <name> [--force]` (bind + sync)
│   │   │   ├── rename.rs               # `tome workspace rename <old> <new>` — rename with project updates
│   │   │   ├── remove.rs               # `tome workspace remove <name> [--force]` — cascade delete
│   │   │   ├── regen_summary.rs        # `tome workspace regen-summary <name>` — explicit regenerate (US4.c)
│   │   │   └── sync.rs                 # `tome workspace sync [<name>]` — sync RULES.md to projects
│   │   ├── harness/                    # Phase 4 US3: Complete harness command surface
│   │   │   ├── mod.rs                  # Dispatcher (6 subcommands) + CentralDbScopeProvider impl
│   │   │   ├── bare.rs                 # `tome harness` (no subcommand) — list all supported harnesses
│   │   │   ├── list.rs                 # `tome harness list [workspace]` — resolve effective harness list
│   │   │   ├── use_.rs                 # `tome harness use <name> [--scope {project|workspace|global}]` + trigger regenerate
│   │   │   ├── remove.rs               # `tome harness remove <name> [--scope]` — delete from settings + trigger regenerate
│   │   │   ├── info.rs                 # `tome harness info` — per-harness details + detection
│   │   │   └── sync.rs                 # `tome harness sync [--force]` — reconcile filesystem
│   │   ├── doctor.rs                   # `tome doctor [--fix] [--verify] [--force]` (US5 adds force flag)
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
│   │   └── atomic_dir.rs               # Atomic directory landing (tempfile + rename); STAGING_PREFIX constant (FR-410)
│   │
│   └── mcp/                            # MCP server (async island, Phase 3+)
│       ├── mod.rs                      # Sync entry point: run()
│       ├── runtime.rs                  # Single-threaded tokio builder
│       ├── log.rs                      # 10 MiB rotate JSON file logger + ContractEventFormat
│       ├── preflight.rs                # FR-110 startup checks (schema, drift, embedder hash)
│       ├── server.rs                   # rmcp server loop + graceful shutdown
│       ├── state.rs                    # McpState definition (embedder, reranker OnceLock)
│       ├── tool_description.rs         # Phase 4 US4.b: Compose runtime tool description from cached summary
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
│   ├── summariser_*.rs                 # Phase 4 US4: Summariser triggers, forward progress, cache, registry tests
│   ├── doctor*.rs                      # Phase 4 US5: Doctor assembly + fixes + binding + harness integration (T366/T367) + orphan cleanup (T370)
│   ├── mcp_*.rs                        # MCP server lifecycle + tools + log rotation + tool description (US4.b)
│   ├── exit_codes.rs                   # Exit code matrix validation
│   ├── manifest_strictness.rs          # Strict/lenient parsing guards
│   ├── atomicity.rs                    # Interrupt-injection tests (SIGINT mid-op)
│   ├── concurrency.rs                  # Two-process index contention
│   ├── schema_migration_e2e.rs         # Forward migration via MIGRATIONS_OVERRIDE
│   ├── sync_boundary.rs                # Structural test: no async outside src/mcp/
│   ├── common/
│   │   ├── mod.rs                      # Test utilities (HOME_MUTEX, HarnessModulesGuard, SummariserOverrideGuard, fixtures)
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
│   └── 004-phase-4-refactor-harnesses/       # Phase 4 (F1–F11 + US1–US5 shipped; Polish phase pending)
│       ├── spec.md
│       ├── plan.md
│       ├── research.md (19 R-decisions)
│       ├── data-model.md (schema v2, Scope reshape, HarnessModule, Summariser, settings layers, Subsystem typed dispatch, ProjectBindingState)
│       ├── contracts/ (13+ contracts: paths-and-layout, harness-modules, settings-composition, sync-algorithm, workspace-commands, summariser, doctor, doctor-extensions-p4, etc.)
│       ├── retro/ (P2.md: F1–F11; P3.md: US1; P4.md–P5.md: US2–US3; P6.md–P7.md: US4; P8.md+: US5)
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
└── CHANGELOG.md                        # Version history (v0.1.0–v0.3.0+, Phase 4 in flight)
```

## Key Directories

### `src/` — Source Code

| Directory | Purpose | Key Files |
|-----------|---------|-----------|
| `catalog/` | Catalog registry, git ops | `manifest.rs`, `store.rs`, `git.rs` |
| `plugin/` | Plugin metadata, lifecycle | `manifest.rs`, `frontmatter.rs`, `lifecycle.rs` |
| `index/` | SQLite + sqlite-vec index | `db.rs`, `schema.rs`, `skills.rs`, `query.rs` |
| `embedding/` | Text embedding + reranking | `fastembed.rs`, `stub.rs`, `download.rs` |
| `workspace/` | Scope resolution, binding, lifecycle | `scope.rs`, `binding.rs`, `init.rs`, `rename.rs`, `remove.rs`, `regen_summary.rs` |
| `harness/` | Phase 4: Harness abstraction + sync | `mod.rs` (trait), 5 harness impls, `sync.rs`, `rules_file.rs`, `mcp_config.rs` |
| `settings/` | Phase 4: Layered composition | `parser.rs`, `resolver.rs` (composition engine), `edit.rs` |
| `summarise/` | Phase 4 US4: Workspace summariser | `llama.rs`, `stub.rs`, `prompts.rs`, `trigger.rs`, `registry.rs` |
| `doctor/` | Phase 4 US5: Health check + auto-repair | `checks.rs`, `fixes.rs`, `binding.rs`, `harness_integration.rs`, `orphan_cleanup.rs` |
| `commands/` | CLI subcommand entry points | Per-command modules + dispatchers |
| `presentation/` | Output formatting + TUI | `tables.rs`, `prompt.rs`, `colour.rs` |
| `util/` | Shared utilities | `atomic_dir.rs` (tempfile + rename) |
| `mcp/` | Async MCP server (Phase 3+) | `runtime.rs`, `server.rs`, `tools/`, `tool_description.rs` (US4.b) |

### `src/doctor/` — Diagnostics & Repair (Phase 4 / US5)

Phase 4 / US5 promotes doctor from a Phase 3 subsystem (models/index/drift/catalog) to a comprehensive whole-system health surface with typed subsystem dispatch and auto-repair framework.

| File | Purpose |
|------|---------|
| `mod.rs` | `assemble_report` + `re_assemble` entry points |
| `checks.rs` | Phase 3: `check_catalogs`, `check_index`, `check_drift`; Phase 4: `check_workspace_registry` |
| `report.rs` | Phase 4 US5: `DoctorReport` struct; typed `Subsystem` enum (11 variants) with custom Ser/Deser wire strings; `SubsystemHealth` enum (5 variants); `ProjectBindingState`; `RulesCopyState` enum (4 variants: Match/Missing/Drift/SourceMissing) |
| `binding.rs` | Phase 4 US5 NEW (T366): `check_binding()` — project marker well-formedness check + workspace registry membership + RULES.md drift via byte-compare |
| `harness_integration.rs` | Phase 4 US5 NEW (T367): `check_harness_integration()` — per-harness rules-file health (standalone vs block-in-file) + MCP-config health (Tome-owned vs user-authored); respects `HARNESS_MODULES_OVERRIDE` |
| `orphan_cleanup.rs` | Phase 4 US5 NEW (T370): `cleanup_stale_staging_dirs()` — sweep `.tome.tmp.*` dirs older than 1 hour from `<root>/workspaces/` + every bound project parent; FR-410 age gate |
| `fixes.rs` | `apply()` per-fix dispatch + `apply_one()` per-subsystem handlers; Phase 4 US5: R-M2 harness sync coalescing, S-M2 user-owned MCP override gate, C-M3 single-project sync for BindingRulesCopy, S-M4 cache-path safety invariant |
| `harness_detect.rs` | Unchanged; probe five well-known harness dirs |

### `src/harness/` — Harness Module Details

Phase 4 / US3 adds complete harness command surface (6 subcommands) backed by production composition resolver.

| File | Purpose |
|------|---------|
| `mod.rs` | `HarnessModule` trait, `SUPPORTED_HARNESSES` registry, `MCP_CONFIG_KEY` static, test injection hook (`HARNESS_MODULES_OVERRIDE`), `with_effective_modules` helper |
| `claude_code.rs` | Claude Code harness: block-in-file rules, JSON MCP config (per-project), description + detection |
| `codex.rs` | Codex harness: block-in-file rules, TOML MCP config (global), description + detection |
| `cursor.rs` | Cursor harness: standalone rules file, JSON MCP config (per-project), description + detection |
| `gemini.rs` | Gemini CLI harness: block-in-file rules, JSON MCP config (global), description + detection |
| `opencode.rs` | OpenCode harness: block-in-file rules (inline strategy), JSON MCP config (per-project), description + detection |
| `rules_file.rs` | Block-in-file + standalone strategies, atomic write helpers, `<!-- tome:begin/end -->` marker management, `@` include directive handling |
| `mcp_config.rs` | Read/write helpers for JSON (preserve_order) + TOML (toml_edit), strict/lenient boundaries |
| `sync.rs` | Sync orchestrator: resolve effective harness list, dispatch per-harness writes, dedup shared paths, cleanup pass, forward-progress on clash, FR-403 (per-harness error tracking) |
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

### `src/summarise/` — Workspace Summariser Details

Phase 4 / US4 implements full summarisation pipeline from trigger to MCP integration.

| File | Purpose |
|------|---------|
| `mod.rs` | `Summariser` trait (identity + summarise method), input/output types, `backend()` singleton entry point |
| `llama.rs` | `LlamaSummariser` production impl via llama-cpp-2 + cached model (US4.d-1 S-M4) |
| `stub.rs` | `StubSummariser` deterministic test impl (returns fixed text) |
| `trigger.rs` | Phase 4 US4.b: `regenerate_for_trigger()` entry, `SUMMARISER_OVERRIDE` thread_local + `SummariserOverrideGuard` RAII, forward-progress invariant (FR-385) |
| `registry.rs` | Pinned Qwen2.5-0.5B-Instruct GGUF entry (model name, files, SHA-256) |
| `prompts.rs` | Fixed `SHORT_PROMPT` + `LONG_PROMPT` templates, length constants (`SHORT_MAX_CHARS=800`, `LONG_MAX_CHARS=2500`) |
| `download.rs` | Model fetch (stub-only in F6) |

### `src/commands/harness/` — Harness Command Surface

Phase 4 / US3 implements full subcommand dispatcher + production `ScopeProvider` impl.

| File | Purpose |
|------|---------|
| `mod.rs` | Dispatcher, `sync_for_project_root()` entry (called by workspace use), `CentralDbScopeProvider` impl (consults workspaces table + reads .toml files) |
| `bare.rs` | `tome harness` (no subcommand) — tabular list of 5 supported harnesses + detection status |
| `list.rs` | `tome harness list [workspace]` — resolve effective harness list via ScopeProvider + composition resolver |
| `use_.rs` | `tome harness use <name> [--scope]` — append harness to settings file via `settings::edit`, run sync on change + trigger regenerate |
| `remove.rs` | `tome harness remove <name> [--scope]` — delete harness from settings file, run cleanup on change + trigger regenerate |
| `info.rs` | `tome harness info [--json]` — per-harness detection, targets, source-of-scope annotation |
| `sync.rs` | `tome harness sync [--force]` — idempotent reconciliation; thin wrapper over `harness::sync::sync_project` |

### `src/mcp/` — MCP Server (Phase 3+ with US4/US5 additions)

Phase 4 / US4.b adds runtime tool description composition from cached summaries; US5 interacts read-only with doctor checks.

| File | Purpose |
|------|---------|
| `mod.rs` | Sync entry point: `run()` |
| `runtime.rs` | Single-threaded tokio builder, lifecycle management |
| `log.rs` | 10 MiB atomic-rotate JSON file logger (contract-formatted for tool logs) |
| `preflight.rs` | Startup checks: schema version, drift, embedder SHA-256, eager load embedder |
| `server.rs` | rmcp tool router, handlers, graceful shutdown on SIGTERM |
| `state.rs` | `McpState` (embedder, reranker OnceLock, scope, paths, ...) |
| `tool_description.rs` | Phase 4 US4.b: Compose runtime description from scaffold + cached short summary (reads settings.toml at startup) |
| `tools/mod.rs` | Tool registration + routing |
| `tools/search_skills.rs` | `search_skills` handler (KNN + rerank, workspace-filtered) |
| `tools/get_skill.rs` | `get_skill` handler (metadata + components walks) |

### `tests/` — Integration Tests

#### Test Files by Phase

| File Pattern | Purpose | Count |
|-----|---------|-------|
| `catalog_*.rs` | Catalog add/remove/update/refcount tests | 8 |
| `plugin_*.rs` | Plugin enable/disable/list/show/interactive | 9 |
| `models_*.rs` | Model download/list/remove | 3 |
| `workspace_*.rs` | Workspace info/init/binding/sync/list/rename/remove (US1–US2) | 28 |
| `harness_*.rs` | Phase 4 US3: Harness list/use/remove/info/sync/composition | 16 |
| `summariser_*.rs` | Phase 4 US4: Triggers, forward-progress, cache, registry, real models | 7 |
| `doctor*.rs` | Phase 4 US5: Fixes, binding (T366), harness integration (T367), orphan cleanup (T370) | 5+ |
| `query.rs` | Query + strict mode + reranking | 1 |
| `reindex.rs` | Reindex all/per-catalog/per-plugin | 1 |
| `status.rs` | Status command + health checks | 1 |
| `mcp_*.rs` | MCP server lifecycle + tools + log + tool description (US4.b) | 8 |
| `exit_codes.rs` | Exit code matrix validation | 1 |
| `manifest_strictness.rs` | Strict/lenient parsing guards | 1 |
| `atomicity.rs` | Interrupt-injection tests (SIGINT mid-op) | 1 |
| `concurrency.rs` | Two-process index contention | 1 |
| `schema_migration_e2e.rs` | Forward migration via MIGRATIONS_OVERRIDE | 1 |
| `sync_boundary.rs` | Structural test: no async outside src/mcp/ | 1 |
| **Total** | 125+ test files, 916 total tests | 125+ |

#### Key Test Fixtures

| File | Purpose |
|------|---------|
| `common/mod.rs` | `HOME_MUTEX`, `HarnessModulesGuard`, `SummariserOverrideGuard`, test-specific helpers, sparse-file model fabricators |
| `fixtures/sample-plugin-catalog/` | Real git-backed plugin tree for catalog add/remove/update tests |

## Module Boundaries

### Where to Add New Code

| If you're adding... | Put it in... | Pattern |
|---------------------|--------------|---------|
| New harness | `src/harness/{name}.rs` + register in `mod.rs` | Impl `HarnessModule` trait (7 methods) |
| New harness subcommand | `src/commands/harness/{cmd}.rs` | Pattern: `run(args, scope, paths, mode)` + `assemble_*(...)` |
| New workspace command | `src/commands/workspace/{cmd}.rs` | Pattern: `run(args, scope, paths, mode)` + `assemble_*(...)` or `run_with_deps(...)` |
| New catalog command | `src/commands/catalog.rs` | Add to existing dispatcher + orchestrator |
| New skill search filter | `src/index/query.rs` | Add to `QueryFilters` struct + SQL |
| New embedder impl | `src/embedding/{name}.rs` | Impl `Embedder` trait + register in tests |
| New summariser impl | `src/summarise/{name}.rs` | Impl `Summariser` trait, add to test injection hook |
| New workspace scope hook | `src/workspace/` + `src/settings/` | Add to scope resolution + composition |
| New settings layer | `src/settings/{layer}.rs` | Add type def to `mod.rs`, parser to `parser.rs` |
| New composition ref type | `src/settings/composition.rs` | Extend `CompositionRef` enum |
| New diagnostic check | `src/doctor/checks.rs` or `binding.rs` or `harness_integration.rs` | Add `pub fn check_*(...)` + classification logic |
| Surgical TOML edit | `src/settings/edit.rs` | Add helper using `toml_edit::DocumentMut` |
| New subsystem (doctor) | `src/doctor/report.rs` | Add variant to `Subsystem` enum + Ser/Deser impl + fix handler to `fixes.rs` |
| Trigger site (enable/disable/etc.) | `src/commands/{cmd}.rs` or `src/plugin/lifecycle.rs` | After mutation commit, call `regenerate_for_trigger(workspace_name, paths)` |

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
pub struct TheHarnessModule;

impl HarnessModule for TheHarnessModule {
    fn name(&self) -> &'static str { ... }
    fn description(&self) -> &'static str { ... }
    fn detect(&self, home: &Path) -> bool { ... }
    fn rules_file_target(&self, project_root: &Path) -> PathBuf { ... }
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

#### Summarisation Trigger Pattern (Phase 4 US4.b)

```rust
// In enable/disable/reindex/catalog-update commands, after workspace_skills mutation commits:

// Commit workspace_skills rows inside one advisory-lock window
index::skills::enable_plugin_atomic(/* ... */)?;
// Lock released here

// Then trigger regeneration (outside lock)
crate::summarise::regenerate_for_trigger(scope.scope.name(), &paths)?;
// Forward-progress: if summariser fails, skill state is retained, cached summary not overwritten
// ModelMissing is silent no-op; other failures exit 24
```

#### Doctor Subsystem Dispatch Pattern (Phase 4 / US5)

```rust
// src/doctor/report.rs — type-safe subsystem dispatch via Subsystem enum
pub enum Subsystem {
    Embedder,
    Reranker,
    Index,
    Drift,
    Catalog(String),
    Schema,
    Summariser,
    Binding,
    BindingRulesCopy,
    HarnessRules(String),
    HarnessMcp(String),
}

// src/doctor/fixes.rs — exhaustive match on enum
match &fix.subsystem {
    Subsystem::Embedder => repair_model(...),
    Subsystem::HarnessMcp(name) => repair_harness_sync_with(...),
    // ...
}
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

#### Test Summariser Injection Pattern (Phase 4 / US4)

```rust
// tests/summariser_*.rs
use tome::summarise::{SummariserOverrideGuard, StubSummariser};

#[test]
fn summariser_trigger_with_stub() -> Result<(), Box<dyn Error>> {
    let (home, _) = HomeGuard::new()?;
    let stub = Arc::new(StubSummariser);
    let _guard = SummariserOverrideGuard::install(stub);  // Installed for test duration
    
    // trigger code path sees SUMMARISER_OVERRIDE, uses stub instead of LlamaSummariser
    // guard drops at end of test, clearing the slot

    Ok(())
}
```

#### Test Harness Module Override Pattern (Phase 4 / US3 + US5)

```rust
// tests/harness_*.rs or tests/doctor*.rs
use tome::harness::HarnessModulesGuard;

#[test]
fn test_with_stub_harness() -> Result<(), Box<dyn Error>> {
    let guard = HarnessModulesGuard::install(vec![/* stub modules */]);
    // During test, HARNESS_MODULES_OVERRIDE is populated
    // with_effective_modules() + lookup() see the override
    // guard drops at end of test
    Ok(())
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

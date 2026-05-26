# Project Structure

> **Purpose**: Document directory layout, module boundaries, and where to add new code.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-26 (Phase 5 / US1 shipped; substitution engine, prompts, entry kind discriminator)

## Directory Layout

```
tome/
├── src/                                # Rust library + binary source
│   ├── main.rs                         # CLI entry: scope resolution, command dispatch, error mapping
│   ├── lib.rs                          # Public exports
│   ├── cli.rs                          # clap derive defs (all commands + global flags)
│   ├── error.rs                        # Closed TomeError enum (30+ variants → exit codes)
│   ├── config.rs                       # config.toml parsing (strict; legacy Phase 3 shape)
│   ├── paths.rs                        # Phase 4: consolidated <home>/.tome/ paths; Phase 5: plugin/workspace data-dir accessors
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
│   │   ├── frontmatter.rs              # SKILL.md + command YAML frontmatter parser (Phase 5: widened fields)
│   │   ├── identity.rs                 # PluginId + Phase 5 NEW: EntryKind enum (Skill | Command)
│   │   ├── components.rs               # Walk skill/command dirs; Phase 5: list_command_files enumerates commands
│   │   └── lifecycle.rs                # enable/disable/reindex orchestration (Phase 5: commands + skills)
│   │
│   ├── substitution/                   # Phase 5 NEW: Variable rendering engine (F3 skeleton + US1 wire)
│   │   ├── mod.rs                      # Public API: render(), SubstitutionError enum
│   │   ├── context.rs                  # SubstitutionContext + SubstitutionContextBuilder + ArgumentValues enum
│   │   ├── builtins.rs                 # {{TOME_*}} placeholder stage (stub in F3; US2 wires {{TOME_PLUGIN_DATA}}, {{TOME_WORKSPACE_DATA}}, {{TOME_WORKSPACE_NAME}})
│   │   ├── env.rs                      # {{$VAR}} env-passthrough stage (stub in F3; US2 wires)
│   │   ├── arguments.rs                # Claude Code $ARGUMENTS / $N / $NAME stage (stub in F3; US3 wires)
│   │   ├── data_dir.rs                 # Lazy plugin/workspace data-dir creation (F3: paths only; US2 wires create_dir_all)
│   │   └── regex_sets.rs               # OnceLock<Regex> slots for compiled stage patterns (uncompiled in F3; US2/US3 populate)
│   │
│   ├── index/                          # Vector search index (SQLite + sqlite-vec)
│   │   ├── mod.rs                      # Public API exports
│   │   ├── db.rs                       # Open, WAL config, schema version check
│   │   ├── schema.rs                   # CREATE TABLE statements + bootstrap (schema v3: Phase 5 addition)
│   │   ├── migrations.rs               # Forward-only schema migrations + framework; Phase 5: v2→v3 migration (kind, when_to_use, searchable, user_invocable columns + backfill)
│   │   ├── vec_ext.rs                  # sqlite-vec extension loader
│   │   ├── skills.rs                   # Phase 5: CRUD over unified skills table with EntryKind discriminator; resolve_entry_body_path helper
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
│   ├── workspace/                      # Scope + context resolution + binding + lifecycle (Phase 3-4, US1 wire-up)
│   │   ├── mod.rs                      # Public API exports
│   │   ├── scope.rs                    # Phase 4: Scope(WorkspaceName) tuple struct
│   │   ├── name.rs                     # WorkspaceName validation + parsing
│   │   ├── resolution.rs               # Workspace vs global determination
│   │   ├── binding.rs                  # Phase 4: Project binding + marker landing (US1.a)
│   │   ├── info.rs                     # WorkspaceInfo report assembly
│   │   ├── init.rs                     # Atomic workspace creation via tempfile
│   │   ├── regen_summary.rs            # Phase 4: Summariser invocation (US2/US4.b)
│   │   ├── rename.rs                   # Phase 4: Workspace rename with project updates (US2)
│   │   ├── remove.rs                   # Phase 4: Workspace removal with 5-step cascade (US2)
│   │   └── sync.rs                     # Phase 4: Central RULES.md sync to projects (US2)
│   │
│   ├── doctor/                         # Diagnostic + auto-repair (Phase 3 US4 + Phase 4 US5)
│   │   ├── mod.rs                      # assemble_report + re_assemble entry
│   │   ├── checks.rs                   # check_catalogs, check_index, check_drift, check_workspace_registry
│   │   ├── harness_detect.rs           # Probe ~/.claude/, ~/.codex/, ~/.cursor/, ~/.gemini/, ~/.opencode/
│   │   ├── report.rs                   # DoctorReport + Subsystem (typed 11-variant enum) + SubsystemHealth
│   │   ├── fixes.rs                    # apply + apply_one (subsystem routing) + per-subsystem repair handlers
│   │   ├── binding.rs                  # Phase 4 US5: check_binding (T366) — marker well-formedness + RULES.md drift
│   │   ├── harness_integration.rs      # Phase 4 US5: check_harness_integration (T367) — per-harness rules/mcp checks
│   │   └── orphan_cleanup.rs           # Phase 4 US5: cleanup_stale_staging_dirs (FR-410) — 1-hour age gate
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
│   │   │   ├── enable.rs               # `tome plugin enable <id>` + trigger regenerate (Phase 5: commands + skills)
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
│   │   ├── atomic_dir.rs               # Atomic directory landing (tempfile + rename); STAGING_PREFIX constant (FR-410)
│   │   └── io.rs                       # Phase 4 Polish: bounded_read_to_string + per-class caps
│   │
│   └── mcp/                            # MCP server (async island, Phase 3+; Phase 5: prompts)
│       ├── mod.rs                      # Sync entry point: run()
│       ├── runtime.rs                  # Single-threaded tokio builder
│       ├── log.rs                      # 10 MiB rotate JSON file logger (contract-formatted for tool logs)
│       ├── preflight.rs                # FR-110 startup checks (schema, drift, embedder hash)
│       ├── server.rs                   # rmcp server loop + graceful shutdown
│       ├── state.rs                    # McpState definition (embedder, reranker OnceLock)
│       ├── tool_description.rs         # Phase 4 US4.b: Compose runtime tool description from cached summary
│       ├── prompt_name.rs              # Phase 5 NEW: Prompt-name derivation (<plugin>__<entry> sanitisation + truncation)
│       ├── prompt_collision.rs         # Phase 5 NEW: Collision detection when entries map to same prompt name
│       ├── prompts.rs                  # Phase 5 NEW: MCP prompts capability (PromptRegistry, PromptRouter hand-rolled)
│       └── tools/                      # MCP tool handlers
│           ├── mod.rs                  # Tool registration
│           ├── search_skills.rs        # search_skills tool (KNN+rerank, workspace-filtered, 4096-char input cap)
│           └── get_skill.rs            # get_skill tool (metadata + components)
│
├── tests/                              # Integration tests (access library as external crate)
│   ├── catalog_*.rs                    # Catalog add/remove/update tests
│   ├── plugin_*.rs                     # Plugin enable/disable/list/show/interactive (Phase 5: commands coverage)
│   ├── models_*.rs                     # Model download/list/remove
│   ├── query.rs                        # Query + strict mode + rerank
│   ├── reindex.rs                      # Reindex all/per-catalog/per-plugin
│   ├── status.rs                       # Status command + health checks
│   ├── workspace_*.rs                  # Workspace info/init/binding/sync/list/rename/remove tests (US1–US2)
│   ├── harness_*.rs                    # Phase 4 US3: Harness list/use/remove/info/sync/composition tests
│   ├── summariser_*.rs                 # Phase 4 US4: Summariser triggers, forward progress, cache, registry tests
│   ├── doctor*.rs                      # Phase 4 US5: Doctor assembly + fixes + binding + harness integration (T366/T367) + orphan cleanup (T370)
│   ├── mcp_*.rs                        # MCP server lifecycle + tools + log rotation + tool description (US4.b) + prompts (US1.b)
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
│       ├── ARCHITECTURE.md             # System design + patterns (Phase 5: substitution, prompts, entry kind)
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
│   ├── 004-phase-4-refactor-harnesses/
│   │   ├── spec.md
│   │   ├── plan.md
│   │   ├── research.md (19 R-decisions)
│   │   ├── data-model.md
│   │   ├── contracts/ (13+ contracts)
│   │   ├── retro/ (P2–P8 retrospectives)
│   │   └── quickstart.md
│   └── 005-phase-5-commands-prompts/        # Phase 5 (F1–F3 + US1 shipped)
│       ├── spec.md
│       ├── plan.md
│       ├── research.md (20 R-decisions)
│       ├── data-model.md (schema v3, EntryKind, SubstitutionContext, PromptRegistry, PromptDescriptor)
│       ├── contracts/ (9+ contracts: exit-codes-p5, schema-migration-p5, entry-schema-p5, substitution-engine, mcp-prompts, etc.)
│       ├── notes/ (Phase 5 research notes: rmcp-prompts-api, etc.)
│       └── quickstart.md
│
├── PRDs/                               # Product requirement documents
│   ├── phase-1.md
│   ├── phase-2.md
│   ├── phase-3.md
│   ├── phase-4.md
│   └── phase-5.md
│
├── Cargo.toml                          # Package definition (MSRV 1.93, v0.5.0-dev)
├── Cargo.lock                          # Dependency lock
├── build.rs                            # sqlite-vec C extension compilation
├── CONSTITUTION.md                     # v1.3.0 — constraints + trade-offs (Phase 4 §Paths amendment; no Phase 5 amendments)
├── CLAUDE.md                           # Project context for Claude Code (Phase 5 planning complete; v0.5.0 roadmap)
└── CHANGELOG.md                        # Version history (v0.1.0–v0.4.0 shipped; Phase 5 in flight)
```

## Key Directories

### `src/` — Source Code

| Directory | Purpose | Key Files |
|-----------|---------|-----------|
| `substitution/` | Phase 5 NEW: Variable rendering engine | `context.rs`, `builtins.rs`, `env.rs`, `arguments.rs`, `data_dir.rs`, `regex_sets.rs` |
| `plugin/` | Plugin metadata, lifecycle (Phase 5: commands) | `manifest.rs`, `frontmatter.rs`, `identity.rs` (EntryKind), `components.rs` (list_command_files), `lifecycle.rs` |
| `index/` | SQLite + sqlite-vec index (Phase 5: v3 schema) | `db.rs`, `schema.rs`, `migrations.rs` (v2→v3), `skills.rs` (EntryKind), `query.rs` |
| `mcp/` | MCP server + Phase 5 prompts | `prompts.rs` (PromptRegistry), `prompt_name.rs`, `prompt_collision.rs`, `tools/` |
| `catalog/` | Catalog registry, git ops | `manifest.rs`, `store.rs`, `git.rs` |
| `embedding/` | Text embedding + reranking | `fastembed.rs`, `stub.rs`, `download.rs` |
| `workspace/` | Scope resolution, binding, lifecycle | `scope.rs`, `binding.rs`, `init.rs`, `rename.rs`, `remove.rs`, `regen_summary.rs` |
| `harness/` | Phase 4: Harness abstraction + sync | `mod.rs` (trait), 5 harness impls, `sync.rs`, `rules_file.rs`, `mcp_config.rs` |
| `settings/` | Phase 4: Layered composition | `parser.rs`, `resolver.rs` (composition engine), `edit.rs` |
| `summarise/` | Phase 4: Workspace summariser | `llama.rs`, `stub.rs`, `prompts.rs`, `trigger.rs`, `registry.rs` |
| `doctor/` | Phase 4: Health check + auto-repair | `checks.rs`, `fixes.rs`, `binding.rs`, `harness_integration.rs`, `orphan_cleanup.rs` |
| `commands/` | CLI subcommand entry points | Per-command modules + dispatchers |
| `presentation/` | Output formatting + TUI | `tables.rs`, `prompt.rs`, `colour.rs` |
| `util/` | Shared utilities | `atomic_dir.rs` (tempfile + rename), `io.rs` (bounded read) |
| `paths.rs` | Phase 4 single-root layout; Phase 5: data-dir accessors | `home_root()`, `Paths struct`, `plugin_data_dir_for()`, `workspace_data_dir_for()` |

### `src/substitution/` — Substitution Engine Details (Phase 5 / F3 + US1)

| File | Purpose |
|------|---------|
| `mod.rs` | `render(body, context) -> Result<String, SubstitutionError>` entry point; `SubstitutionError` enum (4 variants) |
| `context.rs` | `SubstitutionContext` + `SubstitutionContextBuilder`; `ArgumentValues` enum (named/positional) |
| `builtins.rs` | Stage 1: `{{TOME_*}}` built-ins (stub in F3; US2 wires real implementations) |
| `env.rs` | Stage 2: `{{$VAR}}` env passthrough (stub in F3; US2 wires) |
| `arguments.rs` | Stage 3: Claude Code `$ARGUMENTS` / `$N` / `$NAME` (stub in F3; US3 wires) |
| `data_dir.rs` | Lazy plugin/workspace data-dir creation (F3: path computation only; US2 wires `create_dir_all`) |
| `regex_sets.rs` | `OnceLock<Regex>` slots for compiled patterns (uncompiled in F3; US2/US3 populate at startup) |

### `src/mcp/` — MCP Prompts Details (Phase 5 / US1)

| File | Purpose |
|------|---------|
| `prompts.rs` | `PromptRegistry` + `PromptEntry`; hand-rolled `PromptRouter` via rmcp; `PromptsCapability` declaration |
| `prompt_name.rs` | Prompt-name derivation: `<plugin>__<entry>` with sanitisation (`[a-z0-9_-]`), truncation (16+32 caps), override support |
| `prompt_collision.rs` | Collision detection: `CollisionRecord { prompt_name, entries }`; `resolve_collisions(registry)` |
| `tool_description.rs` | Phase 4 US4.b preserved: compose runtime description from scaffold + cached summary |
| `tools/search_skills.rs` | KNN+rerank handler; unchanged but now indexed alongside commands |
| `tools/get_skill.rs` | Metadata + components handler; now routes to skills/commands via `resolve_entry_body_path` |

### `src/index/` — Schema v3 & Entry Records (Phase 5 / US1)

| File | Purpose |
|------|---------|
| `schema.rs` | DDL for v3 schema: adds `kind` column (VARCHAR: skill/command); adds `when_to_use` (nullable TEXT); adds `searchable`, `user_invocable` (BOOLEAN with defaults) |
| `migrations.rs` | Phase 5 v2→v3 forward migration: schema changes + backfill logic (kind via directory walk, searchable/user_invocable defaults per contract) |
| `skills.rs` | `SkillRecord` struct extended with `kind: EntryKind`, `when_to_use: Option<String>`, `searchable: bool`, `user_invocable: bool`; new `resolve_entry_body_path(catalog, plugin, name, kind) -> PathBuf` helper (routes via kind) |

### `src/plugin/` — Commands & Entries (Phase 5 / US1)

| File | Purpose |
|------|---------|
| `identity.rs` | `PluginId` (unchanged); **NEW**: `EntryKind` enum (`Skill` \| `Command`) with `as_str()` accessor |
| `frontmatter.rs` | `SkillFrontmatter` widened with `arguments: Option<Vec<PromptArgument>>`, `argument_hint: Option<String>`, `prompt_name: Option<String>`, `when_to_use: Option<String>`, `searchable: Option<bool>` (default true), `user_invocable: Option<bool>` (default false) |
| `components.rs` | `count_components` (unchanged); **NEW**: `list_command_files(plugin_dir) -> Vec<CommandFile>` enumerates `<plugin>/commands/*.md` flat; `CommandFile { path, name }` |
| `lifecycle.rs` | `enable_plugin` now calls `list_command_files` and collects `PendingCommand` structs alongside `PendingSkill` |

### `src/paths.rs` — Data Directory Accessors (Phase 5 / US1)

| Method | Returns | Purpose |
|--------|---------|---------|
| `plugin_data_dir_for(catalog, plugin)` | `<root>/plugin-data/<catalog>/<plugin>/` | Process-wide plugin scratch space |
| `workspace_data_dir_for(workspace, catalog, plugin)` | `<root>/workspaces/<name>/plugin-data/<catalog>/<plugin>/` | Workspace-scoped plugin scratch space |
| `workspace_dir(workspace)` | `<root>/workspaces/<name>/` | Workspace root (unchanged Phase 4) |

## Module Boundaries

### Where to Add New Code (Phase 5 Updates)

| If you're adding... | Put it in... | Pattern |
|---------------------|--------------|---------|
| New substitution stage | `src/substitution/{stage}.rs` | Stage 1-4 namespace; OnceLock<Regex> in `regex_sets.rs` |
| New built-in variable | `src/substitution/builtins.rs` | Add case to match block; test via `SubstitutionContext` |
| New entry kind | `src/plugin/identity.rs` | Extend `EntryKind` enum; update Ser/Deser; backfill migration |
| Command-specific field | `src/plugin/frontmatter.rs` | Extend `SkillFrontmatter` (lenient parsing); document default |
| Command collection | `src/plugin/lifecycle.rs` | Call `list_command_files`; parse frontmatter; build `PendingCommand` |
| MCP prompt handler | `src/mcp/prompts.rs` | Register route via `PromptRouter::new_dyn`; implement request handler |
| Prompt name edge case | `src/mcp/prompt_name.rs` | Extend `sanitise` / `sanitise_trunc` logic; test Unicode boundaries |
| Prompt collision policy | `src/mcp/prompt_collision.rs` | Extend `resolve_collisions` detection; update warning message |
| Entry body resolution | `src/index/skills.rs` | Update `resolve_entry_body_path` match arms per new kind |
| Schema backfill | `src/index/migrations.rs` | Add new v2→v3 backfill step; test via synthetic DB |
| New harness | `src/harness/{name}.rs` + register in `mod.rs` | Impl `HarnessModule` trait (7 methods) |
| New workspace command | `src/commands/workspace/{cmd}.rs` | Pattern: `run(args, scope, paths, mode)` + `assemble_*` |
| Surgical TOML edit | `src/settings/edit.rs` | Add helper using `toml_edit::DocumentMut` |
| New diagnostic check | `src/doctor/checks.rs` or `binding.rs` or `harness_integration.rs` | Add `pub fn check_*` + classification logic |
| New subsystem (doctor) | `src/doctor/report.rs` | Add variant to `Subsystem` enum + Ser/Deser impl + fix handler to `fixes.rs` |

### Key Patterns

#### Substitution Context Pattern (Phase 5 / US1+US2+US3)

```rust
// src/substitution/context.rs

pub struct SubstitutionContext {
    pub entry: EntryIdentity,  // catalog, plugin, name, kind
    pub workspace: WorkspaceName,
    pub arguments: ArgumentValues,  // named or positional
}

pub struct SubstitutionContextBuilder { ... }

impl SubstitutionContextBuilder {
    pub fn build(self) -> Result<SubstitutionContext, SubstitutionError> { ... }
}

// Consumer calls:
let context = SubstitutionContextBuilder::new(entry, workspace)
    .with_arguments(arguments)?
    .build()?;

let rendered = substitution::render(&body, &context)?;
```

#### Entry Kind Pattern (Phase 5 / US1)

```rust
// src/plugin/identity.rs

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    Skill,
    Command,
}

// In database & wire format: "skill" or "command"
// In lifecycle: discriminates directory walk (skills/ vs commands/)
// In MCP prompts: routes to resolve_entry_body_path(catalog, plugin, name, kind)
```

#### Command Entry Collection Pattern (Phase 5 / US1)

```rust
// src/plugin/lifecycle.rs

pub async fn collect_pending_commands(
    plugin_dir: &Path,
    catalog: &str,
    plugin: &str,
    plugin_version: &str,
) -> Result<Vec<PendingCommand>, TomeError> {
    let files = plugin::components::list_command_files(plugin_dir);
    let mut pending = Vec::new();
    for file in files {
        let body = fs::read_to_string(&file.path)?;
        let (frontmatter, _) = parse_command_frontmatter(&body)?;
        pending.push(PendingCommand {
            catalog: catalog.to_owned(),
            plugin: plugin.to_owned(),
            name: frontmatter.name.or(Some(file.name))?,
            kind: EntryKind::Command,
            description: frontmatter.description?,
            // ... other fields
        });
    }
    Ok(pending)
}
```

#### MCP Prompt Registration Pattern (Phase 5 / US1)

```rust
// src/mcp/prompts.rs

pub fn build_prompt_router(
    registry: &PromptRegistry,
    db: &Connection,
) -> Result<PromptRouter, TomeError> {
    let mut router = PromptRouter::new();
    
    for (prompt_name, entry) in &registry.by_name {
        let handler = {
            let prompt_name = prompt_name.clone();
            let entry = entry.clone();
            move |ctx: PromptContext| -> Pin<Box<dyn Future<Output = Result<PromptGetResponse, McpError>>>> {
                Box::pin(async move {
                    // Handle prompt request: read entry body, render via substitution, return
                    let (body, _) = resolve_entry_body_path(&entry.catalog, &entry.plugin, &entry.name, entry.kind)?;
                    Ok(PromptGetResponse { messages: vec![...] })
                })
            }
        };
        
        router.add_route(PromptRoute::new_dyn(
            prompt_name.clone(),
            PromptDescriptor {
                name: prompt_name.clone(),
                description: entry.description.clone(),
                arguments: entry.arguments.clone(),
            },
            handler,
        ));
    }
    
    Ok(router)
}
```

#### Test Entry Kind Override Pattern

```rust
// tests/common/mod.rs or test file

#[must_use]
pub struct EntryKindOverrideGuard { ... }

impl EntryKindOverrideGuard {
    pub fn install(overrides: Vec<(PluginId, Vec<EntryKind>)>) -> Self {
        // Set ENTRY_KIND_OVERRIDE thread_local
    }
}

// In test:
#[test]
fn command_entry_kind_preserved() -> Result<(), Box<dyn Error>> {
    let guard = EntryKindOverrideGuard::install(vec![(
        "catalog/plugin".parse()?,
        vec![EntryKind::Command],
    )]);
    
    // Test code sees overridden entry kinds
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

*This document shows WHERE code lives. Updated 2026-05-26 against Phase 5 / US1 (substitution skeleton, prompts, entry kind shipped).*

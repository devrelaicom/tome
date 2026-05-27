# Project Structure

> **Purpose**: Document directory layout, module boundaries, and where to add new code.
> **Generated**: 2026-05-27
> **Last Updated**: 2026-05-27 (Phase 5 Polish-complete; per-entry invocability + doctor read-only extensions + single-source-of-truth promotion; 1193 tests)

## Directory Layout

```
tome/
├── src/                                # Rust library + binary source
│   ├── main.rs                         # CLI entry: scope resolution, command dispatch, error mapping
│   ├── lib.rs                          # Public exports
│   ├── cli.rs                          # clap derive defs (all commands + global flags)
│   ├── error.rs                        # Closed TomeError enum (30+ variants → exit codes)
│   ├── config.rs                       # config.toml parsing (strict; legacy Phase 3 shape)
│   ├── paths.rs                        # Phase 4: consolidated <home>/.tome/ paths; Phase 5: plugin/workspace data-dir accessors + plugin_data_root() SSOT
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
│   │   ├── frontmatter.rs              # SKILL.md + command YAML frontmatter parser (Phase 5: widened fields including arguments schema, when_to_use, user_invocable for MCP exposure)
│   │   ├── identity.rs                 # PluginId + Phase 5 NEW: EntryKind enum (Skill | Command) + canonical from_str()
│   │   ├── components.rs               # Walk skill/command dirs; Phase 5: list_command_files enumerates commands
│   │   └── lifecycle.rs                # enable/disable/reindex orchestration (Phase 5: commands + skills)
│   │
│   ├── substitution/                   # Phase 5 / US1–US3: Variable rendering engine (COMPLETE)
│   │   ├── mod.rs                      # Public API: render(), body_has_bare_arguments() helper; SubstitutionError enum (6 variants); COMBINED_RE single-pass loop (US2); ARGUMENTS footer tail (US3)
│   │   ├── context.rs                  # SubstitutionContext + SubstitutionContextBuilder + ArgumentValues enum (named + positional pairs)
│   │   ├── builtins.rs                 # Stage 1 handler: {{TOME_PLUGIN_DATA}}, {{TOME_WORKSPACE_DATA}}, {{TOME_WORKSPACE_NAME}}, {{TOME_CATALOG_NAME}}, {{TOME_PLUGIN_NAME}} (US2)
│   │   ├── env.rs                      # Stage 2 handler: {{$VAR}} env-passthrough + TOME_ENV_ prefix (US2)
│   │   ├── arguments.rs                # Stage 3 handler: Claude Code $ARGUMENTS / $N / $NAME (US3); shell_split + coerce_arguments + apply_arguments_match pipeline
│   │   ├── data_dir.rs                 # Lazy plugin/workspace data-dir creation via ensure_plugin_data() / ensure_workspace_data() (US2)
│   │   └── regex_sets.rs               # OnceLock<Regex> COMBINED_RE (union of all stage patterns, compiled once at startup per US2)
│   │
│   ├── index/                          # Vector search index (SQLite + sqlite-vec)
│   │   ├── mod.rs                      # Public API exports
│   │   ├── db.rs                       # Open, WAL config, schema version check
│   │   ├── schema.rs                   # CREATE TABLE statements + bootstrap (schema v3: Phase 5 addition)
│   │   ├── migrations.rs               # Forward-only schema migrations + framework; Phase 5: v2→v3 migration (kind, when_to_use, searchable, user_invocable columns + backfill)
│   │   ├── vec_ext.rs                  # sqlite-vec extension loader
│   │   ├── skills.rs                   # Phase 5: CRUD over unified skills table with EntryKind discriminator; resolve_entry_body_path + validate_db_stored_path SSOT (Polish)
│   │   ├── query.rs                    # KNN search (workspace-filtered) + optional reranking (Phase 5 / US4: search includes when_to_use embeddings)
│   │   ├── meta.rs                     # Model identity metadata + drift detection
│   │   ├── integrity.rs                # PRAGMA integrity_check wrapper
│   │   ├── lock.rs                     # Advisory lockfile acquisition
│   │   ├── workspace_catalogs.rs       # Phase 4: junction table CRUD (workspace → catalogs)
│   │   └── workspaces.rs               # Phase 4: workspace name resolution (ID lookups)
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
│   │   ├── rename.rs                   # Phase 4: Workspace rename with project updates (US2); Phase 5 / US2: plugin-data relocation
│   │   ├── remove.rs                   # Phase 4: Workspace removal with 5-step cascade (US2)
│   │   └── sync.rs                     # Phase 4: Central RULES.md sync to projects (US2)
│   │
│   ├── doctor/                         # Diagnostic + auto-repair (Phase 3 US4 + Phase 4 US5 + Phase 5 US5)
│   │   ├── mod.rs                      # assemble_report + re_assemble entry
│   │   ├── checks.rs                   # check_catalogs, check_index, check_drift, check_workspace_registry + Phase 5 / US5: build_prompts_report, count_entries_by_kind, detect_orphan_data_dirs (all read-only)
│   │   ├── harness_detect.rs           # Probe ~/.claude/, ~/.codex/, ~/.cursor/, ~/.gemini/, ~/.opencode/
│   │   ├── report.rs                   # DoctorReport + Subsystem (typed 11-variant enum) + SubsystemHealth + Phase 5 / US5: PromptsReport, EntryCountsByKind, OrphanDataDirReport
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
│   │   │   ├── list.rs                 # `tome plugin list` (Phase 5 / US5: per-kind entry counts)
│   │   │   ├── show.rs                 # `tome plugin show <id>` (Phase 5 / US5: ~228 lines extended for searchable/invocable annotations + kind grouping + Polish: validate_db_stored_path)
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
│   │   │   ├── rename.rs               # `tome workspace rename <old> <new>` — rename with project updates + plugin-data relocation (US2)
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
│   │   ├── doctor.rs                   # `tome doctor [--fix] [--verify] [--force]` (Phase 5 / US5: renders extended report with prompts + entry-kind counts + orphan data-dirs)
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
│   └── mcp/                            # MCP server (async island, Phase 3+; Phase 5: prompts + US4 three-tier discovery + US5 read-only extensions + Polish: substitution_helpers)
│       ├── mod.rs                      # Sync entry point: run()
│       ├── runtime.rs                  # Single-threaded tokio builder
│       ├── log.rs                      # 10 MiB rotate JSON file logger (contract-formatted for tool logs)
│       ├── preflight.rs                # FR-110 startup checks (schema, drift, embedder hash)
│       ├── server.rs                   # rmcp server loop + graceful shutdown
│       ├── state.rs                    # McpState definition (embedder, reranker OnceLock)
│       ├── substitution_helpers.rs     # **Phase 5 Polish NEW** build_context_for_entry() SSOT (shared across prompts/get + get_skill_info)
│       ├── tool_description.rs         # Phase 4 US4.b: Compose runtime tool description from cached summary
│       ├── prompt_name.rs              # Phase 5 NEW: Prompt-name derivation (<plugin>__<entry> sanitisation + truncation)
│       ├── prompt_collision.rs         # Phase 5 NEW: Collision detection when entries map to same prompt name
│       ├── prompts.rs                  # Phase 5 NEW: MCP prompts capability (PromptRegistry, PromptRouter hand-rolled)
│       └── tools/                      # MCP tool handlers (Phase 5 / US4–US5: three-tier discovery + read-only extensions)
│           ├── mod.rs                  # Tool registration
│           ├── search_skills.rs        # search_skills tool (KNN+rerank, workspace-filtered, 4096-char input cap, Phase 5 / US4: when_to_use in results, truncate_description hardening; Polish: mirrors truncation at get_skill_info)
│           ├── get_skill_info.rs       # **Phase 5 / US4 NEW** get_skill_info middle-tier tool (full description + when_to_use + 5-cap resource enumeration; Polish: uses build_context_for_entry SSOT)
│           └── get_skill.rs            # get_skill tool (metadata + components)
│
├── tests/                              # Integration tests (access library as external crate)
│   ├── catalog_*.rs                    # Catalog add/remove/update tests
│   ├── plugin_*.rs                     # Plugin enable/disable/list/show/interactive (Phase 5: commands coverage + US5 annotations)
│   ├── models_*.rs                     # Model download/list/remove
│   ├── query.rs                        # Query + strict mode + rerank
│   ├── reindex.rs                      # Reindex all/per-catalog/per-plugin
│   ├── status.rs                       # Status command + health checks
│   ├── workspace_*.rs                  # Workspace info/init/binding/sync/list/rename/remove tests (US1–US2)
│   ├── harness_*.rs                    # Phase 4 US3: Harness list/use/remove/info/sync/composition tests
│   ├── summariser_*.rs                 # Phase 4 US4: Summariser triggers, forward progress, cache, registry tests
│   ├── doctor*.rs                      # Phase 4 US5: Doctor assembly + fixes + binding + harness integration + orphan cleanup; Phase 5 / US5: prompts report + entry counts + orphan data-dirs
│   ├── mcp_*.rs                        # MCP server lifecycle + tools + log rotation + tool description (US4.b) + prompts (US1.b) + **Phase 5 / US4–US5: get_skill_info tests + read-only extensions**
│   ├── substitution_*.rs               # Phase 5: Substitution engine tests (skeleton, builtins, env, arguments, data-dir, e2e)
│   ├── entry_e2e.rs                    # Phase 5 / US3 NEW: Full enable → search → get → prompts pipeline with argument substitution + Phase 5 / US5: invocability visibility
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
│       ├── ARCHITECTURE.md             # System design + patterns (Phase 5 / US5: per-entry invocability + doctor read-only extensions; Polish: single-source-of-truth promotion)
│       ├── STRUCTURE.md                # Directory layout (this file; Polish: substitution_helpers + validate_db_stored_path SSOT)
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
│   └── 005-phase-5-commands-prompts/        # Phase 5 (F1–F3 + US1–US5 shipped + Polish complete)
│       ├── spec.md
│       ├── plan.md
│       ├── research.md (20 R-decisions)
│       ├── data-model.md (schema v3, EntryKind, SubstitutionContext, ArgumentValues, PromptRegistry, ResourceEnumeration, PromptsReport, EntryCountsByKind, OrphanDataDirReport)
│       ├── contracts/ (9+ contracts: exit-codes-p5, schema-migration-p5, entry-schema-p5, substitution-engine, mcp-tools-p5, mcp-prompts, etc.)
│       ├── notes/ (Phase 5 research notes: rmcp-prompts-api, argument-coercion, three-tier discovery, when-to-use-indexing)
│       ├── review/ (Phase 5 reviewer findings + disposition per US)
│       ├── retro/ (P3–P8 retrospectives; P9 Polish retro forthcoming)
│       └── quickstart.md
│
├── PRDs/                               # Product requirement documents
│   ├── phase-1.md
│   ├── phase-2.md
│   ├── phase-3.md
│   ├── phase-4.md
│   └── phase-5.md
│
├── Cargo.toml                          # Package definition (MSRV 1.93, v0.5.0)
├── Cargo.lock                          # Dependency lock
├── build.rs                            # sqlite-vec C extension compilation
├── CONSTITUTION.md                     # v1.3.0 — constraints + trade-offs (Phase 4 §Paths amendment; no Phase 5 amendments)
├── CLAUDE.md                           # Project context for Claude Code (Phase 5 complete + Polish shipped; v0.5.0 final)
└── CHANGELOG.md                        # Version history (v0.1.0–v0.5.0 shipped)
```

## Key Directories

### `src/` — Source Code

| Directory | Purpose | Key Files |
|-----------|---------|-----------|
| `substitution/` | Phase 5 / US1–US3: Variable rendering engine (COMPLETE single-pass pipeline) | `mod.rs` (render loop + body_has_bare_arguments), `context.rs`, `builtins.rs`, `env.rs`, `arguments.rs` (shell_split + coerce_arguments + apply_arguments_match), `data_dir.rs`, `regex_sets.rs` (COMBINED_RE) |
| `plugin/` | Plugin metadata, lifecycle (Phase 5: commands + arguments + when_to_use + user_invocable) | `manifest.rs`, `frontmatter.rs`, `identity.rs` (EntryKind + canonical from_str), `components.rs` (list_command_files), `lifecycle.rs` |
| `index/` | SQLite + sqlite-vec index (Phase 5: v3 schema with when_to_use; Polish: validate_db_stored_path SSOT) | `db.rs`, `schema.rs`, `migrations.rs` (v2→v3), `skills.rs` (EntryKind + when_to_use + validate_db_stored_path), `query.rs` (Phase 5 / US4: when_to_use embeddings) |
| `mcp/` | MCP server + Phase 5 prompts + three-tier discovery + read-only extensions + Polish: substitution_helpers | `prompts.rs` (PromptRegistry), `prompt_name.rs`, `prompt_collision.rs`, `substitution_helpers.rs` (build_context_for_entry SSOT), `tools/` (search_skills, **get_skill_info**, get_skill) |
| `doctor/` | Health check + auto-repair (Phase 5 / US5: read-only extensions) | `checks.rs` (build_prompts_report, count_entries_by_kind, detect_orphan_data_dirs), `report.rs` (PromptsReport, EntryCountsByKind, OrphanDataDirReport) |
| `catalog/` | Catalog registry, git ops | `manifest.rs`, `store.rs`, `git.rs` |
| `embedding/` | Text embedding + reranking | `fastembed.rs`, `stub.rs`, `download.rs` |
| `workspace/` | Scope resolution, binding, lifecycle (Phase 5 / US2: rename relocation) | `scope.rs`, `binding.rs`, `init.rs`, `rename.rs`, `remove.rs`, `regen_summary.rs` |
| `harness/` | Phase 4: Harness abstraction + sync | `mod.rs` (trait), 5 harness impls, `sync.rs`, `rules_file.rs`, `mcp_config.rs` |
| `settings/` | Phase 4: Layered composition | `parser.rs`, `resolver.rs` (composition engine), `edit.rs` |
| `summarise/` | Phase 4: Workspace summariser | `llama.rs`, `stub.rs`, `prompts.rs`, `trigger.rs`, `registry.rs` |
| `commands/` | CLI subcommand entry points (Phase 5 / US5: show + list extended; Polish: validate_db_stored_path) | Per-command modules + dispatchers |
| `presentation/` | Output formatting + TUI | `tables.rs`, `prompt.rs`, `colour.rs` |
| `util/` | Shared utilities | `atomic_dir.rs` (tempfile + rename), `io.rs` (bounded read) |
| `paths.rs` | Phase 4 single-root layout; Phase 5: data-dir accessors; Polish: plugin_data_root() SSOT | `home_root()`, `Paths struct`, `plugin_data_root()` SSOT, `plugin_data_dir_for()`, `workspace_data_dir_for()` |

### `src/substitution/` — Substitution Engine Details (Phase 5 / US1–US3 COMPLETE)

| File | Purpose | Phase 5 / US3 Status |
|------|---------|---------------------|
| `mod.rs` | Single-pass `render(body, context)` entry point (COMBINED_RE loop); `body_has_bare_arguments(body) -> bool` helper; `SubstitutionError` enum (6 variants); ARGUMENTS footer appended in render tail | Production-ready; all four stages dispatched in one loop; ARGUMENTS tail appended after inline substitutions complete |
| `context.rs` | `SubstitutionContext` + `SubstitutionContextBuilder`; `ArgumentValues` enum (named + positional pairs) | Phase 5 / US3: ArgumentValues fully populated during coerce_arguments validation |
| `builtins.rs` | Stage 1 handler: `{{TOME_PLUGIN_DATA}}`, `{{TOME_WORKSPACE_DATA}}`, `{{TOME_WORKSPACE_NAME}}`, `{{TOME_CATALOG_NAME}}`, `{{TOME_PLUGIN_NAME}}` | Wired in US2; lazy data-dir creation triggered on first match |
| `env.rs` | Stage 2 handler: `{{$VAR}}` env-passthrough (TOME_ENV_ prefix) | Wired in US2; handles both `{{$NAME}}` and TOME_ENV_ prefix variants |
| `arguments.rs` | **Phase 5 / US3 COMPLETE** Stage 3 handler: Claude Code `$ARGUMENTS` / `$N` / `$NAME` with three sub-pipelines: `shell_split()` (POSIX quoting parser) → `coerce_arguments()` (match to declared schema) → `apply_arguments_match()` (resolve placeholders) | Full implementation: handles positional ($1, $2, ...), named ($name), and catch-all ($ARGUMENTS); POSIX shell quoting; frontmatter validation |
| `data_dir.rs` | Lazy creation: `ensure_plugin_data()` / `ensure_workspace_data()` | Wired in US2; creates dirs on first `{{TOME_*}}` reference during render |
| `regex_sets.rs` | `OnceLock<Regex>` COMBINED_RE (compiled once at startup) | Populated in US2 via union of all stage patterns (builtins + env + arguments); production dispatch uses `captures_iter` |

### `src/mcp/tools/` — Three-Tier Discovery (Phase 5 / US4, Polish: shared context)

| File | Purpose | Phase 5 / US4 Status | Polish |
|------|---------|---------------------|--------|
| `search_skills.rs` | **Tier 1**: KNN+rerank (5–10 results); truncated descriptions (512 chars default); first 100 chars when_to_use | **Phase 5 / US4 C-1**: truncate_description via char_indices fast-path (O(1) if no truncation, O(k) if truncation at pos k) | Mirrored pattern at get_skill_info |
| `get_skill_info.rs` | **Tier 2 NEW**: Full description (no truncation), when_to_use, plugin_version, user_invocable, **resources (skill-only)** — files + directories with 5-cap per level + "and N more" sentinel | Phase 5 / US4 T303–T308; resource enumeration walks parent dir non-recursively, skips symlinks, BTreeMap for alphabetical JSON order | **Polish**: Uses build_context_for_entry() SSOT (eliminates ~50 LOC duplication with prompts/get) |
| `get_skill.rs` | **Tier 3**: Complete body + components | Unchanged from Phase 5 / US1–US3 | |

### `src/mcp/` — MCP Prompts Details + Polish (Phase 5 / US1, Polish: substitution_helpers)

| File | Purpose | Phase 5 / US1 | Polish |
|------|---------|-------------|--------|
| `prompts.rs` | `PromptRegistry` + `PromptEntry`; hand-rolled `PromptRouter` via rmcp; `PromptsCapability` declaration | Phase 5 / US1 complete | |
| `prompt_name.rs` | Prompt-name derivation: `<plugin>__<entry>` with sanitisation (`[a-z0-9_-]`), truncation (16+32 caps), override support | Phase 5 / US1 complete | |
| `prompt_collision.rs` | Collision detection: `CollisionRecord { prompt_name, entries }`; `resolve_collisions(registry)` | Phase 5 / US1 complete | |
| `tool_description.rs` | Phase 4 US4.b preserved: compose runtime description from scaffold + cached summary | Phase 4 preserved | |
| `substitution_helpers.rs` | **Phase 5 Polish NEW**: `build_context_for_entry()` SSOT (shared across `prompts/get` + `get_skill_info`); eliminates ~50 LOC duplication | | **NEW**: Centralizes plugin version lookup, entry body reading, frontmatter parsing, arguments schema extraction |

### `src/index/` — Schema v3 & Entry Records (Phase 5 / US1, Polish: SSOT for validation)

| File | Purpose | Phase 5 / US1 | Polish |
|------|---------|-------------|--------|
| `schema.rs` | DDL for v3 schema: adds `kind` column (VARCHAR: skill/command); adds `when_to_use` (nullable TEXT); adds `searchable`, `user_invocable` (BOOLEAN with defaults) | Phase 5 / US1 complete | |
| `migrations.rs` | Phase 5 v2→v3 forward migration: schema changes + backfill logic (kind via directory walk, searchable/user_invocable defaults per contract) | Phase 5 / US1 complete | |
| `skills.rs` | `SkillRecord` struct extended with `kind: EntryKind`, `when_to_use: Option<String>`, `searchable: bool`, `user_invocable: bool`; `resolve_entry_body_path(...)` helper (routes via kind); **Phase 5 Polish: `validate_db_stored_path()` SSOT** | Phase 5 / US1 complete | **NEW**: validate_db_stored_path() promoted to pub(crate) as canonical boundary-check (consumed by get_skill + commands/plugin/show) |

### `src/plugin/` — Commands & Entries (Phase 5 / US1–US5, Polish: canonical from_str)

| File | Purpose | Phase 5 | Polish |
|------|---------|---------|--------|
| `identity.rs` | `PluginId` (unchanged); **NEW**: `EntryKind` enum (`Skill` \| `Command`) with `as_str()` accessor | Phase 5 / US1 complete | **NEW**: Canonical `EntryKind::from_str()` consumed at six sites |
| `frontmatter.rs` | `SkillFrontmatter` widened with `arguments: Option<Vec<PromptArgument>>` (ordered list of declared parameters), `argument_hint: Option<String>`, `prompt_name: Option<String>`, `when_to_use: Option<String>` (**Phase 5 / US4: now indexed for semantic search**), `searchable: Option<bool>` (default true), `user_invocable: Option<bool>` (default false, **Phase 5 / US5: enforced in Doctor read-only checks**) | Phase 5 / US1–US5 complete | |
| `components.rs` | `count_components` (unchanged); **NEW**: `list_command_files(plugin_dir) -> Vec<CommandFile>` enumerates `<plugin>/commands/*.md` flat; `CommandFile { path, name }` | Phase 5 / US1 complete | |
| `lifecycle.rs` | `enable_plugin` now calls `list_command_files` and collects `PendingCommand` structs alongside `PendingSkill`; Phase 5 / US3: both are processed through substitution render pipeline; Phase 5 / US4: when_to_use included in embeddings | Phase 5 / US1–US4 complete | |

### `src/doctor/` — Read-Only Extensions (Phase 5 / US5)

| File | Purpose |
|------|---------|
| `checks.rs` | **NEW**: `build_prompts_report(workspace, paths) -> PromptsReport` (reuses PromptRegistry); `count_entries_by_kind(workspace, paths) -> EntryCountsByKind`; `detect_orphan_data_dirs(workspace, paths) -> Vec<OrphanDataDirReport>` — all read-only via open_read_only |
| `report.rs` | **NEW**: `PromptsReport { available: u32, by_kind: { skills: u32, commands: u32 } }`; `EntryCountsByKind { skills: u32, commands: u32, other: u32 }`; `OrphanDataDirReport { path, size, last_modified }` |

### `src/commands/plugin/` — Plugin Show + List (Phase 5 / US5, Polish: validation)

| File | Purpose | Polish |
|------|---------|--------|
| `show.rs` | **~228 lines extended** from US4 baseline: Skills + Commands sections (Kind header), per-entry annotations (`[searchable=true/false]`, `[user_invocable=true/false]`, `[dormant]` when disabled), `EntryView` struct for consistency, human + JSON sync | **NEW**: Calls validate_db_stored_path() on displayed paths (defence-in-depth S-H1) |
| `list.rs` | **~53 lines extended**: Per-kind entry counts in format `plugin: <name> (N skills, M commands)` instead of generic `(N entries)` | |

### `src/paths.rs` — Data Directory Accessors (Phase 5 / US1–US5, Polish: SSOT)

| Method | Returns | Purpose | Status | Polish |
|--------|---------|---------|--------|--------|
| `plugin_data_root()` | `<root>/plugin-data/` | Process-wide plugin-data root (single source of truth per US5) | Phase 5 / US5: new accessor introduced | **NEW SSOT** — replaces two prior inline path computations |
| `plugin_data_dir_for(catalog, plugin)` | `<root>/plugin-data/<catalog>/<plugin>/` | Process-wide plugin scratch space | Path computed; directory created lazily in substitution render | |
| `workspace_data_dir_for(workspace, catalog, plugin)` | `<root>/workspaces/<name>/plugin-data/<catalog>/<plugin>/` | Workspace-scoped plugin scratch space | Path computed; directory created lazily in substitution render | |
| `workspace_dir(workspace)` | `<root>/workspaces/<name>/` | Workspace root (unchanged Phase 4) | Unchanged | |

### `src/workspace/rename.rs` — Workspace Rename + Plugin-Data Relocation (Phase 5 / US2)

| Step | Purpose | Phase 5 / US2 Status |
|------|---------|---------------------|
| 1–5 | Existing rename algorithm (Phase 4 / US2) | Unchanged |
| 6 | **NEW**: Plugin-data relocation within workspace dir rename | **Wired in US2**: Before the final `fs::rename(<old>/, <new>/)`, enumerate and move any existing `<old>/plugin-data/<cat>/<plug>/` subdirectories to the new location |

## Module Boundaries

### Where to Add New Code (Phase 5 / US1–US5 + Polish Updates)

| If you're adding... | Put it in... | Pattern |
|---------------------|--------------|---------|
| New substitution stage | `src/substitution/{stage}.rs` | Add stage handler; extend COMBINED_RE pattern in `regex_sets.rs`; test via SubstitutionContext |
| New built-in variable | `src/substitution/builtins.rs` | Add case to match block in `builtins` handler; wired in appropriate US (US2 for {{TOME_*}}) |
| New argument syntax | `src/substitution/arguments.rs` | Extend `apply_arguments_match` match arms; update `shell_split` quoting rules if needed; test with `coerce_arguments` validation |
| New entry kind | `src/plugin/identity.rs` | Extend `EntryKind` enum; add `from_str()` match arm; update Ser/Deser; backfill migration in v2→v3 |
| Command-specific field | `src/plugin/frontmatter.rs` | Extend `SkillFrontmatter` (lenient parsing); document default; **Phase 5 / US4**: add to when_to_use if it's search-relevant; **Phase 5 / US5**: add to invocability checks if visibility-relevant |
| Command collection | `src/plugin/lifecycle.rs` | Call `list_command_files`; parse frontmatter; build `PendingCommand`; Phase 5 / US3: collect arguments schema from frontmatter |
| MCP prompt handler | `src/mcp/prompts.rs` | Register route via `PromptRouter::new_dyn`; implement request handler |
| Prompt name edge case | `src/mcp/prompt_name.rs` | Extend `sanitise` / `sanitise_trunc` logic; test Unicode boundaries |
| Prompt collision policy | `src/mcp/prompt_collision.rs` | Extend `resolve_collisions` detection; update warning message |
| MCP discovery tier | `src/mcp/tools/{search_skills,get_skill_info,get_skill}.rs` | **Phase 5 / US4 patterns**: search_skills truncates descriptions (char_indices), get_skill_info walks resources (5-cap + sentinel), get_skill returns complete body |
| Resource enumeration | `src/mcp/tools/get_skill_info.rs` | Extend `walk_resources()` logic; maintain 5-cap + sentinel; use BTreeMap for sorted JSON; skip symlinks at every level |
| Description truncation | `src/mcp/tools/search_skills.rs` + `get_skill_info.rs` | Extend `truncate_description()` if new truncation rules needed; verify char_indices fast-path still applies; keep both sites synchronized (Polish pattern) |
| Entry body resolution | `src/index/skills.rs` | Update `resolve_entry_body_path` match arms per new kind; call `validate_db_stored_path()` for boundary checks (Polish SSOT) |
| Path boundary validation | `src/index/skills.rs` | Call canonical `validate_db_stored_path()` SSOT at every read/write point requiring path safety (Polish pattern) |
| Schema backfill | `src/index/migrations.rs` | Add new v2→v3 backfill step; test via synthetic DB; include when_to_use if indexing new fields |
| Data-dir path accessor | `src/paths.rs` | Add new `*_data_dir_for(...)` method; coordinate with `plugin_data_root()` SSOT; update lazy-creation in `data_dir.rs` |
| Data-dir creation | `src/substitution/data_dir.rs` | Add new `ensure_*_data(...)` function; return `SubstitutionError` on failure; call from appropriate substitution stage |
| Shared MCP context | `src/mcp/substitution_helpers.rs` | Add helper function to `build_context_for_entry()` SSOT module (Polish pattern for cross-handler reuse); consume from both prompts/get + get_skill_info |
| Workspace-related mutation | `src/workspace/rename.rs` / `remove.rs` / `init.rs` | Update step sequence; ensure data-dir side effects coordinate (Phase 5 / US2: relocate on rename) |
| Doctor read-only check | `src/doctor/checks.rs` | Add new `pub fn check_*` helper; call via `open_read_only`; add variant to `report.rs` if new subsystem needed |
| Doctor report field | `src/doctor/report.rs` | Add field to `DoctorReport`; add Ser/Deser if reporting new subsystem; no mutation allowed (read-only invariant) |
| Invocability visibility | `src/commands/plugin/{show,list}.rs` | Consult `user_invocable` field + grouping by `EntryKind`; doctor checks enforce visibility constraints |
| New harness | `src/harness/{name}.rs` + register in `mod.rs` | Impl `HarnessModule` trait (7 methods) |
| New workspace command | `src/commands/workspace/{cmd}.rs` | Pattern: `run(args, scope, paths, mode)` + `assemble_*` |
| Surgical TOML edit | `src/settings/edit.rs` | Add helper using `toml_edit::DocumentMut` |
| New subsystem (doctor) | `src/doctor/report.rs` | Add variant to `Subsystem` enum + Ser/Deser impl + fix handler to `fixes.rs` |

### Key Patterns

#### Single-Pass Substitution Pattern (Phase 5 / US2, Polish: unified dispatch)

```rust
// src/substitution/mod.rs — COMBINED_RE single-pass loop (replaces dual-sweep)

pub fn render(body: &str, context: &SubstitutionContext) -> Result<String, SubstitutionError> {
    let combined_re = regex_sets::COMBINED_RE.get_or_init(|| {
        // Compile union of all stage patterns: {{TOME_*}} | {{$*}} | $ARGUMENTS | $N | $name
        // Pattern union ensures each placeholder is matched exactly once
    });

    let mut result = String::new();
    let mut last_end = 0;

    for capture in combined_re.captures_iter(body) {
        let matched_text = capture.get(0).unwrap();
        let start = matched_text.start();
        let end = matched_text.end();

        // Push unmatched prefix
        result.push_str(&body[last_end..start]);

        // Classify match and dispatch to appropriate stage handler
        if BUILTINS_RE.is_match(matched_text.as_str()) {
            let replacement = builtins::resolve(matched_text.as_str(), context)?;
            result.push_str(&replacement);
        } else if ENV_RE.is_match(matched_text.as_str()) {
            let replacement = env::resolve(matched_text.as_str(), context)?;
            result.push_str(&replacement);
        } else if ARGUMENTS_RE.is_match(matched_text.as_str()) {
            let replacement = arguments::resolve(matched_text.as_str(), context)?;
            result.push_str(&replacement);
        }

        last_end = end;
    }

    // Push remaining suffix
    result.push_str(&body[last_end..]);

    // Tail: append ARGUMENTS footer if body references bare $ARGUMENTS
    if body_has_bare_arguments(body) && !context.arguments.positional.is_empty() {
        result.push_str(" -- ");
        result.push_str(&context.arguments.positional.join(" "));
    }

    Ok(result)
}
```

#### Argument Substitution Pipeline (Phase 5 / US3)

```rust
// src/substitution/arguments.rs — Three-stage argument processing

// Stage 1: Parse shell quoting
pub fn shell_split(input: &str) -> Vec<String> {
    // POSIX shell quoting parser: respects single/double quotes, backslash escape
    // Returns all tokens including empty strings from consecutive separators
}

// Stage 2: Validate against declared schema
pub fn coerce_arguments(
    supplied: Vec<String>,
    declared: &[PromptArgument],
) -> Result<ArgumentValues, SubstitutionError> {
    // Match supplied args to declared positional + named parameters
    // Returns ArgumentValues { positional: Vec<String>, named: HashMap<String, String> }
    // Errors on: count mismatch, unknown named args, duplicates
}

// Stage 3: Apply to matched placeholder
pub fn apply_arguments_match(pattern: &str, values: &ArgumentValues) -> String {
    // Resolve $1, $2, ..., $name, $ARGUMENTS to their values
    // Returns empty string for missing optional arguments per Claude Code spec
}
```

#### Lazy Data-Dir Creation Pattern (Phase 5 / US2)

```rust
// src/substitution/builtins.rs — Triggered on first {{TOME_*}} match

pub(super) fn resolve(placeholder: &str, context: &SubstitutionContext) -> Result<String, SubstitutionError> {
    match placeholder {
        "{{TOME_PLUGIN_DATA}}" => {
            // First call to ensure_plugin_data per render pass creates the dir
            let path = data_dir::ensure_plugin_data(
                &context.paths,
                &context.catalog,
                &context.plugin,
            )?;
            Ok(path.to_string_lossy().to_string())
        }
        // ... other built-ins
    }
}
```

#### Workspace Rename Plugin-Data Relocation Pattern (Phase 5 / US2)

```rust
// src/workspace/rename.rs — Step 6 integrated into workspace dir rename

// Steps 1-5: existing rename algorithm (Phase 4 / US2)
// ...

// Step 6: Relocate plugin-data directories (NEW in US2)
let plugin_data_subdir = old_workspace_dir.join("plugin-data");
if plugin_data_subdir.exists() {
    for entry in std::fs::read_dir(&plugin_data_subdir)? {
        // Move each catalog/plugin/ subdirectory to new location
    }
}

// Final atomic rename of workspace dir tree (includes relocated plugin-data)
std::fs::rename(&old_workspace_dir, &new_workspace_dir)?;
```

#### Description Truncation Fast-Path Pattern (Phase 5 / US4, Polish: unified)

```rust
// src/mcp/tools/search_skills.rs::truncate_description — O(1) fast-path, O(k) truncation
// (SAME pattern mirrored at get_skill_info.rs for consistency)

fn truncate_description(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let mut iter = s.char_indices();
    // Walk past `max` chars; if we exhaust the iterator within those,
    // no truncation needed (input already fit).
    for _ in 0..max {
        if iter.next().is_none() {
            return s.to_owned();  // Fast path: input fits, O(n) worst but O(1) avg when no truncation
        }
    }
    // If the (max+1)-th char exists, slice at its byte offset and
    // append the ellipsis. Otherwise the input was exactly `max` chars
    // — no truncation needed.
    match iter.next() {
        None => s.to_owned(),
        Some((byte_idx, _)) => {
            let mut out = String::with_capacity(byte_idx + '\u{2026}'.len_utf8());
            out.push_str(&s[..byte_idx]);
            out.push('\u{2026}');
            out
        }
    }
}
```

#### Shared MCP Context Builder Pattern (Phase 5 Polish, SSOT)

```rust
// src/mcp/substitution_helpers.rs — Shared SSOT across prompts/get + get_skill_info

pub fn build_context_for_entry(
    catalog: &str,
    plugin: &str,
    entry_name: &str,
    kind: EntryKind,
    scope: &Scope,
    paths: &Paths,
) -> Result<SubstitutionContext, TomeError> {
    // Lookup plugin version
    let plugin_version = get_plugin_version(catalog, plugin, paths)?;
    
    // Resolve entry body path (uses kind discriminator)
    let body_path = index::skills::resolve_entry_body_path(catalog, plugin, entry_name, kind);
    
    // Read full body
    let body = fs::read_to_string(&body_path)?;
    
    // Parse frontmatter for arguments + when_to_use
    let frontmatter = plugin::frontmatter::parse_skill_frontmatter(&body)?;
    
    // Build context with all fields populated
    SubstitutionContext::builder()
        .workspace_name(scope.0.clone())
        .catalog(catalog.to_owned())
        .plugin(plugin.to_owned())
        .entry_name(entry_name.to_owned())
        .plugin_version(plugin_version)
        .arguments(coerce_frontmatter_arguments(&frontmatter)?)
        .build()
}
```

#### Path Boundary Validation Pattern (Phase 5 Polish, SSOT)

```rust
// src/index/skills.rs::validate_db_stored_path — Canonical check at every boundary

pub(crate) fn validate_db_stored_path(path: &Path) -> Result<(), TomeError> {
    // Reject .. components and absolute paths
    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(TomeError::PathTraversalAttempt { path: path.to_owned() });
        }
        if matches!(component, std::path::Component::RootDir) {
            return Err(TomeError::PathTraversalAttempt { path: path.to_owned() });
        }
    }
    Ok(())
}

// Called by every read/write site:
// - get_skill::resolve_entry_body_path (S-H1 boundary)
// - commands/plugin/show.rs::list_entries (displayed paths, defence-in-depth)
```

#### Three-Tier MCP Discovery Pattern (Phase 5 / US4, Polish: shared context)

```rust
// src/mcp/tools/{search_skills,get_skill_info,get_skill}.rs

// Tier 1: search_skills — ranked list with truncated descriptions
pub async fn handle_search(state, input) -> Result<SearchResults, Error> {
    // KNN + rerank → top 5–10 with truncate_description(desc, 512)
}

// Tier 2: get_skill_info — full metadata + resource enumeration (Polish: shared context)
pub async fn handle_info(state, input) -> Result<SkillInfo, Error> {
    // Build context via build_context_for_entry() SSOT (shared with prompts/get)
    // Read full description (no truncation) + when_to_use
    // For skills: walk parent dir (1-level deep) with 5-cap per dir
    // resources: { files: [...], directories: { "name": [...], ... } }
}

// Tier 3: get_skill — complete body (unchanged from Phase 1)
pub async fn handle_get(state, input) -> Result<SkillBody, Error> {
    // Lookup entry → read full body markdown + all components
}
```

#### Resource Enumeration Pattern (Phase 5 / US4)

```rust
// src/mcp/tools/get_skill_info.rs::walk_resources — BTreeMap for alphabetical JSON

fn walk_resources(body_path: &Path) -> io::Result<ResourceEnumeration> {
    let parent = body_path.parent()?;

    // Collect + sort top-level files and subdirs
    let mut files: Vec<PathBuf> = Vec::new();
    let mut subdirs: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(parent)? {
        let path = entry?.path();
        let ft = entry?.file_type()?;
        if ft.is_symlink() { continue; }  // Skip symlinks
        if ft.is_dir() { subdirs.push(path); }
        else if ft.is_file() && path != body_path { files.push(path); }
    }

    files.sort_by(|a, b| basename_cmp(a, b));
    subdirs.sort_by(|a, b| basename_cmp(a, b));

    let files_out = clip_and_sentinel(files.iter().map(|p| p.display().to_string()).collect());

    // BTreeMap guarantees alphabetical key order in JSON serialisation
    let mut directories: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for sub in subdirs {
        let name = sub.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let children = list_dir_children(&sub)?;
        directories.insert(name, children);
    }

    Ok(ResourceEnumeration { files: files_out, directories })
}

// Apply 5-cap + "and N more" sentinel rule
fn clip_and_sentinel(items: Vec<String>) -> Vec<String> {
    const PER_DIRECTORY_CAP: usize = 5;
    if items.len() <= PER_DIRECTORY_CAP {
        return items;
    }
    let omitted = items.len() - PER_DIRECTORY_CAP;
    let mut out: Vec<String> = items.into_iter().take(PER_DIRECTORY_CAP).collect();
    out.push(format!("and {omitted} more"));
    out
}
```

#### Doctor Read-Only Extensions Pattern (Phase 5 / US5)

```rust
// src/doctor/checks.rs — All read-only, never opening transactions

pub fn build_prompts_report(workspace: &Scope, paths: &Paths) -> Result<PromptsReport, TomeError> {
    // Reuse mcp::prompts::PromptRegistry::build_for_workspace (no duplication)
    // Call via open_read_only; never take advisory lock
}

pub fn count_entries_by_kind(workspace: &Scope, paths: &Paths) -> Result<EntryCountsByKind, TomeError> {
    // Query enabled entries grouped by skills.kind column
    // Call via open_read_only; never take advisory lock
}

pub fn detect_orphan_data_dirs(workspace: &Scope, paths: &Paths) -> Result<Vec<OrphanDataDirReport>, TomeError> {
    // Walk <root>/plugin-data/ and workspace-scoped dirs
    // Report entries whose skill is not in the index
    // Call via open_read_only; never take advisory lock
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

*This document shows WHERE code lives. Updated 2026-05-27 against Phase 5 COMPLETE + Polish shipped (per-entry invocability + doctor read-only extensions + single-source-of-truth promotion). 1193 tests across 151 suites.*

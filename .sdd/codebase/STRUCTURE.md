# Project Structure

> **Purpose**: Document directory layout, module boundaries, and where to add new code.
> **Generated**: 2026-05-29
> **Last Updated**: 2026-05-29 (Phase 6 Foundational; harness/agents.rs new module; harness trait extended with 7 methods; EntryKind::Agent variant; schema v3→v4 marker migration)

## Directory Layout

```
tome/
├── src/                                # Rust library + binary source
│   ├── main.rs                         # CLI entry: scope resolution, command dispatch, error mapping
│   ├── lib.rs                          # Public exports
│   ├── cli.rs                          # clap derive defs (all commands + global flags)
│   ├── error.rs                        # Closed TomeError enum (30+ variants → exit codes; Phase 6: +4 new codes 43–46)
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
│   │   ├── frontmatter.rs              # SKILL.md + command + agent YAML frontmatter parser (Phase 5: commands widened; Phase 6: agent fields reserved)
│   │   ├── identity.rs                 # PluginId + Phase 5 NEW: EntryKind enum (Skill | Command | Agent) + canonical from_str(); Phase 6: Agent variant + exhaustive match widening
│   │   ├── components.rs               # Walk skill/command dirs; Phase 5: list_command_files enumerates commands; Phase 6: agent enumeration skeleton
│   │   └── lifecycle.rs                # enable/disable/reindex orchestration (Phase 5: commands + skills; Phase 6: agents skeleton)
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
│   │   ├── schema.rs                   # CREATE TABLE statements + bootstrap (schema v4: Phase 6 Foundational marker; v3: Phase 5 addition)
│   │   ├── migrations.rs               # Forward-only schema migrations + framework; Phase 5: v2→v3 migration; Phase 6: v3→v4 marker (no DDL, advances SCHEMA_VERSION only)
│   │   ├── vec_ext.rs                  # sqlite-vec extension loader
│   │   ├── skills.rs                   # Phase 5: CRUD over unified skills table with EntryKind discriminator; resolve_entry_body_path + validate_db_stored_path SSOT (Polish); Phase 6: agent rows (searchable=false, user_invocable=false per FR-070a)
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
│   ├── doctor/                         # Diagnostic + auto-repair (Phase 3 US4 + Phase 4 US5 + Phase 5 US5 + Phase 6 skeleton)
│   │   ├── mod.rs                      # assemble_report + re_assemble entry
│   │   ├── checks.rs                   # check_catalogs, check_index, check_drift, check_workspace_registry + Phase 5 / US5: build_prompts_report, count_entries_by_kind, detect_orphan_data_dirs (all read-only); Phase 6: agent/hook/guardrails diagnostics skeleton
│   │   ├── harness_detect.rs           # Probe ~/.claude/, ~/.codex/, ~/.cursor/, ~/.gemini/, ~/.opencode/
│   │   ├── report.rs                   # DoctorReport + Subsystem (typed 11-variant enum) + SubsystemHealth + Phase 5 / US5: PromptsReport, EntryCountsByKind, OrphanDataDirReport; Phase 6: agent/hook/guardrails report fields reserved
│   │   ├── fixes.rs                    # apply + apply_one (subsystem routing) + per-subsystem repair handlers
│   │   ├── binding.rs                  # Phase 4 US5: check_binding (T366) — marker well-formedness + RULES.md drift
│   │   ├── harness_integration.rs      # Phase 4 US5: check_harness_integration (T367) — per-harness rules/mcp checks
│   │   └── orphan_cleanup.rs           # Phase 4 US5: cleanup_stale_staging_dirs (FR-410) — 1-hour age gate
│   │
│   ├── harness/                        # Phase 4+: Per-harness trait + sync orchestrator + composition; Phase 6: trait extension for hooks/guardrails/agents
│   │   ├── mod.rs                      # HarnessModule trait (Phase 4: 8 methods; Phase 6 Foundational: +7 new methods all safe-by-default); SUPPORTED_HARNESSES registry; shape enums (HooksStrategy, GuardrailsPlacement, GuardrailsTarget, AgentFormat)
│   │   ├── agents.rs                   # **Phase 6 Foundational NEW** Agent type definitions: CanonicalAgent (source form) + TranslatedAgent (per-harness result); skeleton for US1 parsing/translation
│   │   ├── claude_code.rs              # Claude Code harness impl
│   │   ├── codex.rs                    # Codex harness impl
│   │   ├── cursor.rs                   # Cursor harness impl
│   │   ├── gemini.rs                   # Gemini CLI harness impl
│   │   ├── opencode.rs                 # OpenCode harness impl
│   │   ├── rules_file.rs               # Block-in-file + standalone strategies + atomic_write
│   │   ├── mcp_config.rs               # JSON + TOML MCP config read/write primitives
│   │   ├── sync.rs                     # Phase 4: Sync orchestrator (per-project harness writes)
│   │   └── stub.rs                     # StubHarnessModule for test injection; Phase 6: extended with agent/hook method overrides for testing
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
│   │   │   ├── enable.rs               # `tome plugin enable <id>` + trigger regenerate (Phase 5: commands + skills; Phase 6: agents skeleton)
│   │   │   ├── disable.rs              # `tome plugin disable <id> [--force]` + trigger regenerate
│   │   │   ├── list.rs                 # `tome plugin list` (Phase 5 / US5: per-kind entry counts; Phase 6: +agent count)
│   │   │   ├── show.rs                 # `tome plugin show <id>` (Phase 5 / US5: ~228 lines extended for searchable/invocable annotations + kind grouping; Phase 6: +Agents section)
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
│   │   ├── doctor.rs                   # `tome doctor [--fix] [--verify] [--force]` (Phase 5 / US5: renders extended report with prompts + entry-kind counts + orphan data-dirs; Phase 6: agent/hook/guardrails diagnostics skeleton)
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
│       ├── substitution_helpers.rs     # Phase 5 Polish NEW: build_context_for_entry() SSOT (shared across prompts/get + get_skill_info)
│       ├── tool_description.rs         # Phase 4 US4.b: Compose runtime tool description from cached summary
│       ├── prompt_name.rs              # Phase 5 NEW: Prompt-name derivation (<plugin>__<entry> sanitisation + truncation)
│       ├── prompt_collision.rs         # Phase 5 NEW: Collision detection when entries map to same prompt name
│       ├── prompts.rs                  # Phase 5 NEW: MCP prompts capability (PromptRegistry, PromptRouter hand-rolled); skills/commands only (agents excluded per FR-070a)
│       └── tools/                      # MCP tool handlers (Phase 5 / US4–US5: three-tier discovery + read-only extensions; Phase 6: agent rows in schema, not yet exposed)
│           ├── mod.rs                  # Tool registration
│           ├── search_skills.rs        # search_skills tool (KNN+rerank, workspace-filtered, 4096-char input cap, Phase 5 / US4: when_to_use in results, truncate_description hardening; Polish: mirrors truncation at get_skill_info; Phase 6: agent rows searchable=false, excluded from results per FR-070a)
│           ├── get_skill_info.rs       # Phase 5 / US4 NEW: get_skill_info middle-tier tool (full description + when_to_use + 5-cap resource enumeration; Polish: uses build_context_for_entry SSOT; Phase 6: agent rows excluded per FR-070a)
│           └── get_skill.rs            # get_skill tool (metadata + components); Phase 6: agent rows excluded from prompts, MCP discovery per FR-070a
│
├── tests/                              # Integration tests (access library as external crate)
│   ├── catalog_*.rs                    # Catalog add/remove/update tests
│   ├── plugin_*.rs                     # Plugin enable/disable/list/show/interactive (Phase 5: commands coverage + US5 annotations; Phase 6: agent entry-kind tests)
│   ├── models_*.rs                     # Model download/list/remove
│   ├── query.rs                        # Query + strict mode + rerank
│   ├── reindex.rs                      # Reindex all/per-catalog/per-plugin
│   ├── status.rs                       # Status command + health checks
│   ├── workspace_*.rs                  # Workspace info/init/binding/sync/list/rename/remove tests (US1–US2)
│   ├── harness_*.rs                    # Phase 4 US3: Harness list/use/remove/info/sync/composition tests; Phase 6: harness_trait_p6.rs for trait extension
│   ├── summariser_*.rs                 # Phase 4 US4: Summariser triggers, forward progress, cache, registry tests
│   ├── doctor*.rs                      # Phase 4 US5: Doctor assembly + fixes + binding + harness integration + orphan cleanup; Phase 5 / US5: prompts report + entry counts + orphan data-dirs; Phase 6: doctor_* tests extended for agent/hook/guardrails
│   ├── mcp_*.rs                        # MCP server lifecycle + tools + log rotation + tool description (US4.b) + prompts (US1.b) + Phase 5 / US4–US5: get_skill_info tests + read-only extensions; Phase 6: agent exclusion tests
│   ├── substitution_*.rs               # Phase 5: Substitution engine tests (skeleton, builtins, env, arguments, data-dir, e2e)
│   ├── entry_kind_agent_indexing.rs    # **Phase 6 Foundational NEW** Agent entry-kind indexing + schema widening tests
│   ├── harness_trait_p6.rs             # **Phase 6 Foundational NEW** HarnessModule trait extension (7 new methods, safe-by-default impls, exhaustive match widening)
│   ├── schema_migration_p6.rs          # **Phase 6 Foundational NEW** Schema v3→v4 marker migration (no DDL, version advance only)
│   ├── entry_e2e.rs                    # Phase 5 / US3 NEW: Full enable → search → get → prompts pipeline with argument substitution + Phase 5 / US5: invocability visibility; Phase 6: agent rows excluded
│   ├── exit_codes.rs                   # Exit code matrix validation; Phase 6: +4 new codes (43–46)
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
│       ├── ARCHITECTURE.md             # System design + patterns (Phase 5 / US5: per-entry invocability + doctor read-only extensions; Polish: single-source-of-truth promotion; Phase 6 Foundational: harness trait extension)
│       ├── STRUCTURE.md                # Directory layout (this file; Phase 6 Foundational: harness/agents.rs + trait extension)
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
│   ├── 005-phase-5-commands-prompts/        # Phase 5 (F1–F3 + US1–US5 shipped + Polish complete)
│   │   ├── spec.md
│   │   ├── plan.md
│   │   ├── research.md (20 R-decisions)
│   │   ├── data-model.md (schema v3, EntryKind, SubstitutionContext, ArgumentValues, PromptRegistry, ResourceEnumeration, PromptsReport, EntryCountsByKind, OrphanDataDirReport)
│   │   ├── contracts/ (9+ contracts: exit-codes-p5, schema-migration-p5, entry-schema-p5, substitution-engine, mcp-tools-p5, mcp-prompts, etc.)
│   │   ├── notes/ (Phase 5 research notes: rmcp-prompts-api, argument-coercion, three-tier discovery, when-to-use-indexing)
│   │   ├── review/ (Phase 5 reviewer findings + disposition per US)
│   │   ├── retro/ (P3–P8 retrospectives)
│   │   └── quickstart.md
│   └── 006-phase-6-hooks-agents/           # Phase 6 (Foundational wired; spec + plan + contracts complete)
│       ├── spec.md
│       ├── plan.md
│       ├── research.md (20 R-decisions)
│       ├── data-model.md (v4 schema marker, HooksStrategy, GuardrailsTarget, AgentFormat, CanonicalAgent, TranslatedAgent, etc.)
│       ├── contracts/ (9 contracts: exit-codes-p6, schema-migration-p6, entry-schema-p6, harness-modules-p6, hooks-reconciliation, guardrails-prose, agent-translation, etc.)
│       ├── retro/ (P2 Foundational retrospective)
│       └── quickstart.md
│
├── PRDs/                               # Product requirement documents
│   ├── phase-1.md
│   ├── phase-2.md
│   ├── phase-3.md
│   ├── phase-4.md
│   ├── phase-5.md
│   └── phase-6.md
│
├── Cargo.toml                          # Package definition (MSRV 1.93, v0.5.0)
├── Cargo.lock                          # Dependency lock
├── build.rs                            # sqlite-vec C extension compilation
├── CONSTITUTION.md                     # v1.3.0 — constraints + trade-offs (Phase 4 §Paths amendment; no Phase 5 amendments; no Phase 6 amendments)
├── CLAUDE.md                           # Project context for Claude Code (Phase 5 complete + Polish shipped; v0.5.0 final; Phase 6 planning → Foundational)
└── CHANGELOG.md                        # Version history (v0.1.0–v0.5.0 shipped; v0.6.0 in development)
```

## Key Directories

### `src/` — Source Code

| Directory | Purpose | Key Files |
|-----------|---------|-----------|
| `substitution/` | Phase 5 / US1–US3: Variable rendering engine (COMPLETE single-pass pipeline) | `mod.rs` (render loop + body_has_bare_arguments), `context.rs`, `builtins.rs`, `env.rs`, `arguments.rs` (shell_split + coerce_arguments + apply_arguments_match), `data_dir.rs`, `regex_sets.rs` (COMBINED_RE) |
| `plugin/` | Plugin metadata, lifecycle (Phase 5: commands + arguments + when_to_use + user_invocable; Phase 6: agent fields reserved) | `manifest.rs`, `frontmatter.rs`, `identity.rs` (EntryKind + Agent variant + canonical from_str), `components.rs` (list_command_files, agent enumeration skeleton), `lifecycle.rs` |
| `index/` | SQLite + sqlite-vec index (Phase 5: v3 schema with when_to_use; Polish: validate_db_stored_path SSOT; Phase 6: v4 marker migration, agent rows with searchable=false/user_invocable=false) | `db.rs`, `schema.rs` (v4 marker), `migrations.rs` (v3→v4), `skills.rs` (EntryKind + Agent + validate_db_stored_path), `query.rs` (Phase 5 / US4: when_to_use embeddings) |
| `mcp/` | MCP server + Phase 5 prompts + three-tier discovery + read-only extensions + Polish: substitution_helpers; Phase 6: agent rows excluded per FR-070a | `prompts.rs` (PromptRegistry, skills/commands only), `prompt_name.rs`, `prompt_collision.rs`, `substitution_helpers.rs` (build_context_for_entry SSOT), `tools/` (search_skills, get_skill_info, get_skill) |
| `doctor/` | Health check + auto-repair (Phase 5 / US5: read-only extensions; Phase 6: agent/hook/guardrails diagnostics skeleton) | `checks.rs` (build_prompts_report, count_entries_by_kind, detect_orphan_data_dirs; Phase 6: agent/hook checks skeleton), `report.rs` (PromptsReport, EntryCountsByKind, OrphanDataDirReport; Phase 6: agent/hook fields reserved) |
| `catalog/` | Catalog registry, git ops | `manifest.rs`, `store.rs`, `git.rs` |
| `embedding/` | Text embedding + reranking | `fastembed.rs`, `stub.rs`, `download.rs` |
| `workspace/` | Scope resolution, binding, lifecycle (Phase 5 / US2: rename relocation) | `scope.rs`, `binding.rs`, `init.rs`, `rename.rs`, `remove.rs`, `regen_summary.rs` |
| `harness/` | Phase 4: Harness abstraction + sync; Phase 6 Foundational: trait extension for hooks/guardrails/agents | `mod.rs` (trait +7 new methods safe-by-default; shape enums), `agents.rs` (CanonicalAgent + TranslatedAgent skeleton), 5 harness impls, `sync.rs`, `rules_file.rs`, `mcp_config.rs`, `stub.rs` (extended with agent/hook test overrides) |
| `settings/` | Phase 4: Layered composition | `parser.rs`, `resolver.rs` (composition engine), `edit.rs` |
| `summarise/` | Phase 4: Workspace summariser | `llama.rs`, `stub.rs`, `prompts.rs`, `trigger.rs`, `registry.rs` |
| `commands/` | CLI subcommand entry points (Phase 5 / US5: show + list extended; Phase 6: plugin list/show extended with agent entries) | Per-command modules + dispatchers |
| `presentation/` | Output formatting + TUI | `tables.rs`, `prompt.rs`, `colour.rs` |
| `util/` | Shared utilities | `atomic_dir.rs` (tempfile + rename), `io.rs` (bounded read) |
| `paths.rs` | Phase 4 single-root layout; Phase 5: data-dir accessors; Polish: plugin_data_root() SSOT | `home_root()`, `Paths struct`, `plugin_data_root()` SSOT, `plugin_data_dir_for()`, `workspace_data_dir_for()` |

### `src/harness/` — Harness Trait + Agent Types (Phase 6 Foundational)

| File | Purpose | Phase 6 Foundational |
|------|---------|---------------------|
| `mod.rs` | `HarnessModule` trait (Phase 4: 8 methods; Phase 6: +7 new methods all safe-by-default); shape enums (HooksStrategy, GuardrailsPlacement, GuardrailsTarget, AgentFormat) | **NEW**: `hooks_strategy()`, `hook_settings_path()`, `guardrails_target()`, `supports_native_agents()`, `agent_dir()`, `agent_format()`, `translate_agent()` with safe-by-default impls (no production override until US1–US3) |
| `agents.rs` | **NEW**: Agent type definitions for the harness translation pipeline (skeleton) | `CanonicalAgent` (name, description, body, model, tools/disallowed_tools, privileged hooks/mcp_servers/permission_mode) + `TranslatedAgent` (dir, filename, displayed_name, format, rendered, dropped_fields); parsing + translation behaviour deferred to US1 T034 |
| `claude_code.rs` → `opencode.rs` | Five concrete harness implementations | Inherit safe-by-default trait impls; production overrides land in US1–US3 |
| `stub.rs` | Test-injection harness | Extended with agent/hook method overrides for integration test coverage |

### `src/plugin/` — Commands & Entries (Phase 5 extended, Phase 6 Agent variant)

| File | Purpose | Phase 6 Foundational |
|------|---------|---------------------|
| `identity.rs` | `PluginId` (unchanged); `EntryKind` enum (`Skill` \| `Command` \| `Agent`) with `as_str()` accessor + canonical `from_str()` | **NEW**: `Agent` variant; all exhaustive matches widened in lockstep (FR-070a); no catch-all |
| `frontmatter.rs` | `SkillFrontmatter` (Phase 5: commands widened with arguments/when_to_use/user_invocable) | **RESERVED**: Agent frontmatter fields (name, description, model, tools/disallowed_tools, hooks, mcp_servers, permission_mode) — parsing wired in US1 |
| `components.rs` | `count_components` (unchanged); `list_command_files` (Phase 5) | **SKELETON**: `list_agent_files` enumeration (wired in US1) |
| `lifecycle.rs` | `enable_plugin` (Phase 5: commands + skills) | **SKELETON**: Agent collection (`collect_pending_agents`) wired in US1 |

### `src/index/` — Schema & Entry Records (Phase 5 + Phase 6 Foundational)

| File | Purpose | Phase 6 Foundational |
|------|---------|---------------------|
| `schema.rs` | DDL + bootstrap (v4 marker) | **NEW**: v4 schema version constant (no DDL changes; agent rows use existing kind column) |
| `migrations.rs` | Forward-only framework; v2→v3 (Phase 5) | **NEW**: v3→v4 marker migration (`apply()` advances SCHEMA_VERSION only, no backfill; agent indexing in US1) |
| `skills.rs` | `SkillRecord` (Phase 5: kind + when_to_use + searchable + user_invocable) | **WIDENED**: Agent rows always `searchable=false`, `user_invocable=false` per FR-070a (enforced in query filters at MCP discovery time) |

### `src/commands/plugin/` — Plugin List & Show (Phase 6 Foundational)

| File | Purpose | Phase 6 Foundational |
|------|---------|---------------------|
| `list.rs` | Entry count format (Phase 5: "N skills, M commands") | **EXTENDED**: "N skills, M commands, P agents" |
| `show.rs` | Kind-grouped display (Phase 5: Skills + Commands sections) | **EXTENDED**: Agents section added; per-agent annotations (searchable=false, user_invocable=false, dormant) |

### `src/doctor/` — Diagnostics (Phase 5 read-only + Phase 6 skeleton)

| File | Purpose | Phase 6 Foundational |
|------|---------|---------------------|
| `checks.rs` | Phase 5 read-only helpers (build_prompts_report, count_entries_by_kind, detect_orphan_data_dirs) | **SKELETON**: Agent/hook/guardrails diagnostics (checks wired in US5) |
| `report.rs` | Phase 5 report types (PromptsReport, EntryCountsByKind, OrphanDataDirReport) | **RESERVED**: Agent/hook/guardrails report fields (struct shapes TBD in US5) |

### `src/error.rs` — Error Variants (Phase 6 Foundational)

| Error | Code | Purpose |
|-------|------|---------|
| `HookSpecParseError` | 43 | Malformed or unparsable `hooks.json` (US1) |
| `HookSettingsWriteFailed` | 44 | Write failure to `.claude/settings.local.json` during hook merge (US1) |
| `AgentTranslationFailed` | 45 | Agent translation failed; CanonicalAgent → TranslatedAgent pipeline error (US1 T034) |
| `GuardrailsWriteFailed` | 46 | Guardrails prose write failure (US2) |

## Module Boundaries

### Where to Add New Code (Phase 6 Foundational)

| If you're adding... | Put it in... | Pattern |
|---------------------|--------------|---------|
| New harness | `src/harness/{name}.rs` + register in `mod.rs` | Impl `HarnessModule` trait (8 methods Phase 4 + 7 new Phase 6, all methods override-able for per-harness specifics) |
| Agent parsing | `src/plugin/frontmatter.rs` | Extend `SkillFrontmatter` parsing for agent-specific fields; Phase 6: Foundational reserves shape, US1 wires parsing logic |
| Agent enumeration | `src/plugin/components.rs` | Add `list_agent_files(plugin_dir) -> Vec<AgentFile>` (wired in US1); mirrors `list_command_files` pattern |
| Agent lifecycle | `src/plugin/lifecycle.rs` | Add `collect_pending_agents(...)` alongside `collect_pending_commands` (wired in US1); parse frontmatter, build agent records |
| Agent type | `src/harness/agents.rs` | Both types (CanonicalAgent, TranslatedAgent) are defined; add translation logic per-harness via `HarnessModule::translate_agent` override (US1–US3) |
| Harness agent override | `src/harness/{name}.rs` | Override `supports_native_agents()`, `agent_dir()`, `agent_format()`, `translate_agent()` methods (wired in US1–US3) |
| Entry-kind exhaustive match | `src/commands/plugin/{mod,list,show}.rs`, `src/doctor/{checks,report}.rs`, `src/plugin/frontmatter.rs` | Extend all match arms to handle `EntryKind::Agent`; never use catch-all; test via `entry_kind_agent_indexing.rs` |
| Agent doctor check | `src/doctor/checks.rs` | Add check function (wired in US5); call via `open_read_only`; add variant to `report.rs` |
| Agent visibility | `src/commands/plugin/{show,list}.rs` | Consult agent rows from index; filter per `searchable` / `user_invocable` flags (FR-070a: both always false for agents) |
| Hook reconciliation | `src/harness/mod.rs::HookSettingsPath` + per-harness impl | Override `hooks_strategy()` / `hook_settings_path()` to return `RealJson`; US1 wires merge logic |
| Guardrails prose | `src/harness/mod.rs::GuardrailsTarget` + per-harness impl | Override `guardrails_target()` to return placement + suppression rules; US2 wires prose rendering + write |
| New substitution stage | `src/substitution/{stage}.rs` | Add stage handler; extend COMBINED_RE pattern in `regex_sets.rs`; test via SubstitutionContext (Phase 5 pattern) |
| Schema change | `src/index/{schema,migrations}.rs` | Advance SCHEMA_VERSION; add v4→v5 migration with forward-only logic; backfill strategy deferred to the consuming user story |
| Exit code | `src/error.rs` + `tests/exit_codes.rs` | Add variant; compiler enforces `exit_codes.rs` update via exhaustive match; Phase 6 cluster is 43–46 |

### Key Patterns

#### Harness Trait Safe-by-Default Pattern (Phase 6 Foundational)

```rust
// src/harness/mod.rs — New methods all provide safe defaults

pub trait HarnessModule: Send + Sync {
    // Existing 8 methods (Phase 4) — unchanged
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn detect(&self, home: &Path) -> bool;
    // ... etc

    // Phase 6 Foundational — New 7 methods with safe-by-default impls
    // Every harness inherits these defaults; production overrides land in US1–US3

    /// How Tome reconciles plugin hooks for this harness (FR-001, FR-013).
    /// Default: no native hook support (prose guardrails fallback only).
    fn hooks_strategy(&self) -> HooksStrategy {
        HooksStrategy::GuardrailsOnly
    }

    /// Machine-local settings file the `RealJson` strategy merges hooks
    /// into (FR-002). `None` for every `GuardrailsOnly` harness.
    fn hook_settings_path(&self, _project_root: &Path) -> Option<PathBuf> {
        None
    }

    /// The harness's guardrails sink (FR-011, FR-012).
    fn guardrails_target(&self, project_root: &Path) -> GuardrailsTarget {
        GuardrailsTarget {
            placement: GuardrailsPlacement::InFileRegion {
                file: self.rules_file_target(project_root),
            },
            suppress_if_hooks_present: false,
        }
    }

    /// Whether this harness emits native translated agents (FR-030).
    /// Default `false`; only Phase 1–4 harnesses + Gemini stay `false`.
    fn supports_native_agents(&self) -> bool {
        false
    }

    /// Directory native agent files land in (FR-031).
    fn agent_dir(&self, _project_root: &Path) -> Option<PathBuf> {
        None
    }

    /// Native agent serialisation format (FR-030, FR-033).
    fn agent_format(&self) -> Option<AgentFormat> {
        None
    }

    /// Translate a canonical agent into this harness's native form (FR-030, FR-032).
    fn translate_agent(&self, _canonical: &agents::CanonicalAgent) -> agents::TranslatedAgent {
        unreachable!(
            "translate_agent called on a harness without native agent support: {}",
            self.name()
        )
    }
}
```

#### Agent Type Definitions (Phase 6 Foundational)

```rust
// src/harness/agents.rs — Canonical + translated forms

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalAgent {
    pub name: String,
    pub description: Option<String>,
    pub body: String,  // System-prompt Markdown
    pub model: Option<String>,
    pub tools: Option<Vec<String>>,
    pub disallowed_tools: Option<Vec<String>>,
    pub hooks: Option<serde_json::Value>,  // Privileged; passed through opaque
    pub mcp_servers: Option<serde_json::Value>,  // Privileged
    pub permission_mode: Option<String>,  // Privileged
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranslatedAgent {
    pub dir: PathBuf,
    pub filename: String,  // <plugin>__<name>.<ext>
    pub displayed_name: String,  // <name> or clash-prefixed
    pub format: AgentFormat,  // MarkdownYaml or Toml
    pub rendered: String,  // File content
    pub dropped_fields: Vec<String>,  // Diagnostics
}
```

#### EntryKind Exhaustive Match Pattern (Phase 6 Foundational)

```rust
// Every exhaustive match over EntryKind was widened to Agent

match entry.kind {
    EntryKind::Skill => {
        // Handle skill
    }
    EntryKind::Command => {
        // Handle command
    }
    EntryKind::Agent => {
        // Handle agent (Phase 6 NEW)
        // Per FR-070a: agents are never searchable, never user_invocable
        // Queries filter them out at MCP discovery time
    }
}
```

#### Schema Migration v3→v4 Marker Pattern (Phase 6 Foundational)

```rust
// src/index/migrations.rs — No DDL; version advance only

fn migrate_v3_to_v4(db: &rusqlite::Connection) -> Result<(), TomeError> {
    // No structural changes — the `kind` column admits 'agent' without DDL
    // No backfill — agent rows inserted by US1 indexing
    // Only advance the schema version constant
    Ok(())
}

// src/index/schema.rs
pub const SCHEMA_VERSION: u32 = 4;
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

*This document shows WHERE code lives. Updated 2026-05-29 against Phase 6 Foundational WIRED (harness trait extended with 7 methods for hooks/guardrails/agents; EntryKind::Agent variant + exhaustive match widening; schema v3→v4 marker migration; 4 new exit codes 43–46). Test suites: Phase 5 baseline + entry_kind_agent_indexing, harness_trait_p6, schema_migration_p6, exit_codes.*

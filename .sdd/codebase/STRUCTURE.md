# Project Structure

> **Purpose**: Document directory layout, module boundaries, and where to add new code.
> **Generated**: 2026-05-29
> **Last Updated**: 2026-06-04 (Phase 6 / US4; settings/scopes.rs NEW SSOT for canonical scope loaders; prompts.rs persona identities appending + persona-role dispatch; PersonaRole enum + display_name + plugin_version caching on PromptEntry; resolve_expose_personas startup-scope resolution in mcp/mod.rs; index/skills.rs agent queries extended with plugin_version + indexed_at projection)

## Directory Layout

```
tome/
├── src/                                # Rust library + binary source
│   ├── main.rs                         # CLI entry: scope resolution, command dispatch, error mapping
│   ├── lib.rs                          # Public exports
│   ├── cli.rs                          # clap derive defs (all commands + global flags)
│   ├── error.rs                        # Closed TomeError enum (38 variants → exit codes; Phase 6: +4 new codes 43–46)
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
│   │   ├── frontmatter.rs              # SKILL.md + command + agent YAML frontmatter parser (Phase 5: commands widened; Phase 6: agent fields reserved; US1: agent parsing complete)
│   │   ├── identity.rs                 # PluginId + Phase 5 NEW: EntryKind enum (Skill | Command | Agent) + canonical from_str(); Phase 6: Agent variant + exhaustive match widening
│   │   ├── components.rs               # Walk skill/command/agent dirs; Phase 5: list_command_files; Phase 6/US1: list_agent_files fully wired
│   │   └── lifecycle.rs                # enable/disable/reindex orchestration (Phase 5: commands + skills; Phase 6/US1: agents fully integrated)
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
│   │   ├── skills.rs                   # Phase 5: CRUD over unified skills table with EntryKind discriminator; resolve_entry_body_path + validate_db_stored_path SSOT (Polish); Phase 6: agent rows (searchable=false, user_invocable=false per FR-070a); US1: agent_name_clash_set + enabled_agents_for_workspace queries wired; US2: enabled_plugins_for_workspace query wired; US4: agent queries extended to project plugin_version + indexed_at
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
│   ├── doctor/                         # Diagnostic + auto-repair (Phase 3 US4 + Phase 4 US5 + Phase 5 US5 + Phase 6 skeleton + US1 agent integration + US2 hooks integration + US3 guardrails integration)
│   │   ├── mod.rs                      # assemble_report + re_assemble entry
│   │   ├── checks.rs                   # check_catalogs, check_index, check_drift, check_workspace_registry + Phase 5 / US5: build_prompts_report, count_entries_by_kind, detect_orphan_data_dirs (all read-only); Phase 6/US1: agent diagnostics integrated; Phase 6/US2: hooks diagnostics skeleton; Phase 6/US3: guardrails diagnostics skeleton
│   │   ├── harness_detect.rs           # Probe ~/.claude/, ~/.codex/, ~/.cursor/, ~/.gemini/, ~/.opencode/
│   │   ├── report.rs                   # DoctorReport + Subsystem (typed 11-variant enum) + SubsystemHealth + Phase 5 / US5: PromptsReport, EntryCountsByKind, OrphanDataDirReport; Phase 6/US1: agent count integrated
│   │   ├── fixes.rs                    # apply + apply_one (subsystem routing) + per-subsystem repair handlers
│   │   ├── binding.rs                  # Phase 4 US5: check_binding (T366) — marker well-formedness + RULES.md drift
│   │   ├── harness_integration.rs      # Phase 4 US5: check_harness_integration (T367) — per-harness rules/mcp checks
│   │   └── orphan_cleanup.rs           # Phase 4 US5: cleanup_stale_staging_dirs (FR-410) — 1-hour age gate
│   │
│   ├── harness/                        # Phase 4+: Per-harness trait + sync orchestrator + composition; Phase 6: trait extension for hooks/guardrails/agents; US1: native agent translation fully wired; US2: real hooks reconciliation fully wired; US3: guardrails rendering fully wired; US4: no new harness methods
│   │   ├── mod.rs                      # HarnessModule trait (Phase 4: 8 methods; Phase 6 Foundational: +7 new methods all safe-by-default); SUPPORTED_HARNESSES registry; shape enums (HooksStrategy, GuardrailsPlacement, GuardrailsTarget, AgentFormat)
│   │   ├── guardrails.rs               # **Phase 6 / US3 COMPLETE** Guardrails prose soft-fallback SSOT: read_guardrails_source (verbatim body + marker-shape validation, fail-closed), region_key, marker renderers; marker regions per plugin with FR-013 Claude Code suppression (hooks_present filter); used by sync 3a subsystem
│   │   ├── hooks.rs                    # **Phase 6 / US2 COMPLETE** Hooks parsing + rewrite SSOT: read_rewritten_entries, targeted two-variable rewrite (${CLAUDE_PLUGIN_ROOT}/${CLAUDE_PLUGIN_DATA} → absolute; other ${CLAUDE_*} verbatim), merge_into_settings (idempotent append), remove_from_settings (structural removal), ownership model (re-derivation + deep-equal), atomic writes (symlink-refusing, mode-preserving)
│   │   ├── agents.rs                   # **Phase 6 / US1 COMPLETE** Agent type definitions + SSOT parsing/translation: CanonicalAgent::parse, agent_filename, plugin_of_owned_file (ownership split), is_safe_agent_name, map_model (alias table), infer_read_only, displayed_name, render_markdown_yaml, render_codex_toml
│   │   ├── claude_code.rs              # Claude Code harness impl; Phase 6/US1: translate_agent() override, agent_dir(), agent_format(), supports_native_agents() wired; Phase 6/US2: hooks_strategy() = RealJson, hook_settings_path() wired; Phase 6/US3: guardrails_target() with Claude Code suppression, rules_file_target() corrected (CLAUDE.md > .claude/CLAUDE.md, AGENTS.md dropped per FR-020/021/022)
│   │   ├── codex.rs                    # Codex harness impl; Phase 6/US1: translate_agent() override for TOML emission, agent_dir(), agent_format() wired; Phase 6/US3: guardrails_target() in-file AGENTS.md
│   │   ├── cursor.rs                   # Cursor harness impl; Phase 6/US1: translate_agent() override, agent_dir(), agent_format() wired; Phase 6/US3: guardrails_target() with standalone sibling (or in-file per strategy)
│   │   ├── gemini.rs                   # Gemini CLI harness impl; no native agent support (supports_native_agents=false); Phase 6/US3: guardrails_target() in-file AGENTS.md
│   │   ├── opencode.rs                 # OpenCode harness impl; Phase 6/US1: translate_agent() override with <plugin>__<name> display, agent_dir(), agent_format() wired; Phase 6/US3: guardrails_target() in-file AGENTS.md
│   │   ├── rules_file.rs               # **Phase 6 / US3 COMPLETE** Generalised marker engine (not just Phase 4 `tome:begin/end`): MarkerSpec (parameterised regex pair + render funcs), MarkerRegion, find_marker_regions, compose_in_file (lex-merge, deterministic order, orphan removal, atomic write); refuse_symlink/atomic_write promoted to pub(crate); used by guardrails 3a subsystem
│   │   ├── mcp_config.rs               # JSON + TOML MCP config read/write primitives
│   │   ├── sync.rs                     # Phase 4: Sync orchestrator (per-project harness writes); Phase 6/US1: reconcile_agents pass (3c subsystem) fully integrated; Phase 6/US2: reconcile_hooks pass (3b subsystem) fully integrated; Phase 6/US3: reconcile_guardrails pass (3a subsystem) fully integrated BEFORE hooks BEFORE agents; clash set computation (FR-072); forward progress (FR-084); hooks suppression set computation for Claude Code
│   │   └── stub.rs                     # StubHarnessModule for test injection; Phase 6: extended with agent/hook/guardrails method overrides for testing; US1: agent translation test overrides; US2: hooks parsing test overrides; US3: guardrails rendering test overrides
│   │
│   ├── settings/                       # Phase 4: Layered harness composition; Phase 6 / US4: scalar settings resolver + canonical scope loaders
│   │   ├── mod.rs                      # Type defs (ProjectMarkerConfig, WorkspaceSettings, GlobalSettings); resolve_scalar() / resolve_scalar_with() first-declarer-wins scalar resolvers for expose_agents_as_personas (FR-053, R-12)
│   │   ├── parser.rs                   # TOML deserialization (strict)
│   │   ├── composition.rs              # CompositionRef + reference parsing
│   │   ├── resolver.rs                 # Resolve effective harness list (priority walk + composition refs + ScopeProvider trait)
│   │   ├── scopes.rs                   # **Phase 6 / US4 NEW** Canonical scope-file loaders SSOT: load_project_marker, load_workspace_settings, load_global_settings (promoted from three prior copies: mcp::resolve_expose_personas + commands::harness::list + harness::sync); R4-2 SSOT elimination
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
│   │   │   ├── enable.rs               # `tome plugin enable <id>` + trigger regenerate (Phase 5: commands + skills; Phase 6/US1: agents fully integrated)
│   │   │   ├── disable.rs              # `tome plugin disable <id> [--force]` + trigger regenerate
│   │   │   ├── list.rs                 # `tome plugin list` (Phase 5 / US5: per-kind entry counts; Phase 6/US1: agent count)
│   │   │   ├── show.rs                 # `tome plugin show <id>` (Phase 5 / US5: ~228 lines extended for searchable/invocable annotations + kind grouping; Phase 6/US1: Agents section)
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
│   │   │   ├── mod.rs                  # Dispatcher (6 subcommands) + CentralDbScopeProvider impl; Phase 6/US4: uses scopes::load_project_marker + load_workspace_settings + load_global_settings (R4-2)
│   │   │   ├── bare.rs                 # `tome harness` (no subcommand) — list all supported harnesses
│   │   │   ├── list.rs                 # `tome harness list [workspace]` — resolve effective harness list; Phase 6/US4: uses scope loaders
│   │   │   ├── use_.rs                 # `tome harness use <name> [--scope {project|workspace|global}]` + trigger regenerate
│   │   │   ├── remove.rs               # `tome harness remove <name> [--scope]` — delete from settings + trigger regenerate
│   │   │   ├── info.rs                 # `tome harness info` — per-harness details + detection
│   │   │   └── sync.rs                 # `tome harness sync [--force]` — reconcile filesystem; Phase 6/US4: uses scope loaders
│   │   ├── doctor.rs                   # `tome doctor [--fix] [--verify] [--force]` (Phase 5 / US5: renders extended report with prompts + entry-kind counts + orphan data-dirs; Phase 6/US1: agent integration; Phase 6/US2: hooks skeleton; Phase 6/US3: guardrails skeleton)
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
│   └── mcp/                            # MCP server (async island, Phase 3+; Phase 5: prompts + US4 three-tier discovery + US5 read-only extensions + Polish: substitution_helpers; Phase 6/US1: agent rows excluded from search/prompts per FR-070a; Phase 6/US2: hooks excluded from prompts; Phase 6/US3: guardrails excluded from prompts; Phase 6/US4: agent personas appended to Phase 5 registry + persona-role dispatch in prompts/get)
│       ├── mod.rs                      # Sync entry point: run(); Phase 6/US4: resolve_expose_personas() for startup-scope flag resolution
│       ├── runtime.rs                  # Single-threaded tokio builder
│       ├── log.rs                      # 10 MiB rotate JSON file logger (contract-formatted for tool logs)
│       ├── preflight.rs                # FR-110 startup checks (schema, drift, embedder hash)
│       ├── server.rs                   # rmcp server loop + graceful shutdown
│       ├── state.rs                    # McpState definition (embedder, reranker OnceLock)
│       ├── substitution_helpers.rs     # Phase 5 Polish NEW: build_context_for_entry() SSOT (shared across prompts/get + get_skill_info)
│       ├── tool_description.rs         # Phase 4 US4.b: Compose runtime tool description from cached summary
│       ├── prompt_name.rs              # Phase 5 NEW: Prompt-name derivation (<plugin>__<entry> sanitisation + truncation); US4: derive_suffixed_name for persona names
│       ├── prompt_collision.rs         # Phase 5 NEW: Collision detection when entries map to same prompt name
│       ├── prompts.rs                  # Phase 5 NEW: MCP prompts capability (PromptRegistry, PromptRouter hand-rolled); Phase 6/US4: PersonaRole enum + agent persona identities appending + persona-role dispatch in prompts/get (wraps agent body in template, applies Phase 5 substitution); display_name + plugin_version caching on PromptEntry
│       └── tools/                      # MCP tool handlers (Phase 5 / US4–US5: three-tier discovery + read-only extensions; Phase 6/US1: agent rows excluded from search/prompts per FR-070a; Phase 6/US2: hooks excluded; Phase 6/US3: guardrails excluded)
│           ├── mod.rs                  # Tool registration
│           ├── search_skills.rs        # search_skills tool (KNN+rerank, workspace-filtered, 4096-char input cap, Phase 5 / US4: when_to_use in results, truncate_description hardening; Polish: mirrors truncation at get_skill_info; Phase 6/US1: agent rows searchable=false, excluded from results per FR-070a)
│           ├── get_skill_info.rs       # Phase 5 / US4 NEW: get_skill_info middle-tier tool (full description + when_to_use + 5-cap resource enumeration; Polish: uses build_context_for_entry SSOT; Phase 6/US1: agent rows excluded per FR-070a)
│           └── get_skill.rs            # get_skill tool (metadata + components); Phase 6/US1: agent rows excluded from prompts, MCP discovery per FR-070a
│
├── tests/                              # Integration tests (access library as external crate)
│   ├── catalog_*.rs                    # Catalog add/remove/update tests
│   ├── plugin_*.rs                     # Plugin enable/disable/list/show/interactive (Phase 5: commands coverage + US5 annotations; Phase 6/US1: agent entry-kind + translation tests; Phase 6/US2: hooks tests; Phase 6/US3: guardrails tests)
│   ├── models_*.rs                     # Model download/list/remove
│   ├── query.rs                        # Query + strict mode + rerank
│   ├── reindex.rs                      # Reindex all/per-catalog/per-plugin
│   ├── status.rs                       # Status command + health checks
│   ├── workspace_*.rs                  # Workspace info/init/binding/sync/list/rename/remove tests (US1–US2)
│   ├── harness_*.rs                    # Phase 4 US3: Harness list/use/remove/info/sync/composition tests; Phase 6: harness_trait_p6.rs for trait extension; US1: harness_agents_*.rs for translation + sync; US2: harness_hooks_*.rs for parsing + sync; US3: harness_guardrails_*.rs for rendering + sync
│   ├── summariser_*.rs                 # Phase 4 US4: Summariser triggers, forward progress, cache, registry tests
│   ├── doctor*.rs                      # Phase 4 US5: Doctor assembly + fixes + binding + harness integration + orphan cleanup; Phase 5 / US5: prompts report + entry counts + orphan data-dirs; Phase 6/US1: agent integration tests; Phase 6/US2: hooks skeleton tests; Phase 6/US3: guardrails skeleton tests
│   ├── mcp_*.rs                        # MCP server lifecycle + tools + log rotation + tool description (US4.b) + prompts (US1.b) + Phase 5 / US4–US5: get_skill_info tests + read-only extensions; Phase 6/US1: agent exclusion tests; Phase 6/US2: hooks excluded; Phase 6/US3: guardrails excluded; Phase 6/US4: personas_startup_scope, personas_registry_building, personas_prompts_get
│   ├── substitution_*.rs               # Phase 5: Substitution engine tests (skeleton, builtins, env, arguments, data-dir, e2e)
│   ├── entry_kind_agent_indexing.rs    # **Phase 6 Foundational NEW** Agent entry-kind indexing + schema widening tests
│   ├── harness_trait_p6.rs             # **Phase 6 Foundational NEW** HarnessModule trait extension (7 new methods, safe-by-default impls, exhaustive match widening)
│   ├── schema_migration_p6.rs          # **Phase 6 Foundational NEW** Schema v3→v4 marker migration (no DDL, version advance only)
│   ├── harness_agents_translation.rs   # **Phase 6 / US1 NEW** Agent parsing (CanonicalAgent::parse), translation (per-harness overrides), model alias, read-only inference, display-name clash handling
│   ├── harness_agents_sync.rs          # **Phase 6 / US1 NEW** Sync reconciliation (reconcile_agents), clash-set computation, enabled-agent enumeration, per-harness emission, orphan cleanup, forward progress
│   ├── harness_agents_indexing_lifecycle.rs | **Phase 6 / US1 NEW** Agent enumeration (list_agent_files), lifecycle (collect_agent_entries, enable_plugin_atomic), index queries (agent_name_clash_set, enabled_agents_for_workspace)
│   ├── harness_hooks_parsing.rs        # **Phase 6 / US2 NEW** Hooks parsing (read_rewritten_entries), two-variable rewrite, idempotency tests
│   ├── harness_hooks_merge_remove.rs   # **Phase 6 / US2 NEW** Merge/remove semantics (append_if_absent, remove_if_present, prune_empty_event), ownership model tests
│   ├── harness_hooks_sync.rs           # **Phase 6 / US2 NEW** Sync reconciliation (reconcile_hooks), enabled-plugin enumeration, per-harness merge/remove, orphan cleanup, forward progress
│   ├── harness_guardrails_parsing.rs   # **Phase 6 / US3 NEW** Guardrails parsing (read_guardrails_source), verbatim body, marker-shape validation (fail-closed), per-plugin per-harness reconciliation
│   ├── harness_guardrails_reconciliation.rs | **Phase 6 / US3 NEW** Sync reconciliation (reconcile_guardrails), enabled-plugin enumeration, suppression-set computation, per-harness region composition (lex order, overwrite, orphan removal), forward progress
│   ├── harness_guardrails_marker_engine.rs | **Phase 6 / US3 NEW** Generalized marker engine (MarkerSpec, MarkerRegion, find_marker_regions, compose_in_file), lex-order determinism, idempotence, both Phase 4 `tome:begin/end` + guardrails coexistence
│   ├── harness_phase6_correction.rs   # **Phase 6 / US3 NEW** Claude Code rules-file sink corrected to CLAUDE.md > .claude/CLAUDE.md (AGENTS.md dropped per FR-020/021/022)
│   ├── personas_startup_scope.rs       # **Phase 6 / US4 NEW** resolve_expose_personas() startup-scope resolution; project→workspace→global scalar walk (FR-067 test seam)
│   ├── personas_scalar_resolver.rs     # **Phase 6 / US4 NEW** resolve_scalar() / resolve_scalar_with() first-declarer-wins behavior (FR-053, R-12)
│   ├── personas_registry_building.rs   # **Phase 6 / US4 NEW** Agent persona identity appending (clash prefix, reserved drop-persona, unified collision namespace, FR-066)
│   ├── personas_prompts_get.rs         # **Phase 6 / US4 NEW** Persona-role dispatch in prompts/get (template wrapping, substitution, display_name resolution)
│   ├── entry_e2e.rs                    # Phase 5 / US3 NEW: Full enable → search → get → prompts pipeline with argument substitution + Phase 5 / US5: invocability visibility; Phase 6/US1: agent rows excluded
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
│       ├── ARCHITECTURE.md             # System design + patterns (Phase 6 / US4: scalar settings resolver for expose_agents_as_personas + startup-scope resolution; persona identities appended to Phase 5 registry in collision namespace; persona-role path in prompts/get with template wrapping + substitution; PersonaRole enum discriminator)
│       ├── STRUCTURE.md                # Directory layout (Phase 6 / US4: settings/scopes.rs NEW SSOT for canonical scope loaders; prompts.rs persona identities appending + persona-role dispatch; PromptEntry display_name + plugin_version caching; resolve_expose_personas in mcp/mod.rs; agent queries extended with plugin_version + indexed_at projection)
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
│   └── 006-phase-6-hooks-agents/           # Phase 6 (Foundational + US1–US4 complete; spec + plan + contracts + research + retro complete)
│       ├── spec.md
│       ├── plan.md
│       ├── research.md (20 R-decisions)
│       ├── data-model.md (v4 schema marker, HooksStrategy, GuardrailsTarget, GuardrailsPlacement, AgentFormat, CanonicalAgent, TranslatedAgent, agent clash-set, enabled-agent enumeration, RewrittenHooks, ownership model, MarkerSpec, MarkerRegion, PersonaRole, PersonaIdentity, etc.)
│       ├── contracts/ (9+ contracts: exit-codes-p6, schema-migration-p6, entry-schema-p6, harness-modules-p6, hooks-reconciliation, guardrails-prose, agent-translation, agent-sync, agent-indexing, agent-personas, etc.)
│       ├── retro/ (P2–P4 retrospectives)
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
├── CLAUDE.md                           # Project context for Claude Code (Phase 6: Foundational + US1–US4 complete)
└── CHANGELOG.md                        # Version history (v0.1.0–v0.5.0 shipped; Phase 6 US1–US4 in development)
```

## Key Directories

### `src/` — Source Code

| Directory | Purpose | Key Files |
|-----------|---------|-----------|
| `substitution/` | Phase 5 / US1–US3: Variable rendering engine (COMPLETE single-pass pipeline) | `mod.rs` (render loop + body_has_bare_arguments), `context.rs`, `builtins.rs`, `env.rs`, `arguments.rs` (shell_split + coerce_arguments + apply_arguments_match), `data_dir.rs`, `regex_sets.rs` (COMBINED_RE) |
| `plugin/` | Plugin metadata, lifecycle (Phase 5: commands + arguments + when_to_use + user_invocable; Phase 6/US1: agents enumeration + lifecycle + frontmatter parsing) | `manifest.rs`, `frontmatter.rs`, `identity.rs` (EntryKind + Agent variant + canonical from_str), `components.rs` (list_command_files, list_agent_files), `lifecycle.rs` (collect_pending_agents) |
| `index/` | SQLite + sqlite-vec index (Phase 5: v3 schema with when_to_use; Polish: validate_db_stored_path SSOT; Phase 6: v4 marker migration, agent rows with searchable=false/user_invocable=false; US1: agent queries wired; US2: enabled_plugins_for_workspace query wired; US4: agent queries extended with plugin_version + indexed_at) | `db.rs`, `schema.rs` (v4 marker), `migrations.rs` (v3→v4), `skills.rs` (agent queries: agent_name_clash_set, enabled_agents_for_workspace, enabled_plugins_for_workspace; US4: projections extended; US2 hooks enumeration), `query.rs` (Phase 5 / US4: when_to_use embeddings) |
| `mcp/` | MCP server + Phase 5 prompts + three-tier discovery + read-only extensions + Polish: substitution_helpers; Phase 6/US1: agent rows excluded per FR-070a; Phase 6/US2: hooks excluded; Phase 6/US3: guardrails excluded; Phase 6/US4: agent personas appended to registry in collision namespace + persona-role dispatch in prompts/get | `prompts.rs` (PromptRegistry, PersonaRole enum, persona identity appending, persona-role dispatch), `prompt_name.rs` (derive_suffixed_name for personas), `prompt_collision.rs` (unified namespace for personas + skills/commands), `substitution_helpers.rs` (build_context_for_entry SSOT), `mod.rs` (resolve_expose_personas startup-scope), `tools/` (search_skills, get_skill_info, get_skill) |
| `doctor/` | Health check + auto-repair (Phase 5 / US5: read-only extensions; Phase 6/US1: agent count integrated; Phase 6/US2: hooks skeleton; Phase 6/US3: guardrails skeleton) | `checks.rs` (count_entries_by_kind extended for agent count; hooks/guardrails checks skeleton), `report.rs` (agent counts; hooks/guardrails skeleton), fixes.rs |
| `catalog/` | Catalog registry, git ops | `manifest.rs`, `store.rs`, `git.rs` |
| `embedding/` | Text embedding + reranking | `fastembed.rs`, `stub.rs`, `download.rs` |
| `workspace/` | Scope resolution, binding, lifecycle (Phase 5 / US2: rename relocation) | `scope.rs`, `binding.rs`, `init.rs`, `rename.rs`, `remove.rs`, `regen_summary.rs` |
| `settings/` | Phase 4: Layered composition; Phase 6 / US4: scalar resolver + scope loaders SSOT | `mod.rs` (resolve_scalar / resolve_scalar_with, ProjectMarkerConfig/WorkspaceSettings/GlobalSettings with expose_agents_as_personas), `scopes.rs` (NEW: load_project_marker, load_workspace_settings, load_global_settings SSOT), `parser.rs`, `resolver.rs` (composition engine), `edit.rs` |
| `harness/` | Phase 4: Harness abstraction + sync; Phase 6 Foundational: trait extension for hooks/guardrails/agents; US1: native agent translation fully wired; US2: real hooks parsing + sync fully wired; US3: guardrails rendering + marker engine fully wired; US4: no new harness methods | `mod.rs` (trait +7 new methods safe-by-default; shape enums), `guardrails.rs` (verbatim body read, marker validation, per-plugin region rendering SSOT), `hooks.rs` (RewrittenHooks, read_rewritten_entries, merge/remove semantics, atomic writes, two-variable rewrite SSOT), `agents.rs` (CanonicalAgent::parse, translation SSOT, model-alias, naming, render primitives), 5 harness impls (translate_agent overrides + US2: hooks_strategy / hook_settings_path + US3: guardrails_target), `rules_file.rs` (MarkerSpec/MarkerRegion, find_marker_regions, compose_in_file, parameterised marker engine for both Phase 4 `tome:begin/end` + guardrails), `sync.rs` (reconcile_guardrails 3a + reconcile_hooks 3b + reconcile_agents 3c passes), `mcp_config.rs`, `stub.rs` |
| `commands/` | CLI subcommand entry points (Phase 5 / US5: show + list extended; Phase 6/US1: agent integration; Phase 6/US2: hooks integration skeleton; Phase 6/US3: guardrails integration skeleton; Phase 6/US4: scope loaders used in harness list/sync) | Per-command modules + dispatchers |
| `presentation/` | Output formatting + TUI | `tables.rs`, `prompt.rs`, `colour.rs` |
| `util/` | Shared utilities | `atomic_dir.rs` (tempfile + rename), `io.rs` (bounded read) |
| `paths.rs` | Phase 4 single-root layout; Phase 5: data-dir accessors; Polish: plugin_data_root() SSOT | `home_root()`, `Paths struct`, `plugin_data_root()` SSOT, `plugin_data_dir_for()`, `workspace_data_dir_for()` |

### `src/settings/scopes.rs` — Settings Scope Loaders (Phase 6 / US4 NEW)

| Function | Purpose | Contract |
|----------|---------|----------|
| `load_project_marker()` | Load project marker from `<project>/.tome/config.toml` | `Ok(None)` when absent; parse failure → exit 70; IO → exit 7 |
| `load_workspace_settings()` | Load workspace settings from `<root>/workspaces/<name>/settings.toml` | `Ok(None)` when absent; parse failure → exit 70 |
| `load_global_settings()` | Load global settings from `<root>/settings.toml` | Absent file → `GlobalSettings::default()`; parse failure → exit 70 |

### `src/mcp/prompts.rs` — Agent Personas (Phase 6 / US4)

| Type/Function | Purpose | Details |
|----------|---------|---------|
| `PersonaRole` enum | Discriminates Phase 5 command/skill from Phase 6 persona variants | `None` (Phase 5 path), `Agent` (persona-role path with template), `Drop` (fixed body) |
| `PromptEntry::persona` | Persona role for dispatch in `prompts/get` | Added in US4; drives routing to template-wrapping vs body-reading path |
| `PromptEntry::display_name` | Agent's display name (frontmatter `name`, else filename stem) | Empty for non-persona entries; used in persona template rendering |
| `PromptEntry::plugin_version` | Cached from DB for `${TOME_PLUGIN_VERSION}` substitution in persona body | Already present since Polish; extended in US4 to persona entries |
| `build_for_workspace()` | Build registry: Phase 5 skills/commands + (if expose_personas) agent personas + drop-persona | Appends persona identities after Phase 5 query; folds into SINGLE collision namespace |
| `collect_persona_identities()` | Append one `<name>-persona` identity per enabled agent + `drop-persona` | Clash prefix applied before collision pass; drop-persona reserved via empty indexed_at seed |

## Module Boundaries

### Where to Add New Code (Phase 6 / US4)

| If you're adding... | Put it in... | Pattern |
|---------------------|--------------|---------|
| New harness | `src/harness/{name}.rs` + register in `mod.rs` | Impl `HarnessModule` trait (8 methods Phase 4 + 7 new Phase 6, all methods override-able for per-harness specifics) |
| Guardrails parsing logic | `src/harness/guardrails.rs` | Guardrails rendering SSOT complete in US3 (no further changes unless new fallback mechanism added) |
| Guardrails region composition | `src/harness/rules_file.rs` | `compose_in_file()` shared for both Phase 4 `tome:begin/end` + guardrails; MarkerSpec parameterises both |
| Guardrails reconciliation | `src/harness/sync.rs` | `reconcile_guardrails()` (3a) fully wired in US3; called in `sync_project()` BEFORE hooks BEFORE agents |
| Hooks parsing logic | `src/harness/hooks.rs` | Hooks rewriting SSOT complete in US2 (no further changes unless US3 guardrails alter strategy) |
| Hooks merge/remove | `src/harness/hooks.rs` | merge_into_settings / remove_from_settings functions; SSOT for ownership model (re-derivation + deep-equal) |
| Hooks reconciliation | `src/harness/sync.rs` | reconcile_hooks() passes (3b) fully wired in US2; called in sync_project() AFTER guardrails BEFORE agents |
| Agent parsing | `src/plugin/frontmatter.rs` | Agent frontmatter parsing complete in US1 (no changes) |
| Agent enumeration | `src/plugin/components.rs` | `list_agent_files()` fully wired in US1 |
| Agent lifecycle | `src/plugin/lifecycle.rs` | `collect_pending_agents()` fully wired in US1; no further changes needed |
| Agent type | `src/harness/agents.rs` | Both types (CanonicalAgent, TranslatedAgent) complete; all helpers (parse, filename, alias, read-only, naming, renders) complete in US1 |
| Harness agent override | `src/harness/{name}.rs` | Override `supports_native_agents()`, `agent_dir()`, `agent_format()`, `translate_agent()` methods (US1 complete for all five harnesses) |
| Harness hook override | `src/harness/{name}.rs` | Override `hooks_strategy()` and `hook_settings_path()` methods (US2 complete; Claude Code returns RealJson + path; all others GuardrailsOnly + None) |
| Harness guardrails override | `src/harness/{name}.rs` | Override `guardrails_target()` method (US3 complete; Claude Code returns in-file placement + suppress_if_hooks_present=true; all others return in-file AGENTS.md or Cursor sibling placement) |
| Persona startup resolution | `src/mcp/mod.rs` | `resolve_expose_personas()` fully wired in US4; called once per MCP server startup; passed to `PromptRegistry::build_for_workspace()` |
| Persona registry building | `src/mcp/prompts.rs` | `collect_persona_identities()` fully wired in US4; appends identities to shared Phase 5 registry before collision pass |
| Persona dispatch in prompts/get | `src/mcp/prompts.rs` | `PersonaRole` dispatch in `prompts/get` handler fully wired in US4; wraps agent body in template, applies Phase 5 substitution |
| Scalar settings field | `src/settings/mod.rs` | Add Option<bool> field to ProjectMarkerConfig / WorkspaceSettings / GlobalSettings; call `resolve_scalar_with()` with field accessor; reuses resolver for second scalars (e.g., US5's `strip_plugin_agent_privileges`) |
| Entry-kind exhaustive match | `src/commands/plugin/{mod,list,show}.rs`, `src/doctor/{checks,report}.rs`, `src/plugin/frontmatter.rs` | All matches extended to handle `EntryKind::Agent` in US1; defence-in-depth via canonical `from_str()` |
| Agent doctor check | `src/doctor/checks.rs` | Stub extends to real checks in US5 (agent/hook/guardrails diagnostics) |
| Hooks doctor check | `src/doctor/checks.rs` | Skeleton in US2; full implementation in US5 |
| Guardrails doctor check | `src/doctor/checks.rs` | Skeleton in US3; full implementation in later phase |
| Agent visibility | `src/commands/plugin/{show,list}.rs` | Consult agent rows from index; filter per invariants (searchable=false, user_invocable=false always) |
| Hooks visibility | `src/commands/plugin/{show,list}.rs` | US3 guardrails will surface hooks in doctor; US2 no display |
| Guardrails visibility | `src/commands/plugin/{show,list}.rs` | US3 guardrails will surface guardrails status in doctor; not surfaced in show/list |
| Hook reconciliation sink | `src/harness/sync.rs` + per-harness impl | US2 hooks reconciliation complete in sync.rs; US3 wires guardrails (3a) BEFORE hooks (3b) |
| Guardrails rendering sink | `src/harness/sync.rs` + per-harness impl | US3 guardrails reconciliation complete in sync.rs; runs BEFORE hooks (3b) before agents (3c) |
| New marker family | `src/harness/rules_file.rs` + `src/harness/{new}.rs` | Parameterised `MarkerSpec` + `find_marker_regions()` + `compose_in_file()` support any marker family (used by Phase 4 `tome:begin/end` + guardrails in US3) |
| New substitution stage | `src/substitution/{stage}.rs` | Phase 5 complete; no new stages needed |
| Schema change | `src/index/{schema,migrations}.rs` | v4 marker in Foundational, no backfill; agent rows use existing kind column; US1–US2–US3–US4 indexing complete |
| Exit code | `src/error.rs` + `tests/exit_codes.rs` | Phase 6 codes 43–46 all wired in Foundational + US1–US2–US3 (43: hook parse, 44: hook write, 45: agent translation, 46: guardrails write) |
| Persona test | `tests/personas_*.rs` | Four test files for US4: personas_startup_scope, personas_scalar_resolver, personas_registry_building, personas_prompts_get |

### Key Patterns

#### Settings Scalar Resolver Pattern (Phase 6 / FR-053, R-12)

First-declarer-wins `bool` setting walk:
```rust
resolve_scalar(
    project.and_then(|p| p.expose_agents_as_personas),
    workspace.and_then(|w| w.expose_agents_as_personas),
    global.expose_agents_as_personas,
)
```

Reusable closure form for new scalars:
```rust
resolve_scalar_with(
    project, workspace, &global,
    |p| p.expose_agents_as_personas,
    |w| w.expose_agents_as_personas,
    |g| g.expose_agents_as_personas,
)
```

#### Persona Registry Building Pattern (Phase 6 / US4)

1. Query enabled user-invocable entries (skills/commands only)
2. If `expose_personas=true`: append persona identities (one per agent + drop-persona)
3. Resolve collisions over unified namespace (Phase 5 collision resolver reused)
4. Render persona entries with `PersonaRole::Agent` / `PersonaRole::Drop` discriminator

#### Persona Dispatch in prompts/get (Phase 6 / US4)

```rust
match entry.persona {
    PersonaRole::None => { /* Phase 5 body-reading path */ }
    PersonaRole::Agent => { /* Frontmatter-strip, template-wrap, apply substitution */ }
    PersonaRole::Drop => { /* Fixed body, no file */ }
}
```

Template is fixed per contract; wraps agent body (frontmatter-stripped) with role-assumption tags and `$ARGUMENTS` placeholder; Phase 5 substitution applied (no parallel substitution path per NFR-007).

---

*This document shows WHERE code lives. Updated 2026-06-04 against Phase 6 / US4 COMPLETE (settings/scopes.rs NEW SSOT for canonical scope loaders; prompts.rs persona identities appending + persona-role dispatch; PersonaRole enum + display_name + plugin_version caching on PromptEntry; resolve_expose_personas startup-scope resolution in mcp/mod.rs; index/skills.rs agent queries extended with plugin_version + indexed_at projection). Test suites: Phase 5 baseline + entry_kind_agent_indexing, harness_trait_p6, schema_migration_p6, exit_codes + US1: agent_translation, agent_sync_reconciliation, agent_indexing_lifecycle, agent_e2e + US2: hooks_parsing, hooks_merge_remove, hooks_sync_reconciliation + US3: guardrails_parsing, guardrails_reconciliation, guardrails_marker_engine, phase6_correction_claude_code.* + US4: personas_startup_scope, personas_scalar_resolver, personas_registry_building, personas_prompts_get*

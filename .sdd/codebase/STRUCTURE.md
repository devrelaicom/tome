# Project Structure

> **Purpose**: Document directory layout, module boundaries, and where to add new code.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13 (Phase 3 User Story 1) + 2026-05-13 (Phase 4 User Story 2 — interactive browse) + 2026-05-13 (Phase 5 User Story 3 — plugin disable subcommand) + 2026-05-13 (Phase 6 User Story 4 slice 1 — models commands) + 2026-05-13 (Phase 7 User Stories 5–7 — reindex orchestrator, catalog-update cascade, explicit CLI) + 2026-05-13 (Phase 8 User Story 6 — health diagnostics) + 2026-05-14 (Phase 9 User Story 7 — catalog remove cascade) + 2026-05-14 (Foundational F7 + F8 — schema migrations framework, MCP async island) + 2026-05-14 (Phase 3 User Story 1 — MCP server wired) + 2026-05-14 (Phase 3 User Story 2 — workspace context, `tome workspace info/init`) + 2026-05-14 (Phase 3 User Story 3 — per-command scope honouring, reference-counted catalog clone cleanup)

## Directory Layout

```
tome/
├── src/                           # Rust library and binary source
│   ├── main.rs                    # CLI entry point: parse → dispatch → exit (Phase 8: pre-parse --version hook; Phase 3 US1: skip logging/signals for MCP; Phase 3 US2: pre-dispatch workspace resolution; Phase 3 US3: scope resolution gates all commands)
│   ├── lib.rs                     # Public module surface
│   ├── cli.rs                     # clap derive definitions (global flags, subcommands; Phase 3 US1: McpArgs; Phase 3 US2: GlobalScopeArgs with --workspace/--global; Phase 3 US3: all commands route through scope resolution)
│   ├── error.rs                   # Closed TomeError enum + exit code mapping
│   ├── catalog/                   # Catalog management (Phase 1; Phase 3 US3: reference-counted cache)
│   │   ├── mod.rs                 # Module aggregation
│   │   ├── git.rs                 # Git shell-outs, signal handling, credential scrubbing
│   │   ├── manifest.rs            # TOML schema + strict parsing + semantic validation
│   │   └── store.rs               # Atomic registry and cache persistence; reference_count() for shared cache (Phase 3 US3)
│   ├── commands/                  # CLI command handlers
│   │   ├── mod.rs                 # Dispatcher: route to subcommand
│   │   ├── catalog/               # `tome catalog <subcommand>`
│   │   │   ├── mod.rs             # Subcommand dispatcher
│   │   │   ├── add.rs             # Register a catalog; reuse cached clone if URL already cached (Phase 3 US3)
│   │   │   ├── remove.rs          # Unregister a catalog; reference-count cache, delete only if refcount==0 (Phase 3 US3)
│   │   │   ├── list.rs            # List registered catalogs
│   │   │   ├── update.rs          # Refresh catalogs (Phase 7: wires reindex per enabled plugin)
│   │   │   ├── show.rs            # Inspect catalog manifest
│   │   │   └── source.rs          # URL resolution (owner/repo → GitHub URL)
│   │   ├── models/                # `tome models <subcommand>` (Phase 6)
│   │   │   ├── mod.rs             # Dispatcher + shared ModelState enum
│   │   │   ├── download.rs        # Download missing or --force models
│   │   │   ├── list.rs            # List all models with on-disk state
│   │   │   └── remove.rs          # Remove a model (delete manifest + dir)
│   │   ├── plugin/                # `tome plugin <subcommand>` + interactive (Phase 3–5)
│   │   │   ├── mod.rs             # Dispatcher + shared helpers
│   │   │   ├── enable.rs          # Enable a plugin (embed + index)
│   │   │   ├── disable.rs         # Disable a plugin (Phase 5)
│   │   │   ├── list.rs            # List plugins (all or for one catalog)
│   │   │   ├── show.rs            # Show one plugin's metadata + state
│   │   │   └── interactive.rs     # Bare `tome plugin` (no subcommand) interactive browse (Phase 4; Phase 3 US3: receives scope-resolved ResolvedScope)
│   │   ├── workspace/             # `tome workspace <subcommand>` (Phase 3 US2)
│   │   │   ├── mod.rs             # Dispatcher (info / init subcommands)
│   │   │   ├── info.rs            # `tome workspace info` — read-only scope diagnostics
│   │   │   └── init.rs            # `tome workspace init` — atomic .tome/ creation
│   │   ├── query.rs               # `tome query <text>` — KNN search (Phase 3)
│   │   ├── reindex.rs             # `tome reindex [<scope>] [--force]` — re-embedding (Phase 7; ~280 lines)
│   │   ├── status.rs              # `tome status [--verify]` — health diagnostics (Phase 8; ~330 lines)
│   │   └── mcp.rs                 # `tome mcp` — MCP server dispatcher (Phase 3 US1; ~20 lines)
│   ├── config.rs                  # Config and CatalogEntry structures (serde + toml)
│   ├── paths.rs                   # XDG-aware path resolution, cache key computation (scope-parametrized accessors Phase 3 US2)
│   ├── logging.rs                 # tracing-subscriber initialization
│   ├── output.rs                  # Human/JSON formatting, TTY detection
│   ├── plugin/                    # Plugin metadata + lifecycle (Phase 2/3/7)
│   │   ├── mod.rs                 # PluginRecord, PluginStatus, re-exports
│   │   ├── identity.rs            # PluginId: <catalog>/<plugin> address + FromStr
│   │   ├── manifest.rs            # plugin.json (lenient, serde_json; FR-013a)
│   │   ├── frontmatter.rs         # SKILL.md YAML header (lenient + FR-011/FR-012 fallbacks)
│   │   ├── components.rs          # ComponentCounts over skills/agents/commands/hooks/.mcp.json
│   │   └── lifecycle.rs           # enable / disable / reindex_plugin / cascade_disable_for_catalog orchestrator, resolve_plugin_dir (Phase 3/7/9)
│   ├── workspace/                 # Workspace context: scope resolution, workspace info/init (Phase 3 US2)
│   │   ├── mod.rs                 # Module aggregation, re-exports
│   │   ├── scope.rs               # Scope enum, ResolvedScope, ScopeSource types
│   │   ├── resolution.rs          # Scope resolution algorithm (--workspace, --global, env, CWD walk, fallback)
│   │   ├── info.rs                # WorkspaceInfo + ModelIdentity types (emit-only; library assemble function)
│   │   ├── init.rs                # InitOutcome type + atomic .tome/ creation logic
│   │   └── inventory.rs           # Optional workspace registry (${state_dir}/workspaces.txt)
│   ├── index/                     # SQLite + sqlite-vec local skill index (Phase 2/7)
│   │   ├── mod.rs                 # Re-exports (Phase 7: exports reindex_plugin_atomic)
│   │   ├── schema.rs              # CREATE TABLE statements, MetaSeed
│   │   ├── migrations.rs          # Forward-only migration framework + apply_pending (Foundational F7; ~120 lines)
│   │   ├── vec_ext.rs             # sqlite-vec auto-extension registrar
│   │   ├── db.rs                  # open(): paths → conn → PRAGMAs → bootstrap/migrate → verify
│   │   ├── lock.rs                # Advisory write lock via File::try_lock (per-fd, OS-level)
│   │   ├── meta.rs                # Typed MetaKey + read/write + DriftStatus + detect_drift
│   │   ├── integrity.rs           # PRAGMA integrity_check wrapper
│   │   ├── skills.rs              # CRUD + content_hash + enable_plugin_atomic + reindex_plugin_atomic (Phase 7; ~510 lines)
│   │   └── query.rs               # KNN over skill_embeddings joined with skills.enabled = 1
│   ├── embedding/                 # fastembed-rs wrapper, model registry, download (Phase 2)
│   │   ├── mod.rs                 # Embedder + Reranker traits, Scored, ModelKind
│   │   ├── registry.rs            # MODEL_REGISTRY const + ModelManifest (strict serde)
│   │   ├── download.rs            # Atomic, SIGINT-aware reqwest::blocking downloader
│   │   ├── runtime.rs             # No-op placeholder (ort is transitive only)
│   │   ├── fastembed.rs           # FastembedEmbedder + FastembedReranker (Phase 3)
│   │   └── stub.rs                # Deterministic SHA-derived embedder + identity reranker
│   ├── presentation/              # Table + progress + colour + prompt wrappers (Phase 2)
│   │   ├── mod.rs
│   │   ├── tables.rs              # comfy-table helpers, NO_COLOR / non-TTY plain fallback
│   │   ├── progress.rs            # indicatif wrappers, auto-suppress on non-TTY stderr
│   │   ├── colour.rs              # owo-colors + NO_COLOR env + --no-color flag
│   │   └── prompt.rs              # inquire wrappers; refuse on non-TTY (NotATerminal)
│   │
│   ├── mcp/                       # MCP async island (Foundational F8, Phase 3 US1 filled)
│   │   ├── mod.rs                 # Sync entry point + async loop (Phase 3 US1; ~140 lines)
│   │   ├── server.rs              # rmcp::ServerHandler impl with #[tool_router] and #[tool_handler] macros (Phase 3 US1; ~90 lines)
│   │   ├── state.rs               # McpState: embedder, lazy reranker, scope, paths (Phase 3 US1; ~30 lines)
│   │   ├── tools/                 # MCP tool input/output schemas + handler bodies (Phase 3 US1)
│   │   │   ├── mod.rs             # Tool module aggregation (~15 lines)
│   │   │   ├── search_skills.rs   # search_skills tool: input, output, handle function (~150 lines)
│   │   │   └── get_skill.rs       # get_skill tool: input, output, handle function (~100 lines)
│   │   ├── runtime.rs             # Current-thread tokio runtime initialization (~50 lines)
│   │   ├── log.rs                 # Size-based rotation (FR-227) + JSON-lines registry (FR-226; ~100 lines)
│   │   └── preflight.rs           # Pre-flight validation (FR-110): schema gate, drift detect, SHA-256 verify, eager-load embedder (~120 lines)
│
├── tests/                         # Integration tests
│   ├── catalog_add.rs             # test: register a catalog
│   ├── catalog_remove.rs          # test: remove a catalog
│   ├── catalog_list.rs            # test: list catalogs
│   ├── catalog_update.rs          # test: refresh catalogs
│   ├── catalog_show.rs            # test: show catalog manifest
│   ├── catalog_remove_cascade.rs  # test: cascade-disable on removal (Phase 9; Phase 3 US3: extends with refcount coverage)
│   ├── exit_codes.rs              # test: verify all TomeError variants map to expected codes
│   ├── manifest_strictness.rs     # test: verify #[serde(deny_unknown_fields)]
│   ├── path_validation.rs         # test: plugin source path validation
│   ├── scrubbing.rs               # test: credential scrubbing rules
│   ├── atomicity.rs               # test: interruption-injection atomicity
│   ├── error_messages.rs          # test: error messages are user-friendly
│   ├── frontmatter.rs             # test: SKILL.md frontmatter parser (Phase 2)
│   ├── index_schema_bootstrap.rs  # test: fresh DB bootstrap, meta seeding, vec extension (Phase 2)
│   ├── index_lock.rs              # test: advisory lock contention (Phase 2)
│   ├── embedding_stub.rs          # test: stub embedder properties (Phase 2)
│   ├── model_download.rs          # test: model download + checksum validation (Phase 2)
│   ├── paths_phase2.rs            # test: Phase 2 path resolvers (Phase 2)
│   ├── plugin_enable.rs           # test: plugin enable flow (Phase 3)
│   ├── plugin_disable.rs          # test: plugin disable flow (Phase 5)
│   ├── plugin_repeated.rs         # test: repeated-state idempotency (Phase 5)
│   ├── plugin_list.rs             # test: plugin list (Phase 3)
│   ├── plugin_show.rs             # test: plugin show (Phase 3)
│   ├── plugin_interactive.rs      # test: bare `tome plugin` interactive browse via pty (Phase 4)
│   ├── query.rs                   # test: query (KNN + optional rerank) (Phase 3)
│   ├── models_download.rs         # test: models download (Phase 6)
│   ├── models_list.rs             # test: models list (Phase 6)
│   ├── models_remove.rs           # test: models remove (Phase 6)
│   ├── reindex.rs                 # test: reindex via library API with StubEmbedder (Phase 7)
│   ├── status.rs                  # test: health report (Phase 8)
│   ├── version_output.rs          # test: extended --version output (Phase 8)
│   ├── workspace_resolution.rs    # test: workspace scope resolution (Phase 3 US2)
│   ├── workspace_info.rs          # test: workspace info diagnostics (Phase 3 US2)
│   ├── workspace_init.rs          # test: atomic workspace init (Phase 3 US2)
│   ├── workspace_commands.rs      # test: cross-product scope isolation (Phase 3 US3)
│   ├── catalog_cache_refcount.rs  # test: reference-counted catalog cache (Phase 3 US3)
│   ├── sync_boundary.rs           # test: tokio import boundary enforcement (Foundational F8)
│   ├── schema_migrations.rs       # test: schema migrations framework (Foundational F7)
│   ├── concurrency.rs             # test: two-process index contention (Phase 2)
│   ├── catalog_update_reindex.rs  # test: cascade on catalog update (Phase 2/7)
│   ├── mcp_server.rs              # test: MCP server tool registration and descriptions (Phase 3 US1)
│   ├── common/                    # test: shared test fixtures + helpers
│   │   └── mod.rs                 # paths_for, fabricate_installed_model, etc.
│   └── fixtures/
│       ├── sample-catalog/        # Test catalog with valid manifest + plugins (Phase 1)
│       └── sample-plugin/         # Test plugin with skills (Phase 2)
│
├── Cargo.toml                     # Rust package manifest, dependencies, profiles
├── Cargo.lock                     # Locked dependency versions
├── .gitignore                     # Git exclusions
├── .githooks/                     # Versioned git hooks (fmt, clippy, typos, cog verify, cargo test)
├── README.md                      # Project overview and quick start
├── CONSTITUTION.md                # Project values and architectural constraints
├── PRDs/
│   └── phase-1.md                 # Phase 1 product requirements document
├── specs/
│   ├── 001-phase-1-foundations/
│   │   ├── spec.md                # Feature specification (WHAT)
│   │   ├── plan.md                # Implementation plan (WHO, WHEN, HOW)
│   │   ├── research.md            # Research notes (credential scrubbing rules, paths, etc.)
│   │   ├── data-model.md          # Data structures and JSON schemas
│   │   ├── contracts/             # Interface contracts
│   │   │   ├── catalog-manifest.schema.toml
│   │   │   ├── catalog-add.md
│   │   │   └── ...
│   │   └── quickstart.md          # Developer onboarding guide
│   └── 002-phase-2-plugins-index/
│       ├── spec.md                # Phase 2 feature specification
│       ├── plan.md                # Implementation plan
│       ├── research.md            # Research decisions (concurrency, schema migration, etc.)
│       ├── data-model.md          # Index schema, embeddings, drift
│       ├── contracts/             # Interface contracts
│       │   ├── index-schema.sql
│       │   ├── plugin-commands.md
│       │   ├── query.md
│       │   ├── models-commands.md
│       │   ├── reindex.md         # Explicit reindex CLI (Phase 7)
│       │   ├── catalog-extensions.md # Reindex cascade (Phase 7) + Remove cascade (Phase 9)
│       │   ├── status.md          # Health diagnostics (Phase 8)
│       │   ├── exit-codes.md
│       │   └── ...
│       ├── quickstart.md
│       └── retro/                 # Phase 2 retro notes (gotchas, patterns, next-time)
│
├── .sdd/                          # SDD (Specification-Driven Development) artefacts
│   └── codebase/
│       ├── ARCHITECTURE.md        # System design, patterns, data flow
│       ├── STRUCTURE.md           # Directory layout, module boundaries (this file)
│       ├── STACK.md               # Technology stack (generated by tech focus)
│       └── INTEGRATIONS.md        # External services, APIs (generated by tech focus)
│
└── .claude/                       # Claude Code project settings
    └── settings.json              # Allowlists, preferences
```

## Key Directories

### `src/` - Source Code

| Directory | Purpose | Public Interface |
|-----------|---------|-------------------|
| `src/main.rs` | Binary entry point; parses CLI, resolves scope (Phase 3 US2, Phase 3 US3), installs signal handler, dispatches, handles errors. Phase 8: pre-parse hook for `--version`. Phase 3 US1: skips logging/signals for MCP. Phase 3 US2: pre-dispatch workspace resolution. Phase 3 US3: all commands receive scope-resolved `ResolvedScope`. | — (entry point, not a module) |
| `src/lib.rs` | Library surface; aggregates `catalog`, `cli`, `commands`, `config`, `error`, `logging`, `output`, `paths`, `plugin`, `workspace`, `index`, `embedding`, `presentation`, `mcp`. Phase 3 US2: workspace now public. | Public for integration tests. |
| `src/cli.rs` | clap derive definitions for global flags (`--json`, `-v`/`-vv`) and subcommands. Phase 8: `StatusArgs` with `--verify` flag, `disable_version_flag = true`. Phase 3 US1: `McpArgs` added. Phase 3 US2: `GlobalScopeArgs` with `--workspace` / `--global` flags, `WorkspaceCommand` enum. Phase 3 US3: all commands routed through scope resolution. | `Cli`, `Command`, `CatalogCommand`, `ModelsCommand`, `PluginCommand`, `ReindexCommand`, `StatusArgs`, `McpArgs`, `WorkspaceArgs`, arg structs. |
| `src/error.rs` | Closed `TomeError` enum; exit code and category mapping; error variants. Foundational F7: adds `SchemaVersionTooNew` (73) and `SchemaMigrationFailed` (74). Phase 3 US1: adds `McpStartupFailed`, `McpProtocolIo`. Phase 3 US2: adds `WorkspaceNotFound` (71), `WorkspaceMalformed` (75), `WorkspaceConflict` (72). | `TomeError`, `ManifestInvalid`, `PluginState`, etc. (consumed by all). |
| `src/catalog/` | Catalog management: manifest parsing, Git operations, atomic registry persistence. Phase 3 US3: adds reference_count() to walk scope configs and return list of scopes referencing a catalog URL. | `CatalogManifest`, `Git`, `store::load/save/write_atomic`, `store::reference_count`. |
| `src/commands/` | Command handlers; implement `tome catalog/models/plugin/query/reindex/status/workspace/mcp <subcommand>`. Phase 3 US2: workspace command dispatcher. Phase 3 US3: all handlers receive `ResolvedScope` from pre-dispatch. | Per-subcommand `run(args, scope, mode)` functions; library entry points `reindex::run_with_deps()`, `status::assemble_report()`, `workspace::info::assemble()`, `workspace::init()`, `mcp::run(scope, paths)`. |
| `src/config.rs` | `Config` and `CatalogEntry` struct definitions. | `Config`, `CatalogEntry`. |
| `src/paths.rs` | XDG-aware path resolution and cache key computation. Scope-parametrized accessors (Phase 3 US2 deferred to Phase 10 for general refactor). | `Paths`, `Paths::resolve()`, `Paths::cache_dir_for()`, `Paths::model_path()`, `Paths::config_file_for(&Scope)`, `Paths::index_db_for(&Scope)`. |
| `src/logging.rs` | Initialize `tracing-subscriber` (stderr-only, orthogonal to `--json`). | `Verbosity`, `init()`. |
| `src/output.rs` | Format output as human text or JSON; TTY detection. | `Mode`, `write_json()`, `write_error()`, `stdout_is_tty()`. |
| `src/plugin/` | Plugin metadata parsers, lifecycle orchestrator (enable/disable/reindex/cascade). | `PluginId`, `PluginRecord`, `PluginStatus`, `lifecycle::enable/disable/reindex_plugin/cascade_disable_for_catalog`, `lifecycle::auto_disable_orphan`, `lifecycle::resolve_plugin_dir`. |
| `src/workspace/` | Workspace scope resolution, diagnostics, initialization (Phase 3 US2). | `Scope`, `ResolvedScope`, `ScopeSource`, `ScopeKind`, `WorkspaceInfo`, `ModelIdentity`, `InitOutcome`. |
| `src/index/` | SQLite skills DB, KNN search, drift detection, forward-only migrations, atomic mutations. | `open()`, `acquire_lock()`, `enable_plugin_atomic()`, `reindex_plugin_atomic()`, `delete_by_plugin()`, `knn()`, `migrations::apply_pending()`, `MetaSeed`. |
| `src/embedding/` | Model registry, download, embedder/reranker traits. | `Embedder`, `Reranker`, `Scored`, `FastembedEmbedder`, `FastembedReranker`, `MODEL_REGISTRY`. |
| `src/presentation/` | Table, progress, colour, prompt wrappers. | `tables::*`, `progress::*`, `colour::*`, `prompt::*`. |
| `src/mcp/` | Async server boundary, stdio transport handler, tool dispatch, preflight validation, log rotation (Phase 3 US1 filled). | `run(scope, paths)` (async entry), `server::Server`, `state::McpState`, `tools::search_skills`, `tools::get_skill`. |

### `tests/` - Integration Tests

| File | Purpose | Tests |
|------|---------|-------|
| `tests/catalog_add.rs` | Test `tome catalog add` with various source formats, error cases. | Happy path, already-exists, manifest errors. |
| `tests/catalog_remove.rs` | Test `tome catalog remove` with confirmation, `--force`. | Interactive, non-TTY, confirmed. |
| `tests/catalog_remove_cascade.rs` | Test `tome catalog remove` cascade semantics (Phase 9); extends with refcount coverage (Phase 3 US3). | Refuse when enabled plugins exist, cascade on `--force`, no-enabled case, cache refcount behaviour. |
| `tests/catalog_list.rs` | Test `tome catalog list` in human and `--json` modes. | Empty, single, multiple catalogs. |
| `tests/catalog_update.rs` | Test `tome catalog update` (single, all, pinned commit). | Happy path, first-failure stop, pinned ref. |
| `tests/catalog_show.rs` | Test `tome catalog show` manifest contents. | Human and JSON output. |
| `tests/exit_codes.rs` | Verify all `TomeError` variants map to expected exit codes (exhaustive). | One assertion per variant. |
| `tests/manifest_strictness.rs` | Verify `#[serde(deny_unknown_fields)]` is applied correctly. | Unknown field rejection. |
| `tests/path_validation.rs` | Verify plugin source paths are validated (no `..`, no absolute, no escape). | Negative cases. |
| `tests/scrubbing.rs` | Verify credential scrubbing rules (R-8) work correctly. | URL login, SSH login, tokens, long hex, AWS signed URLs, reqwest errors. |
| `tests/atomicity.rs` | Verify atomic writes survive interruption injection. | Write interrupted mid-operation. |
| `tests/error_messages.rs` | Verify error messages are clear and actionable. | Human-readable output. |
| `tests/frontmatter.rs` | Table-driven matrix over SKILL.md parser (Phase 2). | Delimiter failure, YAML-body failure, FR-011/FR-012 fallbacks. |
| `tests/index_schema_bootstrap.rs` | Fresh DB bootstrap, meta seeding, vec extension (Phase 2). | Bootstrap idempotency, schema-too-new refusal. |
| `tests/index_lock.rs` | Advisory lock contention + release (Phase 2). | Lock acquire, pre-existing lockfile reuse. |
| `tests/embedding_stub.rs` | Stub embedder determinism, 384-dim, L2 normalisation (Phase 2). | Distinguishability, dimension check. |
| `tests/model_download.rs` | Model download + checksum validation (Phase 2). | Happy path, checksum mismatch, HTTP 404, placeholder-checksum refusal. |
| `tests/paths_phase2.rs` | Phase 2 path resolvers (Phase 2). | index_db, index_lock, models_dir, model_path resolution. |
| `tests/plugin_enable.rs` | Plugin enable flow (Phase 3). | Happy path, idempotency rejection, frontmatter errors, fallback warnings. |
| `tests/plugin_disable.rs` | Plugin disable flow via CLI (Phase 5). | Happy path, --force flag, non-TTY refusal, confirm prompt, skill records retained. |
| `tests/plugin_repeated.rs` | Repeated-state idempotency for enable/disable (Phase 5). | Re-enable exit 21, re-disable exit 21. |
| `tests/plugin_list.rs` | Plugin list (Phase 3). | Single/multiple catalogs, filtering, human/JSON output. |
| `tests/plugin_show.rs` | Plugin show (Phase 3). | Metadata display, status, component counts, index aggregate. |
| `tests/plugin_interactive.rs` | Interactive browse flow via pty harness (Phase 4). | Catalog selection, plugin selection, enable/disable actions, Esc/Ctrl-C, non-TTY refusal. |
| `tests/query.rs` | Query KNN + optional rerank (Phase 3). | Happy path, filtering, reranking, threshold filtering. |
| `tests/models_download.rs` | Models download (Phase 6). | Happy path, --force flag, spinner, human/JSON output. |
| `tests/models_list.rs` | Models list (Phase 6). | Cheap state check, --verify rehash, ModelState classification. |
| `tests/models_remove.rs` | Models remove (Phase 6). | Happy path, --force flag, non-TTY refusal, usage check, delete sequence. |
| `tests/reindex.rs` | Reindex via library API with StubEmbedder (Phase 7). | Scope resolution (All / Catalog / Plugin), added/modified/removed/unchanged counts, force flag, orphan handling. |
| `tests/status.rs` | Health report via library API (Phase 8). | Embedder/reranker/index state, drift detection, overall health classification. |
| `tests/version_output.rs` | Extended --version output (Phase 8). | Model identities in plain text and JSON forms. |
| `tests/workspace_resolution.rs` | Workspace scope resolution (Phase 3 US2). | --workspace flag, --global flag, env var, CWD walk, fallback, conflict detection. |
| `tests/workspace_info.rs` | Workspace info diagnostics (Phase 3 US2). | Global scope, workspace scope, bootstrap-not-yet, catalog/plugin/skill counts. |
| `tests/workspace_init.rs` | Atomic workspace init (Phase 3 US2). | Happy path, --inherit-global flag, --force flag, atomic semantics, rollback. |
| `tests/workspace_commands.rs` | Cross-product scope isolation (Phase 3 US3). | Commands execute on correct scope (global vs workspace), catalog add/remove refcount, independent state. |
| `tests/catalog_cache_refcount.rs` | Reference-counted catalog cache (Phase 3 US3). | Reuse when URL cached elsewhere, refcount walk, cascade on `remove --force`, orphan cleanup. |
| `tests/sync_boundary.rs` | Structural tokio import boundary (Foundational F8). | Scans src/ (except src/mcp/), fails on any `tokio` import outside mcp/. |
| `tests/schema_migrations.rs` | Schema migrations framework (Foundational F7). | Forward-only boundaries, synthetic fixture e2e test, "no migration registered" guard. |
| `tests/concurrency.rs` | Two-process index contention (Phase 2). | Concurrent enable/list, lockfile contention. |
| `tests/catalog_update_reindex.rs` | Cascade on catalog update (Phase 2/7). | Skills marked stale when catalog ref changes; orphan cascade via auto_disable_orphan. |
| `tests/mcp_server.rs` | MCP server tool registration (Phase 3 US1). | Tool list, tool descriptions via rmcp router, input schemas. |
| `tests/common/mod.rs` | Shared fixtures (Phase 6/7). | `paths_for`, `fabricate_installed_model`, `fabricate_all_installed_models`. |

## Module Boundaries

### Catalog Module: `src/catalog/`

The catalog module is fully self-contained and can be tested in isolation.

```
src/catalog/
├── mod.rs           # Aggregates git, manifest, store
├── git.rs           # Git shell-outs + credential scrubbing + signal handling
├── manifest.rs      # TOML parsing (strict tome-catalog.toml) + JSON parsing (lenient plugin.json)
└── store.rs         # Atomic read/write of config.toml + reference_count() for shared cache (Phase 3 US3)
```

**Responsibility**: Manage the lifecycle of a catalog (fetch, parse, validate, persist, refresh). Reference-count shared cache directories across scopes.

**Public Interface**:
- `git::Git` — facade for git operations.
- `git::install_signal_handler()`, `git::was_cancelled()` — signal handling.
- `manifest::CatalogManifest::parse_and_validate()` — strict parsing and validation.
- `manifest::read_catalog_manifest()` — lenient read for plugin/list/show.
- `store::load()`, `store::save()`, `store::write_atomic()` — atomic persistence.
- `store::reference_count(url, paths) -> Vec<Scope>` — walk scope configs, return referencing scopes (Phase 3 US3).

**What It Cannot Do**:
- Know about CLI argument structures (those live in `cli.rs` and `commands/catalog/`).
- Format output for the user (that's `output.rs` and `presentation/`'s job).
- Initialize logging (that's `logging.rs`'s job).

### Commands Module: `src/commands/`

Each subcommand lives in its own file. All subcommands are dispatched from their respective `mod.rs` files.

```
src/commands/
├── mod.rs           # Top-level dispatcher (catalog vs models vs plugin vs query vs reindex vs status vs workspace vs mcp)
├── catalog/
│   ├── mod.rs       # Dispatcher
│   ├── add.rs       # Register a catalog; reuse cached clone if URL already cached (Phase 3 US3)
│   ├── remove.rs    # Unregister (Phase 9: reads enabled plugins, cascades on --force; Phase 3 US3: reference-count cache)
│   ├── list.rs      # Show all catalogs
│   ├── update.rs    # Refresh (Phase 7: lazy embedder, per-plugin reindex, auto-disable orphans)
│   ├── show.rs      # Show one catalog's manifest
│   └── source.rs    # URL resolution helper
├── models/          # (Phase 6) Explicit model management
│   ├── mod.rs       # Dispatcher + ModelState enum
│   ├── download.rs  # Download missing models (iterate registry, skip if ok unless --force)
│   ├── list.rs      # List all models with state (Ok / Missing / Corrupt / ChecksumMismatched)
│   └── remove.rs    # Remove a model (usage check, confirm, delete manifest + dir)
├── plugin/          # (Phase 3–5)
│   ├── mod.rs       # Dispatcher + shared helpers (model checking, index opening)
│   ├── enable.rs    # Enable a plugin
│   ├── disable.rs   # Disable a plugin (Phase 5; ~108 lines)
│   ├── list.rs      # List plugins
│   ├── show.rs      # Show one plugin
│   └── interactive.rs # Bare `tome plugin` interactive browse (Phase 4; ~515 lines; Phase 3 US3: receives scope-resolved ResolvedScope)
├── workspace/       # (Phase 3 US2) Workspace scope management
│   ├── mod.rs       # Dispatcher (info / init subcommands)
│   ├── info.rs      # `tome workspace info` — read-only scope diagnostics (~40 lines for emit layer)
│   └── init.rs      # `tome workspace init` — atomic .tome/ creation (~20 lines for emit layer)
├── query.rs         # (Phase 3) Query/search
├── reindex.rs       # (Phase 7) Re-embedding (scope parsing, lazy embedder, aggregate output; ~280 lines)
├── status.rs        # (Phase 8) Health diagnostics (read-only; ~330 lines)
└── mcp.rs           # (Phase 3 US1) MCP server dispatcher (~20 lines)
```

**Responsibility**: Translate CLI arguments into library operations; orchestrate error handling and output formatting. All handlers receive `ResolvedScope` from pre-dispatch (Phase 3 US3).

**Signature Pattern** (all subcommands):
```rust
pub fn run(args: SomeArgs, scope: &ResolvedScope, mode: output::Mode) -> Result<(), TomeError>
```

**Interactive Pattern** (Phase 4):
```rust
pub fn run_interactive(scope: &ResolvedScope, mode: output::Mode) -> Result<(), TomeError>
```

**MCP Pattern** (Phase 3 US1):
```rust
pub fn run(_args: McpArgs, scope: &ResolvedScope, _mode: Mode) -> Result<(), TomeError>
```

**Workspace Pattern** (Phase 3 US2):
```rust
pub fn run(cmd: WorkspaceCommand, scope: &ResolvedScope, mode: Mode) -> Result<(), TomeError>
```

**Library Test Entry Point** (Phase 7, `reindex.rs`):
```rust
pub fn run_with_deps(
    scope: Scope,
    plugins: &[PluginId],
    deps: &LifecycleDeps<'_>,
    force: bool,
    mode: Mode,
) -> Result<ReindexAggregate, TomeError>
```

**Library Entry Point** (Phase 8, `status.rs`):
```rust
pub fn assemble_report(scope: &ResolvedScope, paths: &Paths, verify: bool) -> Result<StatusReport, TomeError>
pub fn print_version(json: bool) -> Result<(), TomeError>
```

**Library Entry Points** (Phase 3 US2, `workspace/`):
```rust
pub fn assemble(scope: &ResolvedScope, paths: &Paths) -> Result<WorkspaceInfo, TomeError>
pub fn init(target: &Path, inherit_global: bool, force: bool, paths: &Paths) -> Result<InitOutcome, TomeError>
```

**What It Cannot Do**:
- Directly access `logging` (orthogonal).
- Modify global state (all state mutations go through library functions).
- Know about internal index or embedding details (those are opaque through trait interfaces).

### Plugin Module: `src/plugin/`

```
src/plugin/
├── mod.rs            # PluginRecord, PluginStatus, re-exports
├── identity.rs       # PluginId: <catalog>/<plugin> + FromStr
├── manifest.rs       # plugin.json (lenient parsing)
├── frontmatter.rs    # SKILL.md YAML header (lenient + fallbacks)
├── components.rs     # ComponentCounts walk
└── lifecycle.rs      # enable / disable / reindex_plugin / cascade_disable_for_catalog orchestrator + resolve_plugin_dir + auto_disable_orphan (Phase 3/7/9)
```

**Responsibility** (library-shaped, no CLI):
- Read-only parsing of plugin metadata (manifest.json, SKILL.md frontmatter).
- Orchestrate enable/disable/reindex/cascade: compose index + embedding + manifest parsing into atomic operations.
- Resolve plugin directories (manifest-first, with fallback).
- De-index orphaned plugins on catalog refresh.
- Cascade-disable all enabled plugins for a catalog before removal.

**Public Interface**:
- `PluginId`, `PluginRecord`, `PluginStatus` — types.
- `lifecycle::enable(id, deps) -> Result<EnableOutcome>` — full enable flow.
- `lifecycle::disable(id, paths, config, seeds) -> Result<DisableOutcome>` — full disable flow.
- `lifecycle::reindex_plugin(id, deps, force) -> Result<ReindexOutcome>` — full reindex flow (Phase 7).
- `lifecycle::auto_disable_orphan(id, deps) -> Result<u32>` — de-index orphaned plugin (Phase 7).
- `lifecycle::cascade_disable_for_catalog(paths, catalog, plugins, embedder_seed, reranker_seed) -> Result<u32>` — cascade-disable per catalog (Phase 9).
- `lifecycle::resolve_plugin_dir(id, config) -> Result<PathBuf>` — directory resolution.
- `manifest::parse_plugin_manifest()`, `frontmatter::parse_skill_frontmatter()` — parsers.
- `components::count_components()` — component walk.

**What It Cannot Do**:
- Know about CLI argument structures (those live in `commands/plugin/` or `commands/catalog/`).
- Format output (that's `commands/plugin/` and `presentation/`'s job).
- Prompt the user for downloads (that's `commands/plugin/enable.rs`'s responsibility; the library receives `allow_model_download` boolean).
- Orchestrate interactive browse (that's `commands/plugin/interactive.rs`'s responsibility).

### Workspace Module: `src/workspace/` (Phase 3 US2)

```
src/workspace/
├── mod.rs            # Re-exports (Scope, ResolvedScope, WorkspaceInfo, InitOutcome)
├── scope.rs          # Scope enum (Global / Workspace), ResolvedScope, ScopeSource, ScopeKind types
├── resolution.rs     # resolve(workspace_flag, global_flag, env_opt, cwd) → ResolvedScope
├── info.rs           # WorkspaceInfo + ModelIdentity types (emit-only); assemble() library function
├── init.rs           # InitOutcome type; init() library function (atomic .tome/ creation)
└── inventory.rs      # Optional workspace registry (${state_dir}/workspaces.txt)
```

**Responsibility** (library-shaped, no CLI):
- Determine active scope (global vs workspace) from CLI flags, env var, CWD walk.
- Read-only workspace diagnostics (catalog/plugin/skill counts, schema version, embedder identity).
- Atomic workspace initialization with optional catalog inheritance.
- Optional registration in workspace inventory.

**Public Interface**:
- `Scope`, `ResolvedScope`, `ScopeSource`, `ScopeKind` — scope types.
- `resolution::resolve(workspace_opt, global_opt, env_opt, cwd) -> Result<ResolvedScope, TomeError>` — scope resolution.
- `info::assemble(scope, paths) -> Result<WorkspaceInfo, TomeError>` — read-only diagnostics.
- `init::init(target, inherit_global, force, paths) -> Result<InitOutcome, TomeError>` — atomic workspace init.
- `WorkspaceInfo`, `InitOutcome` — emit-only serializable types.

**What It Cannot Do**:
- Know about CLI argument structures (those live in `cli.rs` and `commands/workspace/`).
- Format output (that's `commands/workspace/` and `presentation/`'s job).
- Manage global logging (that's `logging.rs`'s job).

### Index Module: `src/index/`

```
src/index/
├── mod.rs              # Re-exports (Phase 7: exports reindex_plugin_atomic; Phase 9: exports delete_by_plugin)
├── schema.rs           # CREATE TABLE + MetaSeed
├── migrations.rs       # Forward-only migration framework (Foundational F7; ~120 lines)
├── vec_ext.rs          # sqlite-vec extension loader
├── db.rs               # open() + PRAGMA setup + bootstrap/migrate
├── lock.rs             # Advisory write lock
├── meta.rs             # Metadata read/write + drift detection
├── integrity.rs        # PRAGMA integrity_check
├── skills.rs           # CRUD + content_hash + enable_plugin_atomic + reindex_plugin_atomic + delete_by_plugin (Phase 7/9; ~510 lines)
└── query.rs            # KNN search + filters
```

**Responsibility** (library-shaped, no CLI):
- Maintain SQLite skills DB with vector embeddings.
- Support atomic multi-skill inserts (enable).
- Support atomic per-plugin reindex with smart re-embedding (Phase 7).
- Support atomic per-plugin row deletion (cascade).
- Support atomic enable-flag updates (disable).
- Provide KNN search over enabled skills.
- Detect embedder/reranker drift.
- Manage advisory locks for write operations.
- Enforce forward-only schema evolution (Foundational F7).

**Public Interface**:
- `open(path, seeds) -> Result<Connection>` — open or bootstrap.
- `acquire_lock(path) -> Result<Lock>` — write lock (filesystem level, OS FD-based).
- `enable_plugin_atomic(&mut conn, pending, embed_fn) -> Result<EnableSummary>` — insert skills under one transaction.
- `reindex_plugin_atomic(&mut conn, catalog, plugin, pending, force, embed_fn) -> Result<ReindexSummary>` — diff on-disk vs index, re-embed modified/added, delete removed (Phase 7).
- `delete_by_plugin(conn, catalog, plugin) -> Result<u32>` — delete all skill rows for a plugin pair, return count (Phase 9).
- `mark_all_disabled_for_plugin(conn, catalog, plugin) -> Result<u32>` — flip enabled flag.
- `query::knn(conn, vec, k, filters) -> Result<Vec<Candidate>>` — KNN search.
- `meta::detect_drift(conn) -> Result<DriftStatus>` — drift detection.
- `migrations::apply_pending(conn, current, target) -> Result<u32>` — forward-only migration application (Foundational F7).

**What It Cannot Do**:
- Embed text (that's the embedder's job; it receives an `embed_fn` closure).
- Format output (that's `commands/`'s job).
- Manage CLI prompts (that's `presentation/`'s job).

### Embedding Module: `src/embedding/`

```
src/embedding/
├── mod.rs              # Embedder + Reranker traits, Scored
├── registry.rs         # MODEL_REGISTRY const + ModelEntry + ModelManifest
├── download.rs         # Atomic reqwest::blocking download + SIGINT awareness + sha256_file (Phase 6)
├── runtime.rs          # Placeholder (ort transitive)
├── fastembed.rs        # FastembedEmbedder + FastembedReranker
└── stub.rs             # StubEmbedder + identity reranker (test-only by default)
```

**Responsibility** (library-shaped, no CLI):
- Define `Embedder` and `Reranker` trait interfaces.
- Implement fastembed-backed wrappers (`FastembedEmbedder`, `FastembedReranker`).
- Provide deterministic test double (`StubEmbedder`).
- Manage model registry, download, checksum validation.
- **Phase 6 addition**: Provide streaming `sha256_file()` helper for `models list --verify`.
- **Phase 7 addition**: Registry seeding for reindex (no new entry points).

**Public Interface**:
- `Embedder { fn embed(&self, text: &str) -> Result<Vec<f32>>; }` — trait.
- `Reranker { fn rerank(&self, text: &str, candidates: Vec<Candidate>) -> Result<Vec<Scored>>; }` — trait.
- `Scored { score: f32, candidate: Candidate }` — result type.
- `FastembedEmbedder::load(entry, dir) -> Result<Self>` — load model from disk.
- `FastembedReranker::load(entry, dir) -> Result<Self>` — load reranker from disk.
- `MODEL_REGISTRY` — array of `ModelEntry` (embedder + reranker pinned versions).
- `download::download_model(entry, dir) -> Result<()>` — atomic download with SIGINT awareness.
- `download::sha256_file(path) -> Result<String, TomeError>` — streaming SHA-256 for verification (Phase 6).

**What It Cannot Do**:
- Know about CLI arguments or prompts (that's `commands/plugin/` or `commands/models/`'s job).
- Manage paths (that's `paths.rs`'s job; commands pass the resolved directory).

### MCP Module: `src/mcp/` (Phase 3 US1 filled)

```
src/mcp/
├── mod.rs              # Sync entry point + async loop (Phase 3 US1; ~140 lines)
├── server.rs           # rmcp::ServerHandler impl with #[tool_router] and #[tool_handler] macros (Phase 3 US1; ~90 lines)
├── state.rs            # McpState: embedder, lazy reranker, scope, paths (Phase 3 US1; ~30 lines)
├── tools/              # MCP tool input/output schemas + handler bodies (Phase 3 US1)
│   ├── mod.rs          # Tool module aggregation (~15 lines)
│   ├── search_skills.rs # search_skills tool: input, output, handle function (~150 lines)
│   └── get_skill.rs    # get_skill tool: input, output, handle function (~100 lines)
├── runtime.rs          # Current-thread tokio runtime (~50 lines)
├── log.rs              # Size-based rotation + JSON-lines registry (~100 lines)
└── preflight.rs        # Pre-flight validation (~120 lines)
```

**Responsibility** (async island, Phase 3 US1):
- Provide structural boundary for MCP server logic.
- Scope `tokio` exclusively to this module (constitution anticipated forcing function).
- Manage stdio transport via `rmcp::serve_server`.
- Register and dispatch two MCP tools: `search_skills` and `get_skill`.
- Validate pre-server conditions (schema gate, drift, SHA-256, embedder load).
- Manage MCP-specific logging (size-based rotation, JSON-lines, error-only stderr layer).
- Lazy-load reranker on first `search_skills` call per FR-109.

**Public Interface**:
- `mod::run(scope, paths) -> Result<(), TomeError>` — async sync entry point (Phase 3 US1).
- `server::Server::new(state) -> Self` — server constructor.
- `state::McpState` — shared state carrying embedder, lazy reranker, scope, paths (Phase 3 US1).
- `tools::search_skills::handle(state, input) -> Result<Output, McpError>` — search tool handler (Phase 3 US1).
- `tools::get_skill::handle(state, input) -> Result<Output, McpError>` — fetch tool handler (Phase 3 US1).
- `preflight::PreflightReport` — library-shaped validation result (phase 3 will expand).

**Design Invariants**:
- **Async Island**: Only files under `src/mcp/` use `tokio`. All other modules remain sync.
- **Structural Test**: `tests/sync_boundary.rs` enforces — scans src/ (except src/mcp/), fails on any `tokio` import outside mcp/.
- **No Cross-Module Dependencies**: `mcp/` does NOT import from `commands/` (except `commands::query` for pipeline reuse). MCP reads from library-shaped functions only.
- **Preflight Defensive**: Errors during validation surface as exit codes; does not crash the server.
- **Lazy Reranker**: `tokio::sync::OnceCell` enables async-friendly lazy initialization on first tool call.
- **File Logging Only**: stdout is the MCP protocol channel (FR-221), stderr is for fatal startup errors only (FR-222), diagnostics go to `${XDG_STATE_HOME}/tome/mcp.log`.

**What It Cannot Do**:
- Depend on CLI modules (commands input parsing, interactive prompts).
- Format output for users (that's CLI or MCP spec's job).
- Manage global logging (orthogonal to CLI's tracing; mcp/log.rs sets up its own subscriber).

## Where to Add New Code

| If you're adding... | Put it in... | Example |
|---------------------|--------------|---------|
| New catalog subcommand | `src/commands/catalog/{name}.rs` + add to dispatcher in `mod.rs` | `src/commands/catalog/verify.rs` (verify manifest syntax) |
| New models subcommand | `src/commands/models/{name}.rs` + add to dispatcher in `mod.rs` | `src/commands/models/verify.rs` (verify model integrity) |
| New plugin subcommand | `src/commands/plugin/{name}.rs` + add to dispatcher in `mod.rs` | `src/commands/plugin/verify.rs` (verify plugin integrity) |
| New workspace subcommand | `src/commands/workspace/{name}.rs` + add to dispatcher in `mod.rs` | `src/commands/workspace/show.rs` (show workspace contents) |
| New top-level command | `src/commands/{name}.rs` + add to dispatcher in `src/commands/mod.rs` | `src/commands/preflight.rs` (pre-flight checks; Phase 9 FR-056) |
| New CLI global flag | `src/cli.rs` in `struct Cli` | `#[arg(long, global = true)] pub verify: bool,` |
| New error type | `src/error.rs` in `TomeError` enum | Add variant + exit code + test in `tests/exit_codes.rs` |
| New manifest validation rule | `src/catalog/manifest.rs::validate_semantic()` | Validate plugin version semver |
| New Git operation | `src/catalog/git.rs` as a `Git` method | `pub fn fetch_tags(&self, url: &str) -> Result<Vec<String>>` |
| New plugin metadata field | `src/plugin/manifest.rs` + `frontmatter.rs` | Add `homepage` URL to `PluginManifest` |
| New lifecycle step | `src/plugin/lifecycle.rs` (private fn inside `enable`/`disable`/`reindex_plugin`/`cascade_disable_for_catalog`) | Add model pre-validation before lock |
| New workspace scope logic | `src/workspace/resolution.rs` or `src/workspace/scope.rs` | Add workspace discovery strategy |
| New index operation | `src/index/skills.rs` | `pub fn update_skill_embedding()` for selective re-embedding |
| New KNN filter | `src/index/query.rs` + `QueryFilters` | Add `--min-version` filter |
| New model kind | `src/embedding/mod.rs` (`ModelKind` enum) + `registry.rs` | Add reranker v2 variant |
| New interactive sub-flow | `src/commands/plugin/interactive.rs` (extend existing loop levels) | Add a cascade to `plugin_loop` for plugin tags/categories |
| New reindex scope | `src/commands/reindex.rs` (`Scope` enum) | Add `Org(String)` for organization-scoped reindex |
| New health check | `src/commands/status.rs` (extend `OverallHealth`, `classify_*` helpers) | Add memory/disk usage thresholds |
| New schema migration | `src/index/migrations.rs` (`MIGRATIONS` array) + `src/index/schema.rs` | Register migration step with version tag |
| New MCP server endpoint | `src/mcp/tools/{name}.rs` + register in `mcp::server` (Phase 3 US1 onwards) | New `#[tool]`-decorated method in ServerHandler impl; route via handler function |
| Test for a command | `tests/{command_area}_{action}.rs` | `tests/models_download.rs` |
| Test for error scenario | `tests/error_messages.rs` or new file | Document the error text clearly |
| Test for interactive flow | `tests/plugin_interactive.rs` + `rexpect` pty harness | Additional test cases for specific user paths |
| Test for models command | `tests/models_{download,list,remove}.rs` | Test `--verify`, `--force`, on-disk state handling |
| Test for reindex scope | `tests/reindex.rs` + library API | Test All / Catalog / Plugin scope variants with StubEmbedder |
| Test for status report | `tests/status.rs` + library API | Test health classification, drift detection, overall health |
| Test for cascade behavior | `tests/catalog_remove_cascade.rs` + library API | Test refuse/cascade/no-enabled cases with StubEmbedder |
| Test for workspace feature | `tests/workspace_{resolution,info,init}.rs` + library API | Test scope resolution, info output, atomic init |
| Test for schema migration | `tests/schema_migrations.rs` + synthetic fixture | Register temporary migration via `MIGRATIONS_OVERRIDE` |
| Test for MCP tool | `tests/mcp_server.rs` or per-tool file | Test tool input validation, output schema, handler logic |
| Test shared helper | `tests/common/mod.rs` | Add `fabricate_*` factory functions |
| Test for scope isolation | `tests/workspace_commands.rs` or new file (Phase 3 US3) | Test catalog/plugin/index isolation across scopes |
| Test for catalog cache | `tests/catalog_cache_refcount.rs` (Phase 3 US3) | Test reuse, refcount, cleanup, cascade composition |

## Naming Conventions

| Category | Convention | Examples |
|----------|-----------|----------|
| **Struct/Enum** | PascalCase | `CatalogManifest`, `CatalogEntry`, `TomeError`, `PluginId`, `EnableOutcome`, `DisableOutcome`, `ReindexOutcome`, `Candidate`, `ModelState`, `StatusReport`, `OverallHealth`, `McpState`, `SkillMatch`, `Scope`, `ResolvedScope`, `ScopeKind`, `WorkspaceInfo`, `InitOutcome` |
| **Trait** | PascalCase | `Embedder`, `Reranker`, `Git`, `ServerHandler` |
| **Function/Method** | snake_case | `parse_and_validate()`, `install_signal_handler()`, `enable_plugin_atomic()`, `reindex_plugin_atomic()`, `resolve_plugin_dir()`, `cascade_disable_for_catalog()`, `assemble_report()`, `apply_pending()`, `search_skills()`, `get_skill()`, `resolve()`, `reference_count()` |
| **Constant** | SCREAMING_SNAKE_CASE | `MODEL_REGISTRY`, `SCHEMA_URI`, `HANDLER_INSTALLED`, `MIGRATIONS` |
| **Module** | snake_case directory names | `src/catalog/`, `src/commands/`, `src/plugin/`, `src/workspace/`, `src/index/`, `src/mcp/` |
| **Test** | `#[test]` with descriptive name | `#[test] fn unknown_field_is_rejected()` |
| **Integration test file** | Matches the feature being tested | `tests/plugin_enable.rs` tests `tome plugin enable`; `tests/workspace_info.rs` tests `tome workspace info`; `tests/reindex.rs` tests `tome reindex`; `tests/status.rs` tests `tome status`; `tests/catalog_cache_refcount.rs` tests reference-counted cache (Phase 3 US3) |
| **Interactive loop level** | Private enum in interactive.rs | `LoopExit::Continue`, `LoopExit::Back`, `LoopExit::Quit` |
| **Reindex scope** | PublicEnum in commands/reindex.rs | `Scope::All`, `Scope::Catalog`, `Scope::Plugin` |
| **Workspace scope** | PublicEnum in workspace/scope.rs | `Scope::Global`, `Scope::Workspace`, `ScopeSource::Flag`, `ScopeSource::Env`, `ScopeSource::CwdWalk` |
| **Model state classification** | PublicEnum in commands/models/mod.rs | `ModelState::Ok`, `ModelState::Missing`, `ModelState::Corrupt`, `ModelState::ChecksumMismatched` |
| **Health classification** | PublicEnum in commands/status.rs | `OverallHealth::Ok`, `OverallHealth::Degraded`, `OverallHealth::Unhealthy` |
| **Migration application** | Function in index/migrations.rs | `apply_pending(conn, current, target)` returns Result with exit codes 51/73/74 |
| **MCP tool input/output** | Per-tool module | `tools::search_skills::{Input, Output}`, `tools::get_skill::{Input, Output}` |

## Entry Points

| File | Purpose |
|------|---------|
| `src/main.rs` | Binary entry; resolves scope (Phase 3 US2/US3), parses CLI and dispatches to handlers. Phase 8: pre-parse hook for `--version`. Phase 3 US1: skips logging/signals for MCP. Phase 3 US3: all commands routed through scope resolution. |
| `src/lib.rs` | Library aggregation; exposes public modules for tests. |
| `tests/catalog_add.rs` | Integration tests directly import from `tome::*` and test the library. |
| `tests/reindex.rs` | Library-API tests via `commands::reindex::run_with_deps()` with `StubEmbedder`. |
| `tests/status.rs` | Library-API tests via `commands::status::assemble_report()`. |
| `tests/workspace_info.rs` | Library-API tests via `workspace::info::assemble()` (Phase 3 US2). |
| `tests/workspace_init.rs` | Library-API tests via `workspace::init::init()` (Phase 3 US2). |
| `tests/catalog_remove_cascade.rs` | Library-API tests for cascade via `lifecycle::cascade_disable_for_catalog()` with `StubEmbedder` (Phase 9). |
| `tests/mcp_server.rs` | MCP server tool registration and descriptions (Phase 3 US1). |
| `tests/workspace_commands.rs` | Cross-product scope isolation (Phase 3 US3). |
| `tests/catalog_cache_refcount.rs` | Reference-counted catalog cache (Phase 3 US3). |

## Module Stability Guarantees

- **Stable Public API**: `catalog::git`, `catalog::manifest`, `catalog::store` (including `reference_count`), `config`, `error`, `output`, `paths`, `cli`, `plugin`, `workspace` (Phase 3 US2), `index` (including `migrations::apply_pending`), `embedding`, `presentation`, `commands::status::assemble_report`, `commands::reindex::run_with_deps`, `mcp` (Phase 3 US1).
- **Internal**: Submodule organization within `commands/` is flexible; subcommand `run()` signatures (and `run_interactive()` for bare `plugin`, `run_with_deps()` for `reindex` tests, `assemble_report()` for `status` tests, `run(scope, paths)` for `mcp`) are the public contract.
- **MCP Experimental**: `mcp::run()`, `mcp::server::Server`, `mcp::state::McpState`, `mcp::tools::*` are library-shaped but Phase 3 US1 lands the initial filling (Phase 10 will expand with new tools).

## Generated Files

No files in Phase 1–9 or Foundational are auto-generated.

---

## What Does NOT Belong Here

- Architecture patterns → ARCHITECTURE.md
- Technology choices → STACK.md
- Code style rules → CONVENTIONS.md
- Test patterns → TESTING.md

---

## Phase 3 User Story 3 additions — Per-Command Scope Honouring & Reference-Counted Catalog Cache

Phase 3 US3 gates all commands through workspace scope resolution (Phase 3 US2 was the foundation; US3 ensures
every command receives `ResolvedScope` from pre-dispatch). Catalog cache directories are now reference-counted
across scopes: `store::reference_count(url, paths)` walks scope configs (global + all workspaces via optional
`workspaces.txt` registry) to enumerate all references. `tome catalog add` reuses existing cache if URL is already
cached elsewhere (cheap manifest check, skip git clone). `tome catalog remove` calls `reference_count()` AFTER
config deletion; only deletes cache dir when refcount reaches zero. TOCTOU-benign: concurrent removes race safely
(one winner, other no-ops; dangling cache is recoverable via `tome catalog update`). Pattern is reusable for
any future shared-on-disk resource.

---

*This document shows WHERE code lives. Update when directory structure changes.*

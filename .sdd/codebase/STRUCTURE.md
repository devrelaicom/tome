# Project Structure

> **Purpose**: Document directory layout, module boundaries, and where to add new code.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13 (Phase 3 User Story 1) + 2026-05-13 (Phase 4 User Story 2 — interactive browse) + 2026-05-13 (Phase 5 User Story 3 — plugin disable subcommand) + 2026-05-13 (Phase 6 User Story 4 slice 1 — models commands) + 2026-05-13 (Phase 7 User Stories 5–7 — reindex orchestrator, catalog-update cascade, explicit CLI)

## Directory Layout

```
tome/
├── src/                           # Rust library and binary source
│   ├── main.rs                    # CLI entry point: parse → dispatch → exit
│   ├── lib.rs                     # Public module surface
│   ├── cli.rs                     # clap derive definitions (global flags, subcommands)
│   ├── error.rs                   # Closed TomeError enum + exit code mapping
│   ├── catalog/                   # Catalog management (Phase 1)
│   │   ├── mod.rs                 # Module aggregation
│   │   ├── git.rs                 # Git shell-outs, signal handling, credential scrubbing
│   │   ├── manifest.rs            # TOML schema + strict parsing + semantic validation
│   │   └── store.rs               # Atomic registry and cache persistence
│   ├── commands/                  # CLI command handlers
│   │   ├── mod.rs                 # Dispatcher: route to subcommand
│   │   ├── catalog/               # `tome catalog <subcommand>`
│   │   │   ├── mod.rs             # Subcommand dispatcher
│   │   │   ├── add.rs             # Register a catalog
│   │   │   ├── remove.rs          # Unregister a catalog
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
│   │   │   └── interactive.rs     # Bare `tome plugin` (no subcommand) interactive browse (Phase 4)
│   │   ├── query.rs               # `tome query <text>` — KNN search (Phase 3)
│   │   └── reindex.rs             # `tome reindex [<scope>] [--force]` — re-embedding (Phase 7; ~280 lines)
│   ├── config.rs                  # Config and CatalogEntry structures (serde + toml)
│   ├── paths.rs                   # XDG-aware path resolution, cache key computation
│   ├── logging.rs                 # tracing-subscriber initialization
│   ├── output.rs                  # Human/JSON formatting, TTY detection
│   ├── plugin/                    # Plugin metadata + lifecycle (Phase 2/3/7)
│   │   ├── mod.rs                 # PluginRecord, PluginStatus, re-exports
│   │   ├── identity.rs            # PluginId: <catalog>/<plugin> address + FromStr
│   │   ├── manifest.rs            # plugin.json (lenient, serde_json; FR-013a)
│   │   ├── frontmatter.rs         # SKILL.md YAML header (lenient + FR-011/FR-012 fallbacks)
│   │   ├── components.rs          # ComponentCounts over skills/agents/commands/hooks/.mcp.json
│   │   └── lifecycle.rs           # enable / disable / reindex orchestrator, resolve_plugin_dir (Phase 3/7)
│   ├── index/                     # SQLite + sqlite-vec local skill index (Phase 2/7)
│   │   ├── mod.rs                 # Re-exports (Phase 7: exports reindex_plugin_atomic)
│   │   ├── schema.rs              # CREATE TABLE statements, MetaSeed
│   │   ├── migrations.rs          # Forward-only migration framework + apply_pending
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
│   └── presentation/              # Table + progress + colour + prompt wrappers (Phase 2)
│       ├── mod.rs
│       ├── tables.rs              # comfy-table helpers, NO_COLOR / non-TTY plain fallback
│       ├── progress.rs            # indicatif wrappers, auto-suppress on non-TTY stderr
│       ├── colour.rs              # owo-colors + NO_COLOR env + --no-color flag
│       └── prompt.rs              # inquire wrappers; refuse on non-TTY (NotATerminal)
│
├── tests/                         # Integration tests
│   ├── catalog_add.rs             # test: register a catalog
│   ├── catalog_remove.rs          # test: remove a catalog
│   ├── catalog_list.rs            # test: list catalogs
│   ├── catalog_update.rs          # test: refresh catalogs
│   ├── catalog_show.rs            # test: show catalog manifest
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
│   ├── concurrency.rs             # test: two-process index contention (Phase 2)
│   ├── schema_migrations.rs       # test: schema migrations (Phase 2)
│   ├── version_output.rs          # test: version output (Phase 2)
│   ├── catalog_update_reindex.rs  # test: cascade on catalog update (Phase 2/7)
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
│       │   ├── catalog-extensions.md # Reindex cascade (Phase 7)
│       │   ├── exit-codes.md
│       │   └── ...
│       ├── quickstart.md
│       └── retro/                 # Phase 2 retro notes (gotchas, patterns, next-time)
│
├── .sdd/                          # SDD (Specification-Driven Development) artefacts
│   └── codebase/
│       ├── ARCHITECTURE.md        # System design, patterns, data flow (this section)
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
| `src/main.rs` | Binary entry point; parses CLI, installs signal handler, dispatches, handles errors. | — (entry point, not a module) |
| `src/lib.rs` | Library surface; aggregates `catalog`, `cli`, `commands`, `config`, `error`, `logging`, `output`, `paths`, `plugin`, `index`, `embedding`, `presentation`. | Public for integration tests. |
| `src/cli.rs` | clap derive definitions for global flags (`--json`, `-v`/`-vv`) and subcommands. | `Cli`, `Command`, `CatalogCommand`, `ModelsCommand`, `PluginCommand`, `ReindexCommand`, arg structs. |
| `src/error.rs` | Closed `TomeError` enum; exit code and category mapping; error variants. | `TomeError`, `ManifestInvalid`, `PluginState`, etc. (consumed by all). |
| `src/catalog/` | Catalog management: manifest parsing, Git operations, atomic registry persistence. | `CatalogManifest`, `Git`, `store::load/save/write_atomic`. |
| `src/commands/` | Command handlers; implement `tome catalog/models/plugin/query/reindex <subcommand>`. | Per-subcommand `run(args, mode)` functions; `reindex::run_with_deps()` for library tests. |
| `src/config.rs` | `Config` and `CatalogEntry` struct definitions. | `Config`, `CatalogEntry`. |
| `src/paths.rs` | XDG-aware path resolution and cache key computation. | `Paths`, `Paths::resolve()`, `Paths::cache_dir_for()`, `Paths::model_path()`. |
| `src/logging.rs` | Initialize `tracing-subscriber` (stderr-only, orthogonal to `--json`). | `Verbosity`, `init()`. |
| `src/output.rs` | Format output as human text or JSON; TTY detection. | `Mode`, `write_json()`, `write_error()`, `stdout_is_tty()`. |
| `src/plugin/` | Plugin metadata parsers, lifecycle orchestrator (enable/disable/reindex). | `PluginId`, `PluginRecord`, `PluginStatus`, `lifecycle::enable/disable/reindex_plugin`, `lifecycle::auto_disable_orphan`, `lifecycle::resolve_plugin_dir`. |
| `src/index/` | SQLite skills DB, KNN search, drift detection, atomic mutations. | `open()`, `acquire_lock()`, `enable_plugin_atomic()`, `reindex_plugin_atomic()`, `knn()`, `MetaSeed`. |
| `src/embedding/` | Model registry, download, embedder/reranker traits. | `Embedder`, `Reranker`, `Scored`, `FastembedEmbedder`, `FastembedReranker`, `MODEL_REGISTRY`. |
| `src/presentation/` | Table, progress, colour, prompt wrappers. | `tables::*`, `progress::*`, `colour::*`, `prompt::*`. |

### `tests/` - Integration Tests

| File | Purpose | Tests |
|------|---------|-------|
| `tests/catalog_add.rs` | Test `tome catalog add` with various source formats, error cases. | Happy path, already-exists, manifest errors. |
| `tests/catalog_remove.rs` | Test `tome catalog remove` with confirmation, `--force`. | Interactive, non-TTY, confirmed. |
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
| `tests/concurrency.rs` | Two-process index contention (Phase 2). | Concurrent enable/list, lockfile contention. |
| `tests/schema_migrations.rs` | Schema migrations (Phase 2). | Forward-only migration, idempotency. |
| `tests/version_output.rs` | Version output (Phase 2). | Clap-derived version. |
| `tests/catalog_update_reindex.rs` | Cascade on catalog update (Phase 2/7). | Skills marked stale when catalog ref changes; orphan cascade via auto_disable_orphan. |
| `tests/common/mod.rs` | Shared fixtures (Phase 6/7). | `paths_for`, `fabricate_installed_model`, `fabricate_all_installed_models`. |

## Module Boundaries

### Catalog Module: `src/catalog/`

The catalog module is fully self-contained and can be tested in isolation.

```
src/catalog/
├── mod.rs           # Aggregates git, manifest, store
├── git.rs           # Git shell-outs + credential scrubbing + signal handling
├── manifest.rs      # TOML parsing (strict tome-catalog.toml) + JSON parsing (lenient plugin.json)
└── store.rs         # Atomic read/write of config.toml
```

**Responsibility**: Manage the lifecycle of a catalog (fetch, parse, validate, persist, refresh).

**Public Interface**:
- `git::Git` — facade for git operations.
- `git::install_signal_handler()`, `git::was_cancelled()` — signal handling.
- `manifest::CatalogManifest::parse_and_validate()` — strict parsing and validation.
- `manifest::read_catalog_manifest()` — lenient read for plugin/list/show.
- `store::load()`, `store::save()`, `store::write_atomic()` — atomic persistence.

**What It Cannot Do**:
- Know about CLI argument structures (those live in `cli.rs` and `commands/catalog/`).
- Format output for the user (that's `output.rs` and `presentation/`'s job).
- Initialize logging (that's `logging.rs`'s job).

### Commands Module: `src/commands/`

Each subcommand lives in its own file. All subcommands are dispatched from their respective `mod.rs` files.

```
src/commands/
├── mod.rs           # Top-level dispatcher (catalog vs models vs plugin vs query vs reindex)
├── catalog/
│   ├── mod.rs       # Dispatcher
│   ├── add.rs       # Register a catalog
│   ├── remove.rs    # Unregister
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
│   └── interactive.rs # Bare `tome plugin` interactive browse (Phase 4; ~515 lines)
├── query.rs         # (Phase 3) Query/search
└── reindex.rs       # (Phase 7) Re-embedding (scope parsing, lazy embedder, aggregate output; ~280 lines)
```

**Responsibility**: Translate CLI arguments into library operations; orchestrate error handling and output formatting.

**Signature Pattern** (all subcommands):
```rust
pub fn run(args: SomeArgs, mode: output::Mode) -> Result<(), TomeError>
```

**Interactive Pattern** (Phase 4):
```rust
pub fn run_interactive(mode: output::Mode) -> Result<(), TomeError>
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
└── lifecycle.rs      # enable / disable / reindex_plugin orchestrator + resolve_plugin_dir + auto_disable_orphan (Phase 7)
```

**Responsibility** (library-shaped, no CLI):
- Read-only parsing of plugin metadata (manifest.json, SKILL.md frontmatter).
- Orchestrate enable/disable/reindex: compose index + embedding + manifest parsing into atomic operations.
- Resolve plugin directories (manifest-first, with fallback).
- De-index orphaned plugins on catalog refresh.

**Public Interface**:
- `PluginId`, `PluginRecord`, `PluginStatus` — types.
- `lifecycle::enable(id, deps) -> Result<EnableOutcome>` — full enable flow.
- `lifecycle::disable(id, paths, config, seeds) -> Result<DisableOutcome>` — full disable flow.
- `lifecycle::reindex_plugin(id, deps, force) -> Result<ReindexOutcome>` — full reindex flow (Phase 7).
- `lifecycle::auto_disable_orphan(id, deps) -> Result<u32>` — de-index orphaned plugin (Phase 7).
- `lifecycle::resolve_plugin_dir(id, config) -> Result<PathBuf>` — directory resolution.
- `manifest::parse_plugin_manifest()`, `frontmatter::parse_skill_frontmatter()` — parsers.
- `components::count_components()` — component walk.

**What It Cannot Do**:
- Know about CLI argument structures (those live in `commands/plugin/`).
- Format output (that's `commands/plugin/` and `presentation/`'s job).
- Prompt the user for downloads (that's `commands/plugin/enable.rs`'s responsibility; the library receives `allow_model_download` boolean).
- Orchestrate interactive browse (that's `commands/plugin/interactive.rs`'s responsibility).

### Index Module: `src/index/`

```
src/index/
├── mod.rs              # Re-exports (Phase 7: exports reindex_plugin_atomic)
├── schema.rs           # CREATE TABLE + MetaSeed
├── migrations.rs       # Forward-only migration framework
├── vec_ext.rs          # sqlite-vec extension loader
├── db.rs               # open() + PRAGMA setup + bootstrap/migrate
├── lock.rs             # Advisory write lock
├── meta.rs             # Metadata read/write + drift detection
├── integrity.rs        # PRAGMA integrity_check
├── skills.rs           # CRUD + content_hash + enable_plugin_atomic + reindex_plugin_atomic (Phase 7; ~510 lines)
└── query.rs            # KNN search + filters
```

**Responsibility** (library-shaped, no CLI):
- Maintain SQLite skills DB with vector embeddings.
- Support atomic multi-skill inserts (enable).
- Support atomic per-plugin reindex with smart re-embedding (Phase 7).
- Support atomic enable-flag updates (disable).
- Provide KNN search over enabled skills.
- Detect embedder/reranker drift.
- Manage advisory locks for write operations.

**Public Interface**:
- `open(path, seeds) -> Result<Connection>` — open or bootstrap.
- `acquire_lock(path) -> Result<Lock>` — write lock (filesystem level, OS FD-based).
- `enable_plugin_atomic(&mut conn, pending, embed_fn) -> Result<EnableSummary>` — insert skills under one transaction.
- `reindex_plugin_atomic(&mut conn, catalog, plugin, pending, force, embed_fn) -> Result<ReindexSummary>` — diff on-disk vs index, re-embed modified/added, delete removed (Phase 7).
- `mark_all_disabled_for_plugin(conn, catalog, plugin) -> Result<u32>` — flip enabled flag.
- `query::knn(conn, vec, k, filters) -> Result<Vec<Candidate>>` — KNN search.
- `meta::detect_drift(conn) -> Result<DriftStatus>` — drift detection.

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

## Where to Add New Code

| If you're adding... | Put it in... | Example |
|---------------------|--------------|---------|
| New catalog subcommand | `src/commands/catalog/{name}.rs` + add to dispatcher in `mod.rs` | `src/commands/catalog/verify.rs` (verify manifest syntax) |
| New models subcommand | `src/commands/models/{name}.rs` + add to dispatcher in `mod.rs` | `src/commands/models/verify.rs` (verify model integrity) |
| New plugin subcommand | `src/commands/plugin/{name}.rs` + add to dispatcher in `mod.rs` | `src/commands/plugin/verify.rs` (verify plugin integrity) |
| New top-level command | `src/commands/{name}.rs` + add to dispatcher in `src/commands/mod.rs` | `src/commands/status.rs` (show overall system status) |
| New CLI global flag | `src/cli.rs` in `struct Cli` | `#[arg(long, global = true)] pub verify: bool,` |
| New error type | `src/error.rs` in `TomeError` enum | Add variant + exit code + test in `tests/exit_codes.rs` |
| New manifest validation rule | `src/catalog/manifest.rs::validate_semantic()` | Validate plugin version semver |
| New Git operation | `src/catalog/git.rs` as a `Git` method | `pub fn fetch_tags(&self, url: &str) -> Result<Vec<String>>` |
| New plugin metadata field | `src/plugin/manifest.rs` + `frontmatter.rs` | Add `homepage` URL to `PluginManifest` |
| New lifecycle step | `src/plugin/lifecycle.rs` (private fn inside `enable`/`disable`/`reindex_plugin`) | Add model pre-validation before lock |
| New index operation | `src/index/skills.rs` | `pub fn update_skill_embedding()` for selective re-embedding |
| New KNN filter | `src/index/query.rs` + `QueryFilters` | Add `--min-version` filter |
| New model kind | `src/embedding/mod.rs` (`ModelKind` enum) + `registry.rs` | Add reranker v2 variant |
| New interactive sub-flow | `src/commands/plugin/interactive.rs` (extend existing loop levels) | Add a cascade to `plugin_loop` for plugin tags/categories |
| New reindex scope | `src/commands/reindex.rs` (`Scope` enum) | Add `Org(String)` for organization-scoped reindex |
| Test for a command | `tests/{command_area}_{action}.rs` | `tests/models_download.rs` |
| Test for error scenario | `tests/error_messages.rs` or new file | Document the error text clearly |
| Test for interactive flow | `tests/plugin_interactive.rs` + `rexpect` pty harness | Additional test cases for specific user paths |
| Test for models command | `tests/models_{download,list,remove}.rs` | Test `--verify`, `--force`, on-disk state handling |
| Test for reindex scope | `tests/reindex.rs` + library API | Test All / Catalog / Plugin scope variants with StubEmbedder |
| Test shared helper | `tests/common/mod.rs` | Add `fabricate_*` factory functions |

## Naming Conventions

| Category | Convention | Examples |
|----------|-----------|----------|
| **Struct/Enum** | PascalCase | `CatalogManifest`, `CatalogEntry`, `TomeError`, `PluginId`, `EnableOutcome`, `DisableOutcome`, `ReindexOutcome`, `Candidate`, `ModelState` |
| **Trait** | PascalCase | `Embedder`, `Reranker`, `Git` |
| **Function/Method** | snake_case | `parse_and_validate()`, `install_signal_handler()`, `enable_plugin_atomic()`, `reindex_plugin_atomic()`, `resolve_plugin_dir()` |
| **Constant** | SCREAMING_SNAKE_CASE | `MODEL_REGISTRY`, `SCHEMA_URI`, `HANDLER_INSTALLED` |
| **Module** | snake_case directory names | `src/catalog/`, `src/commands/`, `src/plugin/`, `src/index/` |
| **Test** | `#[test]` with descriptive name | `#[test] fn unknown_field_is_rejected()` |
| **Integration test file** | Matches the feature being tested | `tests/plugin_enable.rs` tests `tome plugin enable`; `tests/reindex.rs` tests `tome reindex` |
| **Interactive loop level** | Private enum in interactive.rs | `LoopExit::Continue`, `LoopExit::Back`, `LoopExit::Quit` |
| **Reindex scope** | PublicEnum in commands/reindex.rs | `Scope::All`, `Scope::Catalog`, `Scope::Plugin` |
| **Model state classification** | PublicEnum in commands/models/mod.rs | `ModelState::Ok`, `ModelState::Missing`, `ModelState::Corrupt`, `ModelState::ChecksumMismatched` |

## Entry Points

| File | Purpose |
|------|---------|
| `src/main.rs` | Binary entry; parses CLI and dispatches to handlers. |
| `src/lib.rs` | Library aggregation; exposes public modules for tests. |
| `tests/catalog_add.rs` | Integration tests directly import from `tome::*` and test the library. |
| `tests/reindex.rs` | Library-API tests via `commands::reindex::run_with_deps()` with `StubEmbedder`. |

## Module Stability Guarantees

- **Stable Public API**: `catalog::git`, `catalog::manifest`, `catalog::store`, `config`, `error`, `output`, `paths`, `cli`, `plugin`, `index`, `embedding`, `presentation`.
- **Internal**: Submodule organization within `commands/` is flexible; subcommand `run()` signatures (and `run_interactive()` for bare `plugin`, `run_with_deps()` for `reindex` tests) are the public contract.

## Generated Files

No files in Phase 1–7 are auto-generated.

---

## What Does NOT Belong Here

- Architecture patterns → ARCHITECTURE.md
- Technology choices → STACK.md
- Code style rules → CONVENTIONS.md
- Test patterns → TESTING.md

---

## Phase 7 additions — User Stories 5–7 (reindex orchestrator, catalog-update cascade, explicit CLI)

Phase 7 landed slices 1–3 across PRs #25–#27. Slice 1 (PR #25) added
`src/index/skills::reindex_plugin_atomic()` (~110 lines, mirrors
`enable_plugin_atomic` atomic contract) and `src/plugin/lifecycle::reindex_plugin()`
(~40 lines public API, ~100 lines helpers) plus `lifecycle::auto_disable_orphan()`
(orphan de-indexing for `tome catalog update` cascade). Slice 2 (PR #26) extended
`src/commands/catalog/update.rs` with reindex-per-plugin and auto-disable cascade;
added `GetOrInsertWithResult` trait for lazy embedder loading (heavy ~345 MB ONNX
only loaded when reindex work is needed). Slice 3 (PR #27) added explicit
`src/commands/reindex.rs` (~280 lines, scope parsing, aggregate emission, library
test entry point) and wired `Command::Reindex(ReindexArgs)` in `cli.rs`. Test
total 195 → 200+ across 29 suites (added `tests/reindex.rs` for library-API
testing with `StubEmbedder`).

### Key architectural patterns (Phase 7):

**Per-Plugin Atomic Reindex**: `reindex_plugin_atomic` mirrors `enable_plugin_atomic`
— one SQLite transaction under one advisory lock. Batch operations (`tome catalog
update`, `tome reindex`) loop per-plugin, committing each before moving to the next.
SIGINT between plugins leaves earlier plugins committed (per-plugin boundary).

**Lazy Embedder Loading**: Heavy embedder (~345 MB ONNX) is loaded only when
reindexing will actually call it. `tome catalog update` and `tome reindex` defer
load until the first enabled plugin is encountered; a sync with zero enabled
plugins never touches model files. The pattern: `Option::get_or_insert_with_result`
trait (PR #26) reads cleanly and avoids double-checking.

**Smart Re-Embedding via Content-Hash** (FR-032): When a skill's `(name, description)`
text composition is identical to what was indexed, Tome skips the embedder call and
reuses the vector. Hash is `SHA256(name + "\n\n" + description)`. Applied by
both `enable` (first-time, always embed) and `reindex` (smart fast-path).

**Sqlite-Vec Virtual Table Workaround**: `skill_embeddings` is a `vec0` virtual table
that does not support `INSERT OR REPLACE`. Upsert logic (Phase 7 PR #25 bug-fix):
DELETE-then-INSERT per skill ID. The DELETE is a no-op on first insert. Same pattern
used by both `enable_plugin_atomic` and `reindex_plugin_atomic`.

**Cascade on Catalog Refresh** (FR-033): `tome catalog update` reindexes enabled
plugins in each catalog after a Git refresh. If a plugin is not found post-refresh
(`PluginNotFound` / `PluginManifestParseError`), `lifecycle::auto_disable_orphan()`
de-indexes all rows and emits a loud warning. Each `lifecycle::reindex_plugin` call
owns its own lock — per-plugin atomicity with respect to interruption.

**Library Test Entry Point** (Phase 7 PR #27): `src/commands/reindex::run_with_deps()`
allows tests to drive the reindex logic with `StubEmbedder`, keeping the CLI binary's
`FastembedEmbedder` out of CI. Scope validation is bypassed (pre-validated by the
caller), so tests can scope by plugin without registering a catalog.

---

*This document shows WHERE code lives. Update when directory structure changes.*

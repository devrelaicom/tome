# Project Structure

> **Purpose**: Document directory layout, module boundaries, and where to add new code.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13 (Phase 3 User Story 1) + 2026-05-13 (Phase 4 User Story 2 — interactive browse) + 2026-05-13 (Phase 5 User Story 3 — plugin disable subcommand)

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
│   │   │   ├── update.rs          # Refresh catalogs
│   │   │   ├── show.rs            # Inspect catalog manifest
│   │   │   └── source.rs          # URL resolution (owner/repo → GitHub URL)
│   │   ├── plugin/                # `tome plugin <subcommand>` + interactive (Phase 3–5)
│   │   │   ├── mod.rs             # Dispatcher + shared helpers
│   │   │   ├── enable.rs          # Enable a plugin (embed + index)
│   │   │   ├── disable.rs         # Disable a plugin (Phase 5)
│   │   │   ├── list.rs            # List plugins (all or for one catalog)
│   │   │   ├── show.rs            # Show one plugin's metadata + state
│   │   │   └── interactive.rs     # Bare `tome plugin` (no subcommand) interactive browse (Phase 4)
│   │   └── query.rs               # `tome query <text>` — KNN search (Phase 3)
│   ├── config.rs                  # Config and CatalogEntry structures (serde + toml)
│   ├── paths.rs                   # XDG-aware path resolution, cache key computation
│   ├── logging.rs                 # tracing-subscriber initialization
│   ├── output.rs                  # Human/JSON formatting, TTY detection
│   ├── plugin/                    # Plugin metadata + lifecycle (Phase 2/3)
│   │   ├── mod.rs                 # PluginRecord, PluginStatus, re-exports
│   │   ├── identity.rs            # PluginId: <catalog>/<plugin> address + FromStr
│   │   ├── manifest.rs            # plugin.json (lenient, serde_json; FR-013a)
│   │   ├── frontmatter.rs         # SKILL.md YAML header (lenient + FR-011/FR-012 fallbacks)
│   │   ├── components.rs          # ComponentCounts over skills/agents/commands/hooks/.mcp.json
│   │   └── lifecycle.rs           # enable / disable orchestrator, resolve_plugin_dir (Phase 3)
│   ├── index/                     # SQLite + sqlite-vec local skill index (Phase 2)
│   │   ├── mod.rs                 # Re-exports
│   │   ├── schema.rs              # CREATE TABLE statements, MetaSeed
│   │   ├── migrations.rs          # Forward-only migration framework + apply_pending
│   │   ├── vec_ext.rs             # sqlite-vec auto-extension registrar
│   │   ├── db.rs                  # open(): paths → conn → PRAGMAs → bootstrap/migrate → verify
│   │   ├── lock.rs                # Advisory write lock via File::try_lock (per-fd, OS-level)
│   │   ├── meta.rs                # Typed MetaKey + read/write + DriftStatus + detect_drift
│   │   ├── integrity.rs           # PRAGMA integrity_check wrapper
│   │   ├── skills.rs              # CRUD + content_hash + enable_plugin_atomic + mark_all_disabled_for_plugin
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
│   ├── concurrency.rs             # test: two-process index contention (Phase 2)
│   ├── schema_migrations.rs       # test: schema migrations (Phase 2)
│   ├── version_output.rs          # test: version output (Phase 2)
│   ├── catalog_update_reindex.rs  # test: cascade on catalog update (Phase 2)
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
| `src/cli.rs` | clap derive definitions for global flags (`--json`, `-v`/`-vv`) and subcommands. | `Cli`, `Command`, `CatalogCommand`, `PluginCommand`, `PluginArgs`, arg structs. |
| `src/error.rs` | Closed `TomeError` enum; exit code and category mapping; error variants. | `TomeError`, `ManifestInvalid`, `PluginState`, etc. (consumed by all). |
| `src/catalog/` | Catalog management: manifest parsing, Git operations, atomic registry persistence. | `CatalogManifest`, `Git`, `store::load/save/write_atomic`. |
| `src/commands/` | Command handlers; implement `tome catalog/plugin/query <subcommand>`. | Per-subcommand `run(args, mode)` functions. |
| `src/config.rs` | `Config` and `CatalogEntry` struct definitions. | `Config`, `CatalogEntry`. |
| `src/paths.rs` | XDG-aware path resolution and cache key computation. | `Paths`, `Paths::resolve()`, `Paths::cache_dir_for()`, `Paths::model_path()`. |
| `src/logging.rs` | Initialize `tracing-subscriber` (stderr-only, orthogonal to `--json`). | `Verbosity`, `init()`. |
| `src/output.rs` | Format output as human text or JSON; TTY detection. | `Mode`, `write_json()`, `write_error()`, `stdout_is_tty()`. |
| `src/plugin/` | Plugin metadata parsers, lifecycle orchestrator. | `PluginId`, `PluginRecord`, `PluginStatus`, `lifecycle::enable/disable`, `lifecycle::resolve_plugin_dir`. |
| `src/index/` | SQLite skills DB, KNN search, drift detection. | `open()`, `acquire_lock()`, `enable_plugin_atomic()`, `knn()`, `MetaSeed`. |
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
| `tests/concurrency.rs` | Two-process index contention (Phase 2). | Concurrent enable/list, lockfile contention. |
| `tests/schema_migrations.rs` | Schema migrations (Phase 2). | Forward-only migration, idempotency. |
| `tests/version_output.rs` | Version output (Phase 2). | Clap-derived version. |
| `tests/catalog_update_reindex.rs` | Cascade on catalog update (Phase 2). | Skills marked stale when catalog ref changes. |

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
├── mod.rs           # Top-level dispatcher (catalog vs plugin vs query)
├── catalog/
│   ├── mod.rs       # Dispatcher
│   ├── add.rs       # Register a catalog
│   ├── remove.rs    # Unregister
│   ├── list.rs      # Show all catalogs
│   ├── update.rs    # Refresh
│   ├── show.rs      # Show one catalog's manifest
│   └── source.rs    # URL resolution helper
├── plugin/          # (Phase 3–5)
│   ├── mod.rs       # Dispatcher + shared helpers (model checking, index opening)
│   ├── enable.rs    # Enable a plugin
│   ├── disable.rs   # Disable a plugin (Phase 5; ~108 lines)
│   ├── list.rs      # List plugins
│   ├── show.rs      # Show one plugin
│   └── interactive.rs # Bare `tome plugin` interactive browse (Phase 4; ~515 lines)
└── query.rs         # (Phase 3) Query/search
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
└── lifecycle.rs      # enable / disable orchestrator + resolve_plugin_dir
```

**Responsibility** (library-shaped, no CLI):
- Read-only parsing of plugin metadata (manifest.json, SKILL.md frontmatter).
- Orchestrate enable/disable: compose index + embedding + manifest parsing into atomic operations.
- Resolve plugin directories (manifest-first, with fallback).

**Public Interface**:
- `PluginId`, `PluginRecord`, `PluginStatus` — types.
- `lifecycle::enable(id, deps) -> Result<EnableOutcome>` — full enable flow.
- `lifecycle::disable(id, paths, config, seeds) -> Result<DisableOutcome>` — full disable flow.
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
├── mod.rs              # Re-exports
├── schema.rs           # CREATE TABLE + MetaSeed
├── migrations.rs       # Forward-only migration framework
├── vec_ext.rs          # sqlite-vec extension loader
├── db.rs               # open() + PRAGMA setup + bootstrap/migrate
├── lock.rs             # Advisory write lock
├── meta.rs             # Metadata read/write + drift detection
├── integrity.rs        # PRAGMA integrity_check
├── skills.rs           # CRUD + enable_plugin_atomic + mark_all_disabled_for_plugin
└── query.rs            # KNN search + filters
```

**Responsibility** (library-shaped, no CLI):
- Maintain SQLite skills DB with vector embeddings.
- Support atomic multi-skill inserts (enable).
- Support atomic enable-flag updates (disable).
- Provide KNN search over enabled skills.
- Detect embedder/reranker drift.
- Manage advisory locks for write operations.

**Public Interface**:
- `open(path, seeds) -> Result<Connection>` — open or bootstrap.
- `acquire_lock(path) -> Result<Lock>` — write lock (filesystem level, OS FD-based).
- `enable_plugin_atomic(&mut conn, pending, embed_fn) -> Result<EnableSummary>` — insert skills under one transaction.
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
├── download.rs         # Atomic reqwest::blocking download + SIGINT awareness
├── runtime.rs          # Placeholder (ort transitive)
├── fastembed.rs        # FastembedEmbedder + FastembedReranker
└── stub.rs             # StubEmbedder + identity reranker (test-only by default)
```

**Responsibility** (library-shaped, no CLI):
- Define `Embedder` and `Reranker` trait interfaces.
- Implement fastembed-backed wrappers (`FastembedEmbedder`, `FastembedReranker`).
- Provide deterministic test double (`StubEmbedder`).
- Manage model registry, download, checksum validation.

**Public Interface**:
- `Embedder { fn embed(&self, text: &str) -> Result<Vec<f32>>; }` — trait.
- `Reranker { fn rerank(&self, text: &str, candidates: Vec<Candidate>) -> Result<Vec<Scored>>; }` — trait.
- `Scored { score: f32, candidate: Candidate }` — result type.
- `FastembedEmbedder::load(entry, dir) -> Result<Self>` — load model from disk.
- `FastembedReranker::load(entry, dir) -> Result<Self>` — load reranker from disk.
- `MODEL_REGISTRY` — array of `ModelEntry` (embedder + reranker pinned versions).
- `download::download_model(entry, dir) -> Result<()>` — atomic download with SIGINT awareness.

**What It Cannot Do**:
- Know about CLI arguments or prompts (that's `commands/plugin/`'s job).
- Manage paths (that's `paths.rs`'s job; commands pass the resolved directory).

## Where to Add New Code

| If you're adding... | Put it in... | Example |
|---------------------|--------------|---------|
| New catalog subcommand | `src/commands/catalog/{name}.rs` + add to dispatcher in `mod.rs` | `src/commands/catalog/verify.rs` (verify manifest syntax) |
| New plugin subcommand | `src/commands/plugin/{name}.rs` + add to dispatcher in `mod.rs` | `src/commands/plugin/verify.rs` (verify plugin integrity) |
| New top-level command | `src/commands/{name}.rs` + add to dispatcher in `src/commands/mod.rs` | `src/commands/models.rs` (list installed models) |
| New CLI global flag | `src/cli.rs` in `struct Cli` | `#[arg(long, global = true)] pub verify: bool,` |
| New error type | `src/error.rs` in `TomeError` enum | Add variant + exit code + test in `tests/exit_codes.rs` |
| New manifest validation rule | `src/catalog/manifest.rs::validate_semantic()` | Validate plugin version semver |
| New Git operation | `src/catalog/git.rs` as a `Git` method | `pub fn fetch_tags(&self, url: &str) -> Result<Vec<String>>` |
| New plugin metadata field | `src/plugin/manifest.rs` + `frontmatter.rs` | Add `homepage` URL to `PluginManifest` |
| New lifecycle step | `src/plugin/lifecycle.rs` (private fn inside `enable`/`disable`) | Add model pre-validation before lock |
| New index operation | `src/index/skills.rs` | `pub fn update_skill_embedding()` for selective re-embedding |
| New KNN filter | `src/index/query.rs` + `QueryFilters` | Add `--min-version` filter |
| New model kind | `src/embedding/mod.rs` (`ModelKind` enum) + `registry.rs` | Add reranker v2 variant |
| New interactive sub-flow | `src/commands/plugin/interactive.rs` (extend existing loop levels) | Add a cascade to `plugin_loop` for plugin tags/categories |
| Test for a command | `tests/{command_area}_{action}.rs` | `tests/plugin_disable.rs` |
| Test for error scenario | `tests/error_messages.rs` or new file | Document the error text clearly |
| Test for interactive flow | `tests/plugin_interactive.rs` + `rexpect` pty harness | Additional test cases for specific user paths |

## Naming Conventions

| Category | Convention | Examples |
|----------|-----------|----------|
| **Struct/Enum** | PascalCase | `CatalogManifest`, `CatalogEntry`, `TomeError`, `PluginId`, `EnableOutcome`, `Candidate` |
| **Trait** | PascalCase | `Embedder`, `Reranker`, `Git` |
| **Function/Method** | snake_case | `parse_and_validate()`, `install_signal_handler()`, `enable_plugin_atomic()`, `resolve_plugin_dir()` |
| **Constant** | SCREAMING_SNAKE_CASE | `MODEL_REGISTRY`, `SCHEMA_URI`, `HANDLER_INSTALLED` |
| **Module** | snake_case directory names | `src/catalog/`, `src/commands/`, `src/plugin/`, `src/index/` |
| **Test** | `#[test]` with descriptive name | `#[test] fn unknown_field_is_rejected()` |
| **Integration test file** | Matches the feature being tested | `tests/plugin_enable.rs` tests `tome plugin enable` |
| **Interactive loop level** | Private enum in interactive.rs | `LoopExit::Continue`, `LoopExit::Back`, `LoopExit::Quit` |

## Entry Points

| File | Purpose |
|------|---------|
| `src/main.rs` | Binary entry; parses CLI and dispatches to handlers. |
| `src/lib.rs` | Library aggregation; exposes public modules for tests. |
| `tests/catalog_add.rs` | Integration tests directly import from `tome::*` and test the library. |

## Module Stability Guarantees

- **Stable Public API**: `catalog::git`, `catalog::manifest`, `catalog::store`, `config`, `error`, `output`, `paths`, `cli`, `plugin`, `index`, `embedding`, `presentation`.
- **Internal**: Submodule organization within `commands/` is flexible; subcommand `run()` signatures (and `run_interactive()` for bare `plugin`) are the public contract.

## Generated Files

No files in Phase 1–5 are auto-generated.

---

## What Does NOT Belong Here

- Architecture patterns → ARCHITECTURE.md
- Technology choices → STACK.md
- Code style rules → CONVENTIONS.md
- Test patterns → TESTING.md

---

## Phase 2 additions — foundational (no user-facing CLI yet)

Phase 2 added four new modules under `src/`, one vendored C library under
`vendor/`, and seven integration-test files under `tests/`. None were wired
into `src/cli.rs` until Phase 3.

### `src/plugin/` — third-party metadata parsers

```
src/plugin/
├── mod.rs              # PluginRecord, PluginStatus, re-exports
├── identity.rs         # PluginId: <catalog>/<plugin> address + FromStr
├── manifest.rs         # plugin.json (lenient, serde_json; FR-013a)
├── frontmatter.rs      # SKILL.md YAML header (lenient + FR-011/FR-012)
└── components.rs       # ComponentCounts over skills/agents/commands/hooks/.mcp.json
```

**Responsibility**: read-only parsing of plugin metadata produced by
upstream tooling (Claude Code plugins). Strictness boundary: lenient parsing
of all third-party inputs; unknown fields are ignored without warning
(FR-013a). Two failure modes for `frontmatter.rs`: delimiter failure is
fatal (caller maps to exit 23), YAML-body failure is per-skill skip-and-warn
(FR-013c).

### `src/index/` — SQLite + sqlite-vec local skill index

```
src/index/
├── mod.rs              # Re-exports
├── schema.rs           # CREATE_STATEMENTS, bootstrap, MetaSeed
├── migrations.rs       # Forward-only migration framework + apply_pending
├── vec_ext.rs          # sqlite-vec auto-extension registrar
├── db.rs               # open(): paths → conn → PRAGMAs → bootstrap/migrate → verify
├── lock.rs             # Advisory write lock via File::try_lock (per-fd, OS-level)
├── meta.rs             # Typed MetaKey + read/write + DriftStatus + detect_drift
├── integrity.rs        # PRAGMA integrity_check wrapper
├── skills.rs           # CRUD + content_hash + enable_plugin_atomic (FR-004)
└── query.rs            # KNN over skill_embeddings joined with skills.enabled = 1
```

**Concurrency model** (research §R2): WAL + 5 s `busy_timeout` + a Tome-owned
advisory lockfile at `${XDG_DATA_HOME}/tome/index.lock`. Read-only commands
(`query`, `plugin list`, `plugin show`, `status`) do not take the lockfile;
mutating commands do. Contention surfaces as `TomeError::IndexBusy` (exit
50) within milliseconds.

### `src/embedding/` — embedder, reranker, model registry

```
src/embedding/
├── mod.rs              # Embedder + Reranker traits, Scored
├── registry.rs         # MODEL_REGISTRY const + ModelManifest (strict serde)
├── download.rs         # Atomic, SIGINT-aware reqwest::blocking downloader
├── runtime.rs          # No-op placeholder (ort is transitive only)
├── fastembed.rs        # FastembedEmbedder + FastembedReranker
└── stub.rs             # Deterministic SHA-derived embedder + identity/reverse reranker
```

**Boundary trait pattern**: `Embedder` and `Reranker` are the seam between
Tome's deterministic core and the ONNX-backed external system (constitution
principle VIII). The stub is unconditional + `#[doc(hidden)]` so integration
tests can use it without a Cargo feature gate; LTO strips it from release
binaries that don't reference it.

### `src/presentation/` — table + progress + colour + prompt wrappers

```
src/presentation/
├── mod.rs
├── tables.rs           # comfy-table helpers, NO_COLOR / non-TTY plain fallback
├── progress.rs         # indicatif wrappers, auto-suppress on non-TTY stderr
├── colour.rs           # owo-colors + NO_COLOR env + --no-color flag
└── prompt.rs           # inquire wrappers; refuse on non-TTY (NotATerminal)
```

### `vendor/sqlite-vec/` — compiled-in C extension

```
vendor/sqlite-vec/
├── sqlite-vec.c        # Pinned amalgamation (v0.1.9)
├── sqlite-vec.h
└── LICENSE
```

`build.rs` compiles the amalgamation against `rusqlite`'s bundled SQLite
headers and links it statically. Loaded into every `Connection` at runtime
via `sqlite3_auto_extension` from `src/index/vec_ext.rs`.

### Phase 2 integration tests

| File | Tests |
|------|-------|
| `tests/frontmatter.rs` | Table-driven matrix over the SKILL.md parser (delimiter / YAML-body failure split + FR-011 / FR-012 fallbacks). |
| `tests/index_schema_bootstrap.rs` | Fresh DB bootstrap, meta seeding, vec extension reachability, schema-too-new refusal. |
| `tests/index_lock.rs` | Advisory lock contention + release; pre-existing lockfile is reusable. |
| `tests/embedding_stub.rs` | Stub determinism, distinguishability (cosine < 0.99), 384-dim length, L2 normalisation. |
| `tests/model_download.rs` | Hand-rolled `TcpListener` HTTP server fixture; happy path, checksum mismatch, HTTP 404, placeholder-checksum refusal. |
| `tests/paths_phase2.rs` | Phase 2 path resolvers (index_db, index_lock, models_dir, model_path). |
| `tests/scrubbing.rs` (extended) | Phase 1 cases + AWS/HF signed URL keys + reqwest error chain redaction. |

---

## Phase 3 additions — User Story 1 (plugin enable/disable, query)

Phase 3 slice 1 (merged) added `src/plugin/lifecycle.rs` (library) and
`src/commands/plugin/{enable,list,show}.rs` (CLI). Slice 2 (merged) added
`src/commands/query.rs` (KNN search). No new modules; composition of
existing layers. `lifecycle::resolve_plugin_dir` is manifest-first (reads
`tome-catalog.toml`, falls back to flat layout) — single source of truth
used by enable, list, show (fixes inconsistency).

Key flows:
- **Enable**: parse args → resolve plugin dir → check model presence + prompt (if TTY) → load embedder → call `lifecycle::enable()` → render outcome.
- **Disable**: parse args → resolve plugin dir → (optional: confirm prompt) → call `lifecycle::disable()` → render outcome.
- **List**: load config, index → walk catalogs (or single if `--catalog`) → join with index state → render table/NDJSON.
- **Show**: parse id → resolve plugin dir → load manifest → load index state → render plugin card.
- **Query**: parse text + filters → embed → KNN (candidate_k = top_k×4 if reranking) → optional rerank → optional --strict threshold → render results.

---

## Phase 4 additions — User Story 2 (interactive browse)

Phase 4 slice 1a (merged) wired `lifecycle::disable` into a module-private
call site; slice 1b (merged) added `src/commands/plugin/interactive.rs`
(~515 lines) implementing a three-level loop pattern (catalog → plugin →
action). Test coverage via `tests/plugin_interactive.rs` (~288 lines) using
`rexpect` pty harness.

**Three-level loop pattern**:
- **`catalog_loop()`**: Display catalog selector; user picks one or Quit.
- **`plugin_loop(catalog_name)`**: Browse plugins in the selected catalog; user picks one or Back.
- **`view_loop(id, plugin_manifest)`**: Display plugin view; user selects action (Enable, Disable, Back).

**Control flow**: Each loop level uses a private `LoopExit` enum (Continue, Back, Quit).

**Error semantics**: Clean exit (OK) on quit/cancel (Esc/Ctrl-C) — always exit 0 per contract. Enable/disable errors propagate verbatim (same codes as non-interactive).

**TTY enforcement**: Non-TTY invocation surfaces as exit code 98 (NotATerminal).

---

## Phase 5 additions — User Story 3 (plugin disable subcommand)

Phase 5 added `src/commands/plugin/disable.rs` (~108 lines) — thin CLI
wrapper over the pre-existing `plugin::lifecycle::disable` orchestrator
from Phase 4. No changes to library boundaries or module structure. New
CLI variant: `PluginCommand::Disable(PluginDisableArgs { id, force })`.

**Key pattern**: Mirrors `enable.rs` in structure. Owns confirmation-prompt
UX (`--force` short-circuit, non-TTY refusal with pointer message to stderr).
No embedder construction — index-only UPDATE via `mark_all_disabled_for_plugin`.

**Test coverage**: `tests/plugin_disable.rs` (~208 lines) exercises the
CLI path; `tests/plugin_repeated.rs` consolidates idempotency contract
(re-disable → exit 21) alongside re-enable. Cheap re-enable verified via
`tests/plugin_enable.rs::cheap_reenable_after_disable_invokes_embedder_zero_times`.

---

*This document shows WHERE code lives. Update when directory structure changes.*

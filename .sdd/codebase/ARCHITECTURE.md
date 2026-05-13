# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13 (Phase 3 User Story 1 — plugin enable/disable, query)

## Architecture Overview

Tome is a synchronous Rust CLI following a classic **parse → dispatch → execute → map-errors → exit** pipeline. The codebase is organized around a **capability-driven** modular architecture where each module owns a distinct responsibility (catalog management, Git operations, configuration, logging, path resolution, output formatting, plugin metadata parsing, skill indexing, model embedding, and interactive presentation). Error handling is centralized in a closed `TomeError` enum that enforces exhaustive exit-code mapping at compile time. Signal handling (SIGINT) is global and atomic, allowing long-running operations (git clone, model download, embedding) to be cancelled gracefully with a well-defined exit code.

## Architecture Pattern

| Pattern | Description |
|---------|-------------|
| **Sync-only CLI** | No async runtime (`tokio`). All I/O and process orchestration use `std::process` and blocking calls. |
| **Closed Error Set** | All failure paths map to a single `TomeError` enum with explicit exit codes; no `Other` or `Unknown` arms. Adding a failure mode requires specification, error type, and test updates. |
| **Atomic Writes** | Registry mutations, cache operations, and index writes use `tempfile` + rename for POSIX atomicity; SQLite WAL provides the index concurrency contract. Interruptions cannot corrupt state. |
| **Capability-Organized Modules** | Modules group related functionality: `catalog/` (manifest + Git + store), `commands/` (CLI handlers), `config/` (manifest deserialization), `paths/` (XDG resolution), `logging/` (tracing setup), `output/` (human/JSON formatting), `plugin/` (metadata parsing + lifecycle), `index/` (SQLite skills DB + KNN), `embedding/` (fastembed wrapper + model registry + download), `presentation/` (tables / progress / colour / prompts). |
| **Credential Scrubbing at Boundary** | All captured `git` and `reqwest` output passes through credential scrubbing before reaching logging, error display, or structured output. |
| **Trait-based Embedding Abstraction** | `Embedder` and `Reranker` are seam interfaces; `FastembedEmbedder` wraps `fastembed-rs`, and a deterministic `StubEmbedder` (unit-test only) provides testability without model files. |
| **Plugin-Dir Resolution: Manifest-First** | `lifecycle::resolve_plugin_dir` reads `tome-catalog.toml`, looks up `id.plugin` in the declared `plugins[].name`, joins with the source; falls back to flat `entry.path.join(&id.plugin)` for backward compat when manifest is absent. Single shared function across `enable`, `disable`, `list`, `show` fixes inconsistency. |

## Core Components

### CLI & Parsing (`src/cli.rs`, `src/main.rs`)

- **Purpose**: Parse global flags (`--json`, `-v`/`-vv`) and dispatch to subcommand handlers.
- **Location**: `src/main.rs` (entry), `src/cli.rs` (clap derive definitions).
- **Dependencies**: `clap` (argument parsing), `catalog::git` (signal handler installation).
- **Dependents**: `commands/` modules (receive parsed args).
- **Pipeline Entry**: `main()` parses CLI → installs signal handler → dispatches to handler → maps result to exit code.

### Catalog Management (`src/catalog/`, `src/commands/catalog/`)

- **Purpose**: Orchestrate catalog registration, refresh, removal, and inspection; manage Git cloning and credential scrubbing.
- **Location**: `src/catalog/` (core logic: git, manifest, store), `src/commands/catalog/` (subcommand handlers).
- **Dependencies**: `git` (shell-outs), `manifest` (TOML parsing + validation), `store` (atomic writes), `config` (registry persistence).
- **Dependents**: Main CLI, integration tests.
- **Key Invariants**:
  - Catalogs are cached at `~/.local/share/tome/catalogs/<sha256(url)>/`.
  - Config is persisted at `~/.config/tome/config.toml` atomically.
  - Git operations capture stderr and pass it through credential scrubbing before error display.

### Plugin Metadata & Lifecycle (`src/plugin/`, `src/commands/plugin/`)

- **Purpose**: Parse plugin manifests and SKILL.md frontmatter (lenient), manage plugin enable/disable state, orchestrate skill embedding and indexing.
- **Location**: `src/plugin/` (metadata parsers, lifecycle orchestrator), `src/commands/plugin/` (CLI handlers).
- **Dependencies**: `catalog::manifest` (read_catalog_manifest), `index::` (open DB, acquire lock, enable_plugin_atomic), `embedding::` (embedder + reranker, model registry, download).
- **Dependents**: Commands.
- **Key Patterns**:
  - `lifecycle::enable()`: parse manifest (exit 22) → check already-enabled (exit 31) → ensure models present (exit 30 unless allow_model_download) → acquire lock → walk skills → collect PendingSkill → embed + insert under one transaction (atomic per FR-004) → release lock.
  - `lifecycle::disable()`: check not-disabled (exit 32) → acquire lock → flip enabled=0 for all (catalog, plugin) rows → release lock.
  - Frontmatter parse: delimiter error is fatal (exit 23); YAML-body error skips one skill + warn (FR-013c).
  - Models: embedder + reranker required by enable and query; optional download in `enable` (CLI owns the TTY prompt; `lifecycle::allow_model_download` is the decision).

### Skill Query & Search (`src/index/query.rs`, `src/commands/query.rs`)

- **Purpose**: Embed the user's query text, perform KNN over enabled skills, optionally rerank, filter, and render results.
- **Location**: `src/commands/query.rs` (CLI entry and result presentation), `src/index/query.rs` (KNN SQL + filter logic).
- **Dependencies**: `index::` (open read-only, knn), `embedding::` (embedder + reranker), `catalog::manifest` (filter validation).
- **Flow**:
  1. Parse query text and filter flags (`--catalog`, `--plugin`, `--no-rerank`, `--min-score`, `--strict`).
  2. Validate filter flags against registered catalogs (cheap manifest reads).
  3. Open index read-only.
  4. Check embedder drift (exit 41/42 if stale); check reranker drift (warn-only).
  5. Check model presence (embedder always required; reranker if not `--no-rerank`).
  6. Load embedder (always); load reranker (unless `--no-rerank`).
  7. Embed query text.
  8. KNN with `candidate_k = top_k × 4` if reranking, else `top_k`.
  9. Apply reranker or cosine-similarity scoring.
  10. Trim to `top_k`, apply optional `--strict` threshold filter.
  11. Render as table (human) or NDJSON (JSON).

### Git Interface (`src/catalog/git.rs`)

- **Purpose**: Spawn `git` processes, scrub credentials from captured output, handle SIGINT cancellation.
- **Location**: `src/catalog/git.rs`.
- **Dependencies**: `regex` (credential patterns), `ctrlc` (signal handling), `std::process`.
- **Key Methods**:
  - `clone_shallow(url, dest, ref)`: Clone a specific branch/tag/commit.
  - `scrub_credentials(bytes)`: Apply regex rules (R-8 from research.md) to mask tokens, SSH hostnames, etc.
  - `install_signal_handler()`: Set up SIGINT handler (idempotent).
  - `was_cancelled()`: Check if SIGINT fired.
- **Signal Handling**: A global `AtomicBool` is flipped when SIGINT is received; spawned child processes are killed and `TomeError::Interrupted` (exit code 8) is returned.

### Manifest Parsing & Validation (`src/catalog/manifest.rs`)

- **Purpose**: Parse `tome-catalog.toml` (strict) and plugin `plugin.json` (lenient); validate structure and semantic constraints.
- **Location**: `src/catalog/manifest.rs`.
- **Schema Enforcement**:
  - `tome-catalog.toml`: `#[serde(deny_unknown_fields)]` on every struct; unknown fields produce `ManifestInvalid::UnknownField`.
  - `plugin.json`: lenient parsing (serde_json, unknown fields ignored) per FR-013a.
- **Validation Pipeline** (for `tome-catalog.toml`):
  1. UTF-8 decode.
  2. TOML syntax parse.
  3. Required field check (name, description, version, owner.name, owner.email).
  4. Semantic validation (semver version, valid email).
  5. Unique plugin names.
  6. Relative-path plugin sources (no `..`, no absolute paths, no URLs, must resolve within catalog).
- **Error Propagation**: Each failure produces a specific `ManifestInvalid` variant that maps to exit code 5.

### Index & Skills Database (`src/index/`)

- **Purpose**: Maintain a local SQLite skills index with vector embeddings, enable/disable state tracking, drift detection, and KNN search.
- **Location**: `src/index/`.
- **Concurrency**: WAL mode + 5s `busy_timeout` + optional advisory lockfile (`${XDG_DATA_HOME}/tome/index.lock`). Read-only operations (`query`, `plugin list/show`, `status`) do not take the lock; mutating operations (`plugin enable/disable`) do. Contention surfaces as `TomeError::IndexBusy` (exit 50) within milliseconds.
- **Schema**:
  - `meta`: embedder + reranker identity + drift flags.
  - `skills`: `(catalog, plugin, name, description, path, plugin_version, embedding, enabled, indexed_at)` + content-hash column for smart re-embedding.
  - Vectors are L2-normalized 384-dim floats.
- **Key Operations**:
  - `enable_plugin_atomic()`: walk PendingSkill vec, embed each, insert under one transaction, return EnableSummary (total / newly_embedded counts).
  - `mark_all_disabled_for_plugin()`: flip `enabled = 0` for all rows matching `(catalog, plugin)`.
  - `knn(query_vec, k, filters)`: search by `enabled = 1`, apply optional catalog/plugin filters, return top k candidates by cosine distance.

### Embedding & Model Registry (`src/embedding/`)

- **Purpose**: Wrap `fastembed-rs` and `ort` into a testable trait interface; manage model downloads and registry.
- **Location**: `src/embedding/`.
- **Model Registry** (`src/embedding/registry.rs`):
  - Two models pinned: `bge-small-en-v1.5` (embedder, 45 MB, INT8) and `bge-reranker-base` (reranker, 280 MB, INT8).
  - Strict `ModelManifest` JSON schema (per model's `manifest.json`); downloaded models are atomically persisted.
  - Checksums (SHA-256) validated on download; placeholder checksums rejected.
- **Embedder & Reranker Traits** (`src/embedding/mod.rs`):
  - `Embedder::embed(text: &str) -> Result<Vec<f32>>` — produces 384-dim L2-normalized vectors.
  - `Reranker::rerank(text: &str, candidates: Vec<Candidate>) -> Result<Vec<Scored>>` — cross-encoder logits.
  - `FastembedEmbedder` / `FastembedReranker`: production implementations via `ort`.
  - `StubEmbedder` / identity reranker: deterministic stubs for unit tests (no network, no files).
- **Download** (`src/embedding/download.rs`):
  - Atomic downloads: write to temp, verify checksum, rename.
  - SIGINT-aware: polls `git::was_cancelled()`.
  - Credential scrubbing on `reqwest` errors.

### Presentation Layer (`src/presentation/`)

- **Purpose**: Wrap table rendering, progress spinners, colour output, and interactive prompts with TTY awareness and `NO_COLOR` support.
- **Location**: `src/presentation/`.
- **Modules**:
  - `tables.rs`: `comfy-table` helpers; falls back to plain text on non-TTY or `NO_COLOR`.
  - `progress.rs`: `indicatif` spinners; auto-suppressed on non-TTY stderr.
  - `colour.rs`: `owo-colors` + `NO_COLOR` env + `--no-color` flag.
  - `prompt.rs`: `inquire` (Select / MultiSelect / Confirm); refuses on non-TTY with `NotATerminal` error.

### Configuration (`src/config.rs`)

- **Purpose**: Define `Config` and `CatalogEntry` structures; serialize/deserialize via `serde` + `toml`.
- **Location**: `src/config.rs`.
- **Key Types**:
  - `Config`: Top-level document; keyed by catalog display name (BTreeMap for deterministic ordering).
  - `CatalogEntry`: Name, URL, tracked ref, local path, last-synced timestamp.
- **Strict Parsing**: `#[serde(deny_unknown_fields)]` on all structs.

### Path Resolution (`src/paths.rs`)

- **Purpose**: Resolve XDG-aware configuration and data directories; compute content-addressed cache keys; resolve index DB, lock, and model paths.
- **Location**: `src/paths.rs`.
- **XDG Compliance**: Honour `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, fall back to `~/.config` and `~/.local/share`.
- **Phase 2 additions**:
  - `index_db`: `${XDG_DATA_HOME}/tome/index.db`.
  - `index_lock`: `${XDG_DATA_HOME}/tome/index.lock`.
  - `models_dir`: `${XDG_DATA_HOME}/tome/models/`.
  - `model_path(name)` / `model_manifest(name)`: resolve model directories and manifest.json files.

### Logging (`src/logging.rs`)

- **Purpose**: Initialize `tracing-subscriber` with stderr-only output, orthogonal to `--json`.
- **Location**: `src/logging.rs`.
- **Verbosity**: `-v` = info, `-vv` = debug; env vars `TOME_LOG`, `RUST_LOG` supported.
- **Key Invariant**: Logs go to stderr; primary command output (--json or human) goes to stdout. This keeps structured data uncontaminated by debug output.

### Output Formatting (`src/output.rs`)

- **Purpose**: Format results as human-readable text or machine-readable JSON; handle TTY detection and `NO_COLOR`.
- **Location**: `src/output.rs`.
- **Modes**:
  - `Mode::Human`: Friendly multiline text, colours enabled (auto-disabled on non-TTY or NO_COLOR env var).
  - `Mode::Json`: One JSON object per line (NDJSON); always valid for piping.
- **Error Handling**: Error records include category, exit code, and message; always written to stderr.

### Error Handling (`src/error.rs`)

- **Purpose**: Define the closed `TomeError` enum and map each variant to an exit code and category.
- **Location**: `src/error.rs`.
- **Variants** (18+ total, Phase 3 additions):
  - Phase 1: `Internal`, `Usage`, `CatalogNotFound`, `CatalogAlreadyExists`, `ManifestInvalid`, `GitFailed`, `Io`, `Interrupted`.
  - Phase 2: `IndexIntegrityCheckFailure`, `IndexBusy`, `ModelMissing`, `PluginNotFound`, `SkillFrontmatterParseError`.
  - Phase 3: `PluginAlreadyInState`, `QueryNoResultsStrict`, drift checks (exit 41/42).
- **Compile-Time Enforcement**: The `TomeError::exit_code()` method is exhaustive; adding a variant forces edits to `tests/exit_codes.rs`, the spec, and the PRD.

## Data Flow

### Primary User Flow: `tome plugin enable <catalog>/<plugin>`

```
CLI parse (--json, -v, args)
       ↓
dispatch to plugin::enable::run()
       ↓
parse PluginId
       ↓
load config, paths
       ↓
resolve plugin directory (manifest-first)
       ↓
probe model presence + prompt user if missing (TTY only)
       ↓
load FastembedEmbedder
       ↓
lifecycle::enable(id, deps {embedder, allow_model_download=false, …})
  Step 2 → parse plugin.json (lenient)
  Step 3 → check already-enabled (exit 31)
  Step 4 → ensure models present (skipped: allow_model_download=false, already prompted)
  Step 5 → acquire advisory lock
  Step 6–9 → walk skills → collect PendingSkill (with frontmatter parse)
         → embed each (with SIGINT poll)
         → insert under one transaction
         → return EnableSummary
       ↓
format output (human: ✓ N skills / JSON: NDJSON record)
       ↓
exit(0)
```

### Plugin Disable Flow: `tome plugin disable <catalog>/<plugin>`

```
CLI parse
       ↓
dispatch to plugin::disable::run()
       ↓
parse PluginId, load config, paths
       ↓
resolve plugin directory
       ↓
(optional: confirm prompt if interactive)
       ↓
lifecycle::disable(id, paths, config, embedder_seed, reranker_seed)
  → acquire lock
  → mark_all_disabled_for_plugin() [flip enabled=0]
  → release lock
       ↓
format output
       ↓
exit(0)
```

### Query Flow: `tome query <text>`

```
CLI parse (--top-k, --catalog, --plugin, --no-rerank, --strict, --min-score, …)
       ↓
dispatch to query::run()
       ↓
validate filters (check catalogs exist)
       ↓
open index read-only
       ↓
check embedder drift (exit 41 if stale)
       ↓
check reranker drift (warn-only)
       ↓
check model presence (embedder required; reranker if not --no-rerank)
       ↓
load embedder + reranker (with spinners)
       ↓
embed query text
       ↓
KNN (candidate_k = top_k×4 if reranking)
       ↓
apply filters (--catalog, --plugin)
       ↓
optional rerank (cross-encoder logits)
       ↓
trim to top_k
       ↓
apply --strict threshold filter (or warn)
       ↓
render table (human) or NDJSON (JSON)
       ↓
exit(0)
```

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| **CLI** (`src/main.rs`, `src/cli.rs`) | Parse args, install signal handler, dispatch, map errors to exit codes. | Commands, logging, output. | Catalog, config, paths (indirectly via commands). |
| **Commands** (`src/commands/`) | Orchestrate catalog/plugin/query operations; call library logic and format output. | Lifecycle, catalog, config, paths, error, output, embedding, index, presentation. | Logging (by design; logging is orthogonal). |
| **Plugin Lifecycle** (`src/plugin/lifecycle.rs`) | Enable/disable orchestrator; compose metadata parsers, index, and embedding. | Plugin metadata (manifest, frontmatter), index (open, lock, enable_plugin_atomic), embedding (embedder trait, model registry), catalog (manifest reader). | Commands (reverse dependency only). |
| **Index** (`src/index/`) | SQLite operations, schema, KNN, drift detection, advisory locks. | rusqlite, sqlite-vec, index schema. | Commands, embedding, plugin (reverse dependency only). |
| **Embedding** (`src/embedding/`) | Model registry, download, trait implementations (fastembed wrapper, stub). | reqwest, ort, fastembed-rs, serde. | Commands, index (reverse dependency only). |
| **Catalog** (`src/catalog/`) | Git operations, manifest parsing, atomic persistence. | Git (process spawning), manifest (parsing), store (writes), config (types). | Commands (reverse dependency only). |
| **Git** (`src/catalog/git.rs`) | Spawn and manage git subprocesses; scrub credentials from all output. | `std::process`, regex, ctrlc. | Manifest, config, commands. |
| **Manifest** (`src/catalog/manifest.rs`) | Parse and validate TOML (strict) and JSON (lenient); enforce schema constraints. | serde, toml, serde_json, error types. | Git, store, commands. |
| **Store** (`src/catalog/store.rs`) | Atomic read/write of config files. | tempfile, std::fs, config types. | Git, manifest, commands. |
| **Config** (`src/config.rs`) | Define and serialize registry and catalog entry structures. | serde, toml, time (timestamps). | Catalog, commands (reverse dependency only). |
| **Paths** (`src/paths.rs`) | Resolve XDG directories and content-addressed cache keys; resolve index/model paths. | sha2, hex, std::env. | All other modules. |
| **Logging** (`src/logging.rs`) | Initialize tracing. | tracing, tracing-subscriber. | All modules (orthogonal; no dependencies into logging). |
| **Output** (`src/output.rs`) | Format results as human or JSON; detect TTY. | serde_json, std::io, error types. | No other modules (clean boundary). |
| **Presentation** (`src/presentation/`) | Table, progress, colour, prompt rendering; TTY and `NO_COLOR` awareness. | comfy-table, indicatif, owo-colors, inquire, std::io. | Commands (reverse dependency only). |
| **Error** (`src/error.rs`) | Define closed error enum and exit code mapping. | thiserror, std::path, anyhow. | No other modules (consumed by all). |

## Dependency Rules

1. **No cycles**: The dependency graph is a DAG. `main.rs` → `cli.rs` → `commands/` → `{plugin, index, embedding, catalog, config, paths, output, presentation, error}`.
2. **Library shapes**: `plugin::lifecycle` and `index::` are library-shaped (no CLI); they return structured outcomes (`EnableOutcome`, `DisableOutcome`, `Candidate` vec, `Scored` vec) that `commands/` layers format for output.
3. **Trait seams**: `Embedder` and `Reranker` traits decouple the library from model implementations; tests inject `StubEmbedder`.
4. **Error type at the root**: `error.rs` has no internal dependencies; all modules depend on it (or types it wraps).
5. **Orthogonal logging**: `logging.rs` is initialized at startup and orthogonal to `--json` mode. No module imports `logging`; the global subscriber is set up once in `main()`.
6. **Config types, not logic, in `config.rs`**: `config.rs` defines only data structures; I/O is in `store.rs`.
7. **Plugin-dir resolution is centralized**: `plugin::lifecycle::resolve_plugin_dir` is the single source of truth; re-exported to CLI handlers via `commands/plugin/mod.rs` to avoid cross-boundary imports.

## Key Interfaces & Contracts

| Interface | Purpose | Implementation |
|-----------|---------|-----------------|
| `TomeError` | Closed enum of all failure modes; exit codes are exhaustive. | `src/error.rs` |
| `CatalogManifest` | Schema for `tome-catalog.toml`; enforces strict parsing and semantic validation. | `src/catalog/manifest.rs` |
| `PluginManifest` | Schema for `plugin.json`; lenient parsing (unknown fields ignored). | `src/plugin/manifest.rs` |
| `SkillFrontmatter` | Parsed YAML header from `SKILL.md`; fallback logic for name/description. | `src/plugin/frontmatter.rs` |
| `PluginId` | Address `<catalog>/<plugin>`; `FromStr` implementation. | `src/plugin/identity.rs` |
| `PluginRecord` + `PluginStatus` | Display record for a plugin + tri-state status. | `src/plugin/mod.rs` |
| `Config` + `CatalogEntry` | Registry schema; persisted to `~/.config/tome/config.toml`. | `src/config.rs` |
| `Paths` | XDG-aware path resolution; index DB, lock, model paths. | `src/paths.rs` |
| `Git` | Facade for git operations; scrubs credentials from all output. | `src/catalog/git.rs` |
| `store::write_atomic` | Atomic file write for registry and cache mutations. | `src/catalog/store.rs` |
| `Embedder` + `Reranker` | Trait interfaces for embedding and reranking. | `src/embedding/mod.rs` |
| `FastembedEmbedder` + `FastembedReranker` | ONNX-backed implementations via `fastembed-rs` and `ort`. | `src/embedding/fastembed.rs` |
| `StubEmbedder` | Deterministic test double; produces SHA-derived vectors. | `src/embedding/stub.rs` (test-only by default, LTO-stripped from release). |
| `EnableOutcome` + `DisableOutcome` | Structured results of plugin lifecycle operations. | `src/plugin/lifecycle.rs` |
| `LifecycleDeps` | Dependency injection struct for `lifecycle::enable/disable`. | `src/plugin/lifecycle.rs` |
| `Candidate` + `Scored` | KNN result and scored result records. | `src/embedding/mod.rs` |
| `output::Mode` | Enum selecting human or JSON formatting. | `src/output.rs` |

## Signal Handling & Cancellation

**Mechanism**: Global `AtomicBool` flipped by `ctrlc` handler.

**Installation**: Once in `main.rs` via `git::install_signal_handler()` (idempotent).

**Polling**: Commands check `git::was_cancelled()` periodically or after long-running operations (git clone, skill walk, embedding loop, model download).

**Exit Code**: 8 (`TomeError::Interrupted`).

**Invariants**:
- In-flight child processes are killed.
- Atomic writes ensure partial state is not left on disk.
- Index transactions are rolled back on interruption (via `was_cancelled()` checks inside `enable_locked`).
- Tests can reset the flag via `git::reset_cancellation_for_tests()`.

## Atomic Writes & Concurrency

**Pattern**: Write to a temporary file in the same directory as the target, fsync, then rename.

**Locations**:
- Config persistence: `store::save()` → `store::write_atomic()`.
- Cache mutations: Temp dir cloned into, then atomically renamed to final location.
- Index mutations: SQLite WAL + advisory lockfile (`index.lock`). Mutating operations acquire the lock; read-only operations do not.
- Model persistence: Download to temp, verify checksum, rename.

**POSIX Atomicity**: On single filesystem, rename is atomic; readers either see the old or new version, never partial state.

**SQLite Concurrency**: WAL mode + 5s `busy_timeout` allows multiple readers + one writer. Advisory lockfile is a Tome-owned per-FD OS-level lock; held for the duration of index mutations.

**Tested**: `tests/atomicity.rs` verifies interruption injection; `tests/concurrency.rs` verifies two-process index contention.

## Cross-Cutting Concerns

| Concern | Implementation | Location |
|---------|----------------|----------|
| **Credential Scrubbing** | Regex rules applied to all captured git and reqwest output before display or error propagation. | `src/catalog/git.rs::scrub_credentials()`, `src/embedding/download.rs` |
| **Error Mapping** | Every `Result<_, TomeError>` eventually reaches `main()`, which maps to exit code. | `src/error.rs`, `src/main.rs` |
| **Logging & Verbosity** | Global tracing subscriber initialized once; orthogonal to `--json` mode. | `src/logging.rs`, `src/main.rs` |
| **TTY Detection** | Used by interactive commands (removal confirmation, model download prompt), progress spinners, and output formatting. | `src/output.rs::stdin_is_tty()`, `stdout_is_tty()`, `src/presentation/prompt.rs`, `src/presentation/progress.rs` |
| **Model Presence** | Two-stage check: manifest.json exists on disk AND parses. Applied before `enable` and `query`. | `src/plugin/mod.rs::model_manifest_ok()`, `src/embedding/download.rs` |
| **Path Validation** | Plugin sources in `tome-catalog.toml` are validated relative to catalog root (no `..`, no escape). Plugin source in `lifecycle::resolve_plugin_dir` is manifest-declared or flat-layout fallback. | `src/catalog/manifest.rs::validate_source()`, `src/plugin/lifecycle.rs::resolve_plugin_dir()` |
| **Drift Detection** | Embedder and reranker identity (name + version) stored in index meta; compared at query time. Embedder drift → hard fail (exit 41/42); reranker drift → warn. | `src/index/meta.rs`, `src/commands/query.rs::check_drift()` |

---

## What Does NOT Belong Here

- Directory structure details → STRUCTURE.md
- Technology versions → STACK.md
- External service configs → INTEGRATIONS.md
- Code style rules → CONVENTIONS.md

---

*This document describes HOW the system is organized. Keep focus on patterns and relationships.*

# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13 (Phase 3 User Story 1) + 2026-05-13 (Phase 4 User Story 2 — interactive browse) + 2026-05-13 (Phase 5 User Story 3 — plugin disable subcommand) + 2026-05-13 (Phase 6 User Story 4 slice 1 — models commands)

## Architecture Overview

Tome is a synchronous Rust CLI following a classic **parse → dispatch → execute → map-errors → exit** pipeline. The codebase is organized around a **capability-driven** modular architecture where each module owns a distinct responsibility (catalog management, Git operations, configuration, logging, path resolution, output formatting, plugin metadata parsing, skill indexing, model embedding, model lifecycle management, and interactive presentation). Error handling is centralized in a closed `TomeError` enum that enforces exhaustive exit-code mapping at compile time. Signal handling (SIGINT) is global and atomic, allowing long-running operations (git clone, model download, embedding) to be cancelled gracefully with a well-defined exit code.

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
| **Interactive Three-Level Loop Pattern** | Bare `tome plugin` (no subcommand) enters an interactive flow: `catalog_loop` → `plugin_loop` → `view_loop`, each with a `LoopExit` enum to handle Back/Quit unwinds and error propagation (Phase 4, User Story 2). |

## Core Components

### CLI & Parsing (`src/cli.rs`, `src/main.rs`)

- **Purpose**: Parse global flags (`--json`, `-v`/`-vv`) and dispatch to subcommand handlers.
- **Location**: `src/main.rs` (entry), `src/cli.rs` (clap derive definitions).
- **Dependencies**: `clap` (argument parsing), `catalog::git` (signal handler installation).
- **Dependents**: `commands/` modules (receive parsed args).
- **Pipeline Entry**: `main()` parses CLI → installs signal handler → dispatches to handler → maps result to exit code.
- **Phase 4 Change**: `PluginArgs` now wraps an `Option<PluginCommand>` to allow bare `tome plugin` with no subcommand. Routes to `commands::plugin::run_interactive()` when the command is `None`.
- **Phase 5 Change**: `PluginCommand` now includes `Disable(PluginDisableArgs { id: String, force: bool })` variant.
- **Phase 6 Change**: `ModelsCommand` enum added with `Download`, `List`, `Remove` variants; routes via `Command::Models(ModelsCommand)` to `commands::models::run()`.

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
- **Location**: `src/plugin/` (metadata parsers, lifecycle orchestrator), `src/commands/plugin/` (CLI handlers + interactive flow).
- **Dependencies**: `catalog::manifest` (read_catalog_manifest), `index::` (open DB, acquire lock, enable_plugin_atomic), `embedding::` (embedder + reranker, model registry, download).
- **Dependents**: Commands.
- **Key Patterns**:
  - `lifecycle::enable()`: parse manifest (exit 22) → check already-enabled (exit 31) → ensure models present (exit 30 unless allow_model_download) → acquire lock → walk skills → collect PendingSkill → embed + insert under one transaction (atomic per FR-004) → release lock.
  - `lifecycle::disable()`: check not-disabled (exit 32) → acquire lock → flip enabled=0 for all (catalog, plugin) rows → release lock. Cheap re-enable follows since embeddings are retained.
  - Frontmatter parse: delimiter error is fatal (exit 23); YAML-body error skips one skill + warn (FR-013c).
  - Models: embedder + reranker required by enable and query; optional download in `enable` (CLI owns the TTY prompt; `lifecycle::allow_model_download` is the decision).

### Plugin Disable Subcommand (`src/commands/plugin/disable.rs`)

- **Purpose**: Thin CLI wrapper over `plugin::lifecycle::disable`; owns confirmation-prompt UX (`--force` short-circuit, non-TTY refusal with pointer message).
- **Location**: `src/commands/plugin/disable.rs` (~108 lines).
- **Public Interface**: `pub fn run(args: PluginDisableArgs, mode: output::Mode) -> Result<(), TomeError>`.
- **Flow**:
  1. Parse `PluginId` from args.
  2. Load config, verify plugin exists (fail fast before prompt).
  3. If not `--force`, check TTY (non-TTY → emit pointer line to stderr, return `NotATerminal` exit 54).
  4. TTY: prompt with default "no" per spec. User decline → clean exit Ok(()) + optional stderr note.
  5. User accept or `--force`: call `lifecycle::disable()` (returns `DisableOutcome`).
  6. Emit human or JSON output.
- **Error Semantics**: Same exit codes as `lifecycle::disable` (exit 32 if already disabled). Non-TTY without `--force` → exit 54 (`NotATerminal`).
- **Pattern**: Mirrors `enable.rs` in structure (validate → prompt → call library → emit). No embedder construction — index-only UPDATE. Cheap re-enable tested via `tests/plugin_enable.rs::cheap_reenable_after_disable_invokes_embedder_zero_times`.

### Interactive Browse Flow (`src/commands/plugin/interactive.rs`)

- **Purpose**: Bare `tome plugin` (no subcommand) — provide an interactive catalog → plugin → action browse loop (Phase 4, User Story 2).
- **Location**: `src/commands/plugin/interactive.rs` (~515 lines).
- **Public Interface**: `pub fn run_interactive(mode: output::Mode) -> Result<(), TomeError>`.
- **Loop Architecture** (three levels):
  1. **`catalog_loop()`**: Display catalog selector; user picks one or Quit.
  2. **`plugin_loop(catalog_name)`**: Browse plugins in the selected catalog; user picks one or Back.
  3. **`view_loop(id, plugin_manifest)`**: Display plugin view (mirrors `plugin show`); user selects action: Enable, Disable, or Back.
- **Control Flow**: Each loop level uses a private `LoopExit` enum to encode:
  - `Continue` → advance to next level.
  - `Back` → unwind to previous level.
  - `Quit` → clean exit with `Ok(())` (exit 0).
- **Error Handling**:
  - Enable/disable errors propagate verbatim (same exit codes as non-interactive subcommands).
  - User cancellation (Esc / Ctrl-C from `inquire` prompts) surfaces as `TomeError::Interrupted`, which is trapped and translated to `Ok(())` per the contract ("always exits 0 on clean exit").
- **TTY Enforcement** (FR-051): Non-TTY invocation via `presentation::prompt::select()` will refuse with `NotATerminal`, propagating as exit code 98 (FR-022a).

### Model Lifecycle Management (`src/commands/models/`)

- **Purpose**: Provide user-facing CLI for managing downloaded model artefacts (download, list, remove).
- **Location**: `src/commands/models/` (dispatch + per-subcommand handlers).
- **Dependencies**: `embedding::registry` (MODEL_REGISTRY, ModelEntry), `embedding::download` (download_model, sha256_file), `paths` (models_dir), `presentation` (tables, progress, prompts).
- **Dependents**: CLI main dispatcher.
- **Subcommands** (Phase 6, User Story 4):
  - **`download`** (`src/commands/models/download.rs`): Iterate MODEL_REGISTRY, skip if manifest exists and valid unless `--force`, atomic download via `embedding::download::download_model` with indicatif spinner. Emits human or NDJSON per mode.
  - **`list`** (`src/commands/models/list.rs`): Cheap path = check manifest + file existence + size; `--verify` flag rehashes via `embedding::download::sha256_file`. Renders ModelState (Ok / Missing / Corrupt / ChecksumMismatched) as table (human) or NDJSON (JSON).
  - **`remove`** (`src/commands/models/remove.rs`): Check model is not in use by any enabled plugin, check model exists (exit 30 if missing), confirm prompt with `--force` short-circuit, non-TTY without `--force` → exit 54 with pointer message. Deletes manifest first, then directory.
- **Shared Pattern**: Mirrors `plugin/` module layout (per-subcommand file under group `mod.rs` that dispatches). Owns user-facing UX (prompts, spinners, table rendering). Calls library functions (`embedding::download`, `embedding::registry`) for the actual work.

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
  - **Phase 6 addition**: `pub fn sha256_file(path) -> Result<String, TomeError>` streaming SHA-256 helper for `models list --verify`.

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
- **Phase 6 Change**: Relaxed `write_json` to accept `T: Serialize + ?Sized` for JSON serialization flexibility.

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
check plugin exists (fail-fast before prompt)
       ↓
if not --force:
  → check TTY (non-TTY → emit pointer, return NotATerminal exit 54)
  → confirm prompt (default "no"; abort → Ok(()), no state change)
       ↓
lifecycle::disable(id, paths, config, embedder_seed, reranker_seed)
  → acquire lock
  → mark_all_disabled_for_plugin() [flip enabled=0]
  → release lock
  → return DisableOutcome (skills_retained count)
       ↓
format output (human: ✓ disabled N records / JSON: NDJSON record)
       ↓
exit(0)
```

### Interactive Browse Flow: `tome plugin` (no subcommand)

```
CLI parse (bare Command::Plugin with option=None)
       ↓
dispatch to plugin::run_interactive()
       ↓
check TTY (inquire will refuse non-TTY as NotATerminal)
       ↓
catalog_loop():
  → present Select over catalog names + Quit option
  → user picks catalog or Quit
  → Quit → return Ok(()) → exit(0)
  → catalog picked → call plugin_loop(catalog_name)
       ↓
plugin_loop(catalog_name):
  → load index, walk (catalog, plugin) pairs
  → present Select over plugin names + Back option
  → user picks plugin or Back
  → Back → return to catalog_loop
  → plugin picked → call view_loop(id, manifest)
       ↓
view_loop(id, manifest):
  → render plugin view (as in `plugin show`)
  → present Select: [Enable | Disable | Back]
  → Enable → call enable::run(id)
         → on success, redraw plugin view (loop within view)
         → on error, propagate (exit with non-zero code)
  → Disable → confirm prompt, call lifecycle::disable()
         → on success, redraw plugin view
         → on error, propagate
  → Back → return to plugin_loop
       ↓
Esc / Ctrl-C at any level:
  → inquire surfaces as TomeError::Interrupted
  → trap and convert to Ok(())
  → exit(0)
```

### Model Lifecycle Flow: `tome models download | list | remove`

```
CLI parse (--json, -v, subcommand-specific args)
       ↓
dispatch to models::{download,list,remove}::run()

Download subcommand:
  → iterate MODEL_REGISTRY
  → for each model:
      → check if manifest exists && is valid
      → if missing or --force:
          → show indicatif spinner
          → call embedding::download::download_model()
          → emit human line or NDJSON record
  → exit(0)

List subcommand:
  → iterate MODEL_REGISTRY
  → for each model:
      → cheap check: manifest exists + files exist + correct sizes
      → if --verify: stream SHA-256 via sha256_file()
      → compute ModelState (Ok / Missing / Corrupt / ChecksumMismatched)
  → render comfy-table (human) or NDJSON (JSON)
  → exit(0)

Remove subcommand:
  → parse model name
  → check model exists (exit 30 if missing)
  → check no enabled plugins use it
  → if not --force: check TTY, prompt (non-TTY → exit 54 with pointer)
  → delete manifest, then directory
  → emit human line or NDJSON record
  → exit(0)
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
| **Commands** (`src/commands/`) | Orchestrate catalog/plugin/query/models operations; call library logic and format output. | Lifecycle, catalog, config, paths, error, output, embedding, index, presentation. | Logging (by design; logging is orthogonal). |
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
8. **Interactive flow is command-layer only**: `commands/plugin/interactive.rs` uses presentation layer (prompts, tables), command handlers (enable, disable), and lifecycle/config APIs. It is test-driven via `rexpect` pty harness (`tests/plugin_interactive.rs`) rather than unit-test injection.
9. **Models commands mirror plugin commands layout**: `commands/models/` follows the same per-subcommand-file + group-dispatcher pattern as `commands/plugin/`. Library-side work (download, SHA-256) lives in `embedding/download.rs` and `embedding/registry.rs`; CLI-side work (prompts, progress, tables) lives in `commands/models/{download,list,remove}.rs`.

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
| `LoopExit` | Private enum in `interactive.rs` encoding Back/Quit/Continue state. | `src/commands/plugin/interactive.rs` |
| `ModelState` | Classification of a registered model's on-disk install state (Ok / Missing / Corrupt / ChecksumMismatched). | `src/commands/models/mod.rs` |

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
| **TTY Detection** | Used by interactive commands (removal confirmation, model download prompt, interactive browse), progress spinners, and output formatting. | `src/output.rs::stdin_is_tty()`, `stdout_is_tty()`, `src/presentation/prompt.rs`, `src/presentation/progress.rs`, `src/commands/plugin/interactive.rs`, `src/commands/models/remove.rs` |
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

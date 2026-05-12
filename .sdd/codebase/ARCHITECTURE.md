# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-11

## Architecture Overview

Tome is a synchronous Rust CLI following a classic **parse → dispatch → execute → map-errors → exit** pipeline. The codebase is organized around a **capability-driven** modular architecture where each module owns a distinct responsibility (catalog management, Git operations, configuration, logging, path resolution, output formatting). Error handling is centralized in a closed `TomeError` enum that enforces exhaustive exit-code mapping at compile time. Signal handling (SIGINT) is global and atomic, allowing long-running operations (git clone) to be cancelled gracefully with a well-defined exit code.

## Architecture Pattern

| Pattern | Description |
|---------|-------------|
| **Sync-only CLI** | No async runtime (`tokio`). All I/O and process orchestration use `std::process` and blocking calls. |
| **Closed Error Set** | All failure paths map to a single `TomeError` enum with explicit exit codes; no `Other` or `Unknown` arms. Adding a failure mode requires specification, error type, and test updates. |
| **Atomic Writes** | Registry mutations and cache operations use `tempfile` + rename for POSIX atomicity; interruptions cannot corrupt state. |
| **Capability-Organized Modules** | Modules group related functionality: `catalog/` (manifest + Git + store), `commands/` (CLI handlers), `config/` (manifest deserialization), `paths/` (XDG resolution), `logging/` (tracing setup), `output/` (human/JSON formatting), `error/` (closed error enum). |
| **Credential Scrubbing at Boundary** | All captured `git` stderr/stdout passes through `git::scrub_credentials` before reaching logging, error display, or structured output. |

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

- **Purpose**: Parse `tome-catalog.toml` and validate its structure and semantic constraints.
- **Location**: `src/catalog/manifest.rs`.
- **Schema Enforcement**: `#[serde(deny_unknown_fields)]` on every struct; unknown fields produce `ManifestInvalid::UnknownField`.
- **Validation Pipeline**:
  1. UTF-8 decode.
  2. TOML syntax parse.
  3. Required field check (name, description, version, owner.name, owner.email).
  4. Semantic validation (semver version, valid email).
  5. Unique plugin names.
  6. Relative-path plugin sources (no `..`, no absolute paths, no URLs, must resolve within catalog).
- **Error Propagation**: Each failure produces a specific `ManifestInvalid` variant that maps to exit code 5.

### Atomic Registry Store (`src/catalog/store.rs`)

- **Purpose**: Persist and load `config.toml` atomically; prevent corruption from interruptions.
- **Location**: `src/catalog/store.rs`.
- **Atomicity Strategy**: Write to a temp file in the same directory as the target, `fsync`, then rename (POSIX-atomic on single filesystem).
- **Key Functions**:
  - `load(config_file)`: Load and parse TOML; return empty Config if file missing.
  - `save(config_file, config)`: Serialize to TOML and write atomically.
  - `write_atomic(target, bytes)`: Low-level atomic write (used by cache operations).

### Configuration (`src/config.rs`)

- **Purpose**: Define `Config` and `CatalogEntry` structures; serialize/deserialize via `serde` + `toml`.
- **Location**: `src/config.rs`.
- **Key Types**:
  - `Config`: Top-level document; keyed by catalog display name (BTreeMap for deterministic ordering).
  - `CatalogEntry`: Name, URL, tracked ref, local path, last-synced timestamp.
- **Strict Parsing**: `#[serde(deny_unknown_fields)]` on all structs.

### Path Resolution (`src/paths.rs`)

- **Purpose**: Resolve XDG-aware configuration and data directories; compute content-addressed cache keys.
- **Location**: `src/paths.rs`.
- **XDG Compliance**: Honour `XDG_CONFIG_HOME`, `XDG_DATA_HOME`, fall back to `~/.config` and `~/.local/share`.
- **Cache Addressing**: `cache_dir_for(url)` returns `~/.local/share/tome/catalogs/<sha256(url)>/`.

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
- **Variants** (8 total):
  - `Internal(anyhow::Error)` → exit 1 (programmer-facing surprise, caught panic).
  - `Usage(String)` → exit 2 (bad CLI usage).
  - `CatalogNotFound(String)` → exit 3.
  - `CatalogAlreadyExists(String)` → exit 4.
  - `ManifestInvalid(ManifestInvalid)` → exit 5.
  - `GitFailed { catalog, detail }` → exit 6.
  - `Io(std::io::Error)` → exit 7.
  - `Interrupted` → exit 8 (SIGINT).
- **Compile-Time Enforcement**: The `TomeError::exit_code()` method is exhaustive; adding a variant forces edits to `tests/exit_codes.rs`, the spec, and the PRD.

## Data Flow

### Primary User Flow: `tome catalog add`

```
CLI parse (--json, -v, args)
       ↓
install_signal_handler()
       ↓
dispatch to catalog::add::run()
       ↓
resolve source (owner/repo → GitHub URL, or file://, or absolute Git URL)
       ↓
compute cache path (sha256(url))
       ↓
load config from ~/.config/tome/config.toml
       ↓
create temp dir in ~/.local/share/tome/catalogs/
       ↓
git::clone_shallow(url, temp, ref) [captures stderr → scrub_credentials]
       ↓
read manifest from cloned repo
       ↓
manifest::parse_and_validate() [strict TOML → semantic checks]
       ↓
atomic rename temp → final cache path
       ↓
persist new CatalogEntry to config.toml (atomic write)
       ↓
output success (human or JSON)
       ↓
exit(0)
```

### Cancellation Flow

At any point in the above, if SIGINT is received:
1. `ctrlc` handler flips `git::CANCELLED` atomic bool.
2. Next poll of `was_cancelled()` returns true.
3. In-flight child processes are killed.
4. `TomeError::Interrupted` is returned.
5. Error is formatted and written to stderr.
6. Process exits with code 8.

### Refresh Flow: `tome catalog update`

```
load config
       ↓
for each catalog (sequentially):
  git::fetch (captures stderr → scrub)
  ↓
  atomic rename fresh tree into place
  ↓
  update last_synced timestamp
  ↓
  save config
       ↓ (on first error, stop and exit)
  output success
```

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| **CLI** (`src/main.rs`, `src/cli.rs`) | Parse args, install signal handler, dispatch, map errors to exit codes. | Commands, logging, output. | Catalog, config, paths (indirectly via commands). |
| **Commands** (`src/commands/`) | Orchestrate catalog operations (add, remove, list, update, show); call catalog logic and format output. | Catalog, config, paths, error, output. | Logging (by design; logging is orthogonal). |
| **Catalog** (`src/catalog/`) | Git operations, manifest parsing, atomic persistence. | Git (process spawning), manifest (parsing), store (writes), config (types). | Commands (reverse dependency only). |
| **Git** (`src/catalog/git.rs`) | Spawn and manage git subprocesses; scrub credentials from all output. | `std::process`, regex, ctrlc. | Manifest, config, commands. |
| **Manifest** (`src/catalog/manifest.rs`) | Parse and validate TOML; enforce schema constraints. | serde, toml, error types. | Git, store, commands. |
| **Store** (`src/catalog/store.rs`) | Atomic read/write of config files. | tempfile, std::fs, config types. | Git, manifest, commands. |
| **Config** (`src/config.rs`) | Define and serialize registry and catalog entry structures. | serde, toml, time (timestamps). | Catalog, commands (reverse dependency only). |
| **Paths** (`src/paths.rs`) | Resolve XDG directories and content-addressed cache keys. | sha2, hex, std::env. | All other modules. |
| **Logging** (`src/logging.rs`) | Initialize tracing. | tracing, tracing-subscriber. | All modules (orthogonal; no dependencies into logging). |
| **Output** (`src/output.rs`) | Format results as human or JSON; detect TTY. | serde_json, std::io, error types. | No other modules (clean boundary). |
| **Error** (`src/error.rs`) | Define closed error enum and exit code mapping. | thiserror, std::path, anyhow. | No other modules (consumed by all). |

## Dependency Rules

1. **No cycles**: The dependency graph is a DAG. `main.rs` → `cli.rs` → `commands/` → `catalog/`, `config/`, `paths/`, `output/`, `error/`.
2. **Error type at the root**: `error.rs` has no internal dependencies; all modules depend on it (or types it wraps).
3. **Orthogonal logging**: `logging.rs` is initialized at startup and orthogonal to `--json` mode. No module imports `logging`; the global subscriber is set up once in `main()`.
4. **Config types, not logic, in `config.rs`**: `config.rs` defines only data structures; I/O is in `store.rs`.

## Key Interfaces & Contracts

| Interface | Purpose | Implementation |
|-----------|---------|-----------------|
| `TomeError` | Closed enum of all failure modes; exit codes are exhaustive. | `src/error.rs` |
| `CatalogManifest` | Schema for `tome-catalog.toml`; enforces strict parsing and semantic validation. | `src/catalog/manifest.rs` |
| `Config` + `CatalogEntry` | Registry schema; persisted to `~/.config/tome/config.toml`. | `src/config.rs` |
| `Paths` | XDG-aware path resolution and cache key computation. | `src/paths.rs` |
| `Git` | Facade for git operations; scrubs credentials from all output. | `src/catalog/git.rs` |
| `store::write_atomic` | Atomic file write for registry and cache mutations. | `src/catalog/store.rs` |
| `output::Mode` | Enum selecting human or JSON formatting. | `src/output.rs` |

## Signal Handling & Cancellation

**Mechanism**: Global `AtomicBool` flipped by `ctrlc` handler.

**Installation**: Once in `main.rs` via `git::install_signal_handler()` (idempotent).

**Polling**: Commands check `git::was_cancelled()` periodically or after long-running operations.

**Exit Code**: 8 (`TomeError::Interrupted`).

**Invariants**:
- In-flight child processes are killed.
- Atomic writes ensure partial state is not left on disk.
- Tests can reset the flag via `git::reset_cancellation_for_tests()`.

## Atomic Writes

**Pattern**: Write to a temporary file in the same directory as the target, fsync, then rename.

**Locations**:
- Config persistence: `store::save()` → `store::write_atomic()`.
- Cache mutations: Temp dir cloned into, then atomically renamed to final location.

**POSIX Atomicity**: On single filesystem, rename is atomic; readers either see the old or new version, never partial state.

**Tested**: `tests/atomicity.rs` verifies that interruption injection does not corrupt persisted state.

## Cross-Cutting Concerns

| Concern | Implementation | Location |
|---------|----------------|----------|
| **Credential Scrubbing** | Regex rules applied to all captured git output before display or error propagation. | `src/catalog/git.rs::scrub_credentials()` |
| **Error Mapping** | Every `Result<_, TomeError>` eventually reaches `main()`, which maps to exit code. | `src/error.rs`, `src/main.rs` |
| **Logging & Verbosity** | Global tracing subscriber initialized once; orthogonal to `--json` mode. | `src/logging.rs`, `src/main.rs` |
| **TTY Detection** | Used by interactive commands (removal confirmation) and output formatting. | `src/output.rs::stdin_is_tty()`, `stdout_is_tty()` |
| **Path Validation** | Manifest plugin sources are validated relative to catalog root (no `..`, no escape). | `src/catalog/manifest.rs::validate_source()` |

---

## What Does NOT Belong Here

- Directory structure details → STRUCTURE.md
- Technology versions → STACK.md
- External service configs → INTEGRATIONS.md
- Code style rules → CONVENTIONS.md

---

*This document describes HOW the system is organized. Keep focus on patterns and relationships.*

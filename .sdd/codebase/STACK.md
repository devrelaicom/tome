# Technology Stack

> **Purpose**: Document what executes in this codebase - languages, runtimes, frameworks, and critical dependencies.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13 (Phase 5 User Story 2 — `tome plugin disable` subcommand + re-enable verification)
> **Updated**: 2026-05-13 (Phase 6 User Story 4 slice 1 — `tome models download | list | remove` CLI; slice 2 adds 9 integration tests)
> **Updated**: 2026-05-13 (Phase 7 User Story 5 slices 1–3 — `reindex_plugin_atomic` + `tome catalog update` cascade + `tome reindex` CLI; no new dependencies)
> **Updated**: 2026-05-13 (Phase 8 User Story 6 slices 1–2 — `tome status [--verify]` + version pre-parse hook; 14 new tests; no new dependencies)

## Languages & Runtimes

| Language | Version | Purpose |
|----------|---------|---------|
| Rust | stable (MSRV: 1.93) | Primary implementation language; synchronous (no async runtime in Phase 1–8) |

## Frameworks

Phase 1–8 is a CLI application, not a web framework-based project.

| Framework | Version | Purpose |
|-----------|---------|---------|
| clap | 4.x | CLI argument parsing and help generation; bare `tome plugin` (no subcommand) dispatches to interactive flow via `Option<PluginCommand>` derive pattern; `--version` intercepted by pre-parse hook in `main.rs` to honour `--json` and include embedder/reranker identities |

## Critical Dependencies

| Package | Version | Purpose | Usage Scope |
|---------|---------|---------|-------------|
| `serde` + `serde_derive` | 1.x | Configuration and manifest (de)serialisation | All TOML parsing for `config.toml` and `tome-catalog.toml`; Tome-owned structs use `#[serde(deny_unknown_fields)]` (FR-013a boundary) |
| `toml` | 0.8 | TOML format support | Manifest and config file parsing |
| `thiserror` | 2.x | Typed error enums | Closed `TomeError` enum in `src/error.rs` (all fallible operations); 18+ enumerated failure variants with dedicated exit codes |
| `anyhow` | 1.x | Error context chaining | Application-level error wrapping at boundaries |
| `tracing` + `tracing-subscriber` | 0.1, 0.3 | Structured logging to stderr | Diagnostic output orthogonal to `--json` stdout |
| `sha2` | 0.10 | Content-addressed cache naming and model integrity | URL hashing for `cache_dir_for()` in `src/paths.rs`; model download verification in `src/embedding/download.rs` |
| `regex` | 1.x | Credential scrubbing patterns | Git stderr sanitisation in `src/catalog/git.rs` (4 regex patterns); extended in Phase 3 to cover model download URLs (principle XIII) |
| `ctrlc` | 3.x | Signal handling (SIGINT) | Global cancellation handler with exit code 8; SIGINT cancels in-flight git operations and model downloads |
| `tempfile` | 3.x | Atomic file writes | Registry, per-catalog cache, models directory, and manifest mutations (atomicity boundary: rename-based) |
| `hex` | 0.4 | Hex encoding for SHA256 digests | Cache directory naming alongside sha2; model checksum comparison in `src/embedding/download.rs` |
| `semver` | 1.x | Semantic version parsing | Catalog manifest version field validation |
| `time` | 0.3 | Timestamp formatting and parsing | Logging and manifest timestamps; RFC 3339 serialisation for `ModelManifest.installed_at` |
| `serde_json` | 1.x | JSON serialisation (NDJSON output) | `--json` mode formatting for stdout; `ModelManifest` serialisation to `manifest.json` |

### Phase 2 — foundational (no user-facing CLI wired until Phase 3)

| Package | Version | Purpose | Usage Scope |
|---------|---------|---------|-------------|
| `rusqlite` | 0.32 (`bundled`) | Embedded SQLite (statically linked, no system dep) | `src/index/*` — the local skill index database; WAL mode + advisory lockfile (FR-040) |
| `sqlite-vec` | vendored (v0.1.9) | KNN vector search extension for SQLite | `vendor/sqlite-vec/` compiled by `build.rs`; loaded via `sqlite3_auto_extension` in `src/index/vec_ext.rs` |
| `serde_yaml` | 0.9 | Lenient YAML frontmatter parsing for third-party `SKILL.md` | `src/plugin/frontmatter.rs` — parses upstream metadata leniently (FR-013a boundary; does not validate unknown fields) |
| `fastembed` | 4.x | ONNX-backed text embedding + reranking | `src/embedding/fastembed.rs` — loads BGE models from `${XDG_DATA_HOME}/tome/models/` at runtime; CPU execution provider only |
| `reqwest` | 0.12 (`blocking`, `rustls-tls`, no defaults) | Synchronous HTTPS for model downloads | `src/embedding/download.rs` — downloads `MODEL_REGISTRY` entries with SHA-256 verification and atomicity |
| `indicatif` | 0.17 | Progress bars + spinners (TTY-aware) | `src/presentation/progress.rs` — download/reindex progress; refuses on non-TTY |
| `comfy-table` | 7.x | Table rendering for human-mode list/show output | `src/presentation/tables.rs` — `tome plugin list`, `tome models list`, query results |
| `owo-colors` | 4.x | Terminal colours with native `NO_COLOR` support | `src/presentation/colour.rs` — colourised output respecting `NO_COLOR` environment variable (principle I) |
| `inquire` | 0.7 (`crossterm`, no defaults) | Interactive Select/MultiSelect/Confirm prompts | `src/presentation/prompt.rs` — interactive plugin enable/disable/list/show; bare `tome plugin` browse flow; `--force` flag can skip confirmation in disable; refuses on non-TTY (principle III) |
| `cc` (build-dep) | 1.x | C compiler driver for the vendored sqlite-vec amalgamation | `build.rs` only |

ONNX Runtime (`ort`) is a transitive dependency through `fastembed`; Tome does
not link it directly. `src/embedding/runtime.rs` is a stub placeholder (Phase 2 foundational),
becoming load-bearing only if a direct dependency is added.

### Phase 3 — user-stories (slice 1 landed)

Phase 3 wires the Phase 2 foundational pieces into user-facing CLI surfaces:
- `tome plugin enable | disable | list | show` — lifecycle orchestrator in `src/plugin/lifecycle.rs`
- `tome query` — KNN search with optional reranking in `src/commands/query.rs`
- Model registry now carries real upstream SHA-256 digests and file sizes (no longer all-zero placeholders)
- Test helper `StubEmbedder::with_force_fail_after(n)` added to `src/embedding/stub.rs` for atomicity testing

No new production dependencies in Phase 3 slice 1 — all pieces are Phase 2 additions wired through Phase 1 plumbing.

### Phase 4 — user-story slice 1 (interactive browse flow)

Phase 4 slice 1 ships the bare `tome plugin` interactive CLI surface:
- `src/commands/plugin/interactive.rs::run_interactive` orchestrates catalog → plugin → action flow
- Uses `inquire` `Select`, `Confirm`, and terminal detection (existing production dep, no new additions)
- Test-driven via `rexpect` pty harness in `tests/plugin_interactive.rs` (dev-only, Unix-only)

| Package | Version | Purpose | Usage Scope | Phase 4 |
|---------|---------|---------|-------------|---------|
| `rexpect` | 0.7 | Unix pty harness for interactive CLI testing | `tests/plugin_interactive.rs` only; drives the interactive flow through a real pseudoterminal | dev-dep; no runtime impact |

No new production dependencies in Phase 4 slice 1 — `rexpect` is test-only and does not compile into the release binary.

### Phase 5 — user-story slice 2 (disable subcommand + verification)

Phase 5 slice 2 adds the `tome plugin disable <id>` subcommand with cheap re-enable verification:
- `src/commands/plugin/disable.rs` (~108 lines) — CLI wrapper over `plugin::lifecycle::disable`
- New `PluginCommand::Disable(PluginDisableArgs { id, force })` variant in `src/cli.rs`
- Dispatch wired in `src/commands/plugin/mod.rs`
- Confirmation UX reuses existing `inquire` (Phase 2); `--force` flag short-circuits the prompt
- Test coverage: `tests/plugin_disable.rs` (~190 lines, CLI binary); `tests/plugin_repeated.rs` (~120 lines, library + CLI hybrid) for enable/disable/enable cycle

No new production dependencies in Phase 5 slice 2 — all pieces reuse Phase 1–4 infrastructure (`inquire` for confirmation, existing lifecycle plumbing).

### Phase 6 — user-story slice 1 (explicit model management)

Phase 6 slice 1 adds explicit model artefact CLI management:
- `src/commands/models/download.rs`, `list.rs`, `remove.rs` (~360 lines total) — `tome models {download,list,remove}` subcommands
- New `ModelsCommand::Download | List | Remove` variants in `src/cli.rs`
- Dispatch wired in `src/commands/models/mod.rs`
- Helper `embedding::download::sha256_file(path) -> Result<String, TomeError>` promoted to `pub` for content verification in list
- Signature relaxation: `output::write_json<T: Serialize + ?Sized>` (adds `?Sized` bound to serialize slice types in JSON output)

No new production dependencies in Phase 6 slice 1 — all pieces reuse Phase 1–5 infrastructure (progress bars, tables, JSON formatting).

Phase 6 slice 2 adds 9 integration tests across `tests/models_{download,list,remove}.rs` using sparse-file pattern for staging 280 MB artefacts at zero disk cost. Helpers `fabricate_installed_model` and `fabricate_all_installed_models` added to `tests/common/mod.rs`. No production code or dependency changes.

### Phase 7 — user-story slices 1–3 (reindex: library orchestrator, catalog integration, CLI)

Phase 7 slice 1 introduces the reindex library orchestrator:
- `reindex_plugin_atomic(id, deps, force)` in `src/index/skills.rs` — mirrors `enable_plugin_atomic`, atomically re-embeds skills with `ReindexSummary` outcome (added/modified/removed/unchanged breakdown).
- `ReindexOutcome`, `pub fn reindex_plugin(id, deps, force)`, `pub fn auto_disable_orphan(id, deps)` in `src/plugin/lifecycle.rs`.
- **Bugfix**: `upsert_skill` latent issue — `sqlite-vec` virtual table does NOT support `INSERT OR REPLACE`. Switched to `DELETE`-then-`INSERT` pattern.
- 9 new unit tests in `src/index/` and `src/plugin/`.

Phase 7 slice 2 wires reindex into `tome catalog update`:
- `pub fn enabled_plugins_for_catalog(catalog, conn)` in `src/index/skills.rs` — filters enabled plugins for a given catalog.
- `pub fn reindex_catalog_plugins(catalog, deps)` in `src/commands/catalog/update.rs` with `CatalogReindexOutcome` + `PluginChange` struct.
- `commands/catalog` module promoted to `pub mod` for downstream access.
- Lazy `FastembedEmbedder` loading — only instantiated when an enabled plugin exists in a refreshed catalog.
- Auto-disable cascades on `PluginNotFound` / `PluginManifestParseError`.
- 3 new integration tests via library API.

Phase 7 slice 3 adds the `tome reindex` CLI subcommand:
- `pub fn run(args: ReindexArgs, mode: Mode)` in `src/commands/reindex.rs` (new file).
- Scope grammar: omitted (all enabled plugins) | `<catalog>` (all enabled in one catalog) | `<catalog>/<plugin>` (exactly one plugin).
- `--force` flag for FR-016 recovery (re-embed unchanged skills).
- `Command::Reindex(ReindexArgs)` variant in `src/cli.rs`.
- `commands/reindex` module added to `src/commands/mod.rs`.
- Lazy `FastembedEmbedder` — only loaded if reindex scope contains enabled plugins.
- `Scope` enum, `ReindexAggregate` outcome (duration, skills processed, outcome categories).
- `pub fn run_with_deps` for library-API testing (mirrors `enable_plugin_with_deps`).
- 7 new tests: 4 library-API (scope parsing, resolve targets, aggregate output) + 3 CLI binary error paths (invalid scope, no plugins in scope, bad flags).

**No new production dependencies** across Phase 7 slices 1–3 — all pieces reuse Phase 1–6 infrastructure (lifecycle, reindex logic sits in existing orchestrator; CLI uses existing tables/progress/JSON formatters). Test count: 204 → 213 → 216 → 223 across 33 suites.

### Phase 8 — user-story slices 1–2 (status health check + version pre-parse)

Phase 8 slice 1 ships the `tome status [--verify]` read-only health check subcommand:
- `src/commands/status.rs` (~330 lines) — per-subsystem diagnostics (models, index, drift detection via `detect_drift` in `src/index/meta.rs`)
- New `Command::Status(StatusArgs)` and `StatusArgs { verify: bool }` in `src/cli.rs`
- Dispatch wired in `src/commands/plugin/mod.rs`
- Helpers `ModelState`, `cheap_state`, `read_manifest`, `primary_file_path`, `human_mb` promoted from `pub(crate)` to `pub` in `src/commands/models/mod.rs` for reuse
- Lazy drift detection — skipped unless `--verify` is set
- Exit semantics: 0 when healthy; 1 when degraded (reranker-only) or unhealthy (anything else); report always rendered before exit

Phase 8 slice 2 adds version pre-parse hook in `src/main.rs`:
- Clap's auto `--version` disabled via `disable_version_flag = true` on `Cli` derive
- Pre-parse hook detects `--version` / `-V` in `std::env::args()` before clap dispatch
- Delegates to `commands::status::print_version(json)` to honour `--json` flag and include embedder/reranker identities (per contract `contracts/version-output.md`)
- Short-circuits to `std::process::exit(0)` after printing
- Test coverage: `tests/status.rs` (10 tests covering health report variants, JSON mode, exit codes) + `tests/version_output.rs` (4 tests covering flag detection and embedder/reranker output)
- Helper `registry_seeds` in `src/commands/plugin/mod.rs` promoted from `pub(crate)` to `pub` for test bootstrapping

**No new production dependencies** in Phase 8 — all pieces reuse Phase 1–7 infrastructure (status logic combines existing model/index/meta logic; version printing is a thin wrapper over embedder registry entries). Test count: 223 → 237 across 33 → 35 suites.

## Package Managers & Build Tools

| Tool | Version | Purpose |
|------|---------|---------|
| Cargo | (bundled with Rust) | Workspace management and builds |
| rustfmt | (pinned in rust-toolchain.toml) | Code formatting |
| clippy | (pinned in rust-toolchain.toml) | Linting with `-D warnings` (enforced in pre-commit and CI) |

## Runtime Environment

| Environment | Details |
|-------------|---------|
| OS Targets | Linux (ubuntu-latest) and macOS (macos-latest) — CI verified on both |
| Deployment | Single binary (`target/release/tome`); installed via `cargo install --path .` |
| Binary Size | < 50 MB stripped (enforced by CI; revised from 10 MB ceiling in CONSTITUTION v1.2.0 after Phase 3 slice 1 measured 29.56 MB on Linux; `ort` CPU-only static linking is the load-bearing constraint) |
| Output | Human-readable (default) or NDJSON (`--json`); logging to stderr only (orthogonal to stdout); colours respect `NO_COLOR` and auto-disable on non-TTY |
| Model runtime | CPU-only ONNX Runtime (via `fastembed`); models downloaded at first use into `${XDG_DATA_HOME}/tome/models/`; fixed registry (compile-time constants) ensures bit-for-bit reproducibility |

## Not Used (Explicitly Excluded)

- **Async runtime**: No `tokio`, `async-std`, or similar. Phase 1–8 remains synchronous (`reqwest::blocking`, `rusqlite`, `fastembed`); the MCP server is the future forcing function.
- **Git library**: No `libgit2`, `git2`, or vendored Git. `std::process::Command` shells out to system `git` (constitution principle XII).
- **Direct ONNX Runtime dep**: `ort` is reached transitively through `fastembed` only; no direct linkage from Tome code.
- **Custom npm/cargo registry overrides**: All packages resolve from public registries.
- **Async database drivers** (e.g., `sqlx`): `rusqlite` is synchronous, suitable for a CLI with no concurrent connections (FR-040).

---

## What Does NOT Belong Here

- Directory structure → STRUCTURE.md
- System design patterns → ARCHITECTURE.md
- External service integrations → INTEGRATIONS.md
- Dev tools (linting, formatting) → CONVENTIONS.md
- Test frameworks → TESTING.md

---

*This document captures only what executes. It reflects the actual Cargo.toml, Cargo.lock, and Phase 1–8 source code.*

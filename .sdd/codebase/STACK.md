# Technology Stack

> **Purpose**: Document what executes in this codebase - languages, runtimes, frameworks, and critical dependencies.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-14 (Phase 3 Polish PRs #51–#58 — final ship as v0.3.0; 399 tests across 53 suites; no new dependencies)

## Languages & Runtimes

| Language | Version | Purpose |
|----------|---------|---------|
| Rust | stable (MSRV: 1.93) | Primary implementation language; synchronous outside `src/mcp/`; Phase 3 / US1 introduces single-threaded tokio in `src/mcp/` only |

## Frameworks

Phase 1–9 is a CLI application, not a web framework-based project. Phase 3 Foundational F8 introduces MCP server scaffolding scoped to `src/mcp/`, wired in Phase 3 / US1.

| Framework | Version | Purpose |
|-----------|---------|---------|
| clap | 4.x | CLI argument parsing and help generation; bare `tome plugin` (no subcommand) dispatches to interactive flow via `Option<PluginCommand>` derive pattern; `--version` intercepted by pre-parse hook in `main.rs` to honour `--json` and include embedder/reranker identities |

## Critical Dependencies

| Package | Version | Purpose | Usage Scope |
|---------|---------|---------|-------------|
| `serde` + `serde_derive` | 1.x | Configuration and manifest (de)serialisation | All TOML parsing for `config.toml` and `tome-catalog.toml`; Tome-owned structs use `#[serde(deny_unknown_fields)]` (FR-013a boundary); emit-only types in Phase 3 / US2 (`WorkspaceInfo`, `InitOutcome`) carry `Serialize` only |
| `toml` | 0.8 | TOML format support | Manifest and config file parsing; workspace init config generation |
| `thiserror` | 2.x | Typed error enums | Closed `TomeError` enum in `src/error.rs` (all fallible operations); 18+ enumerated failure variants with dedicated exit codes; Phase 3 adds codes 60–61 (MCP) and 70–75 (workspace/schema) |
| `anyhow` | 1.x | Error context chaining | Application-level error wrapping at boundaries |
| `tracing` + `tracing-subscriber` | 0.1, 0.3 | Structured logging to stderr | Diagnostic output orthogonal to `--json` stdout; Phase 3 F8 enables `json` feature for MCP log subscriber (JSON-lines to file via `src/mcp/log.rs`); custom `ContractEventFormat` in Phase 3 Polish renders contract-pinned field names (`ts`, `level`, `target`, `msg`) instead of defaults |
| `sha2` | 0.10 | Content-addressed cache naming and model integrity | URL hashing for `cache_dir_for()` in `src/paths.rs`; model download verification in `src/embedding/download.rs`; workspace registry deduplication |
| `regex` | 1.x | Credential scrubbing patterns | Git stderr sanitisation in `src/catalog/git.rs` (4 regex patterns); extended in Phase 3 to cover model download URLs and MCP log field scrubbing (principle XIII) |
| `ctrlc` | 3.x | Signal handling (SIGINT) | Global cancellation handler with exit code 8; SIGINT cancels in-flight git operations and model downloads; Phase 3 Polish: explicit SIGTERM handler for MCP server (Unix-only) with 5s graceful-shutdown timeout |
| `tempfile` | 3.x | Atomic file writes | Registry, per-catalog cache, models directory, manifest mutations, and workspace init staging dir (atomicity boundary: rename-based); `tempfile::Builder::tempdir_in` for same-filesystem POSIX-atomic rename in `src/workspace/init.rs` (Phase 3 / US2) |
| `hex` | 0.4 | Hex encoding for SHA256 digests | Cache directory naming alongside sha2; model checksum comparison in `src/embedding/download.rs` |
| `semver` | 1.x | Semantic version parsing | Catalog manifest version field validation |
| `time` | 0.3 | Timestamp formatting and parsing | Logging and manifest timestamps; RFC 3339 serialisation for `ModelManifest.installed_at` |
| `serde_json` | 1.x | JSON serialisation (NDJSON output) | `--json` mode formatting for stdout; `ModelManifest` serialisation to `manifest.json`; `WorkspaceInfo` and `InitOutcome` serialisation in Phase 3 / US2; `--json` byte-stability tests pin wire format |

### Phase 2 — foundational (no user-facing CLI wired until Phase 3)

| Package | Version | Purpose | Usage Scope |
|---------|---------|---------|-------------|
| `rusqlite` | 0.32 (`bundled`) | Embedded SQLite (statically linked, no system dep) | `src/index/*` — the local skill index database; WAL mode + advisory lockfile (FR-040); workspace-scoped index in Phase 3 Foundational F1 |
| `sqlite-vec` | vendored (v0.1.9) | KNN vector search extension for SQLite | `vendor/sqlite-vec/` compiled by `build.rs`; loaded via `sqlite3_auto_extension` in `src/index/vec_ext.rs`; symlink rejection hardening in Phase 3 Polish |
| `serde_yaml` | 0.9 | Lenient YAML frontmatter parsing for third-party `SKILL.md` | `src/plugin/frontmatter.rs` — parses upstream metadata leniently (FR-013a boundary; does not validate unknown fields) |
| `fastembed` | 4.x | ONNX-backed text embedding + reranking | `src/embedding/fastembed.rs` — loads BGE models from `${XDG_DATA_HOME}/tome/models/` at runtime; CPU execution provider only; Phase 3 Polish: eager-load at MCP startup via pre-flight pipeline (FR-110) |
| `reqwest` | 0.12 (`blocking`, `rustls-tls`, no defaults) | Synchronous HTTPS for model downloads | `src/embedding/download.rs` — downloads `MODEL_REGISTRY` entries with SHA-256 verification and atomicity; credential scrubbing on error chains |
| `indicatif` | 0.17 | Progress bars + spinners (TTY-aware) | `src/presentation/progress.rs` — download/reindex progress; refuses on non-TTY |
| `comfy-table` | 7.x | Table rendering for human-mode list/show output | `src/presentation/tables.rs` — `tome plugin list`, `tome models list`, query results, doctor reports |
| `owo-colors` | 4.x | Terminal colours with native `NO_COLOR` support | `src/presentation/colour.rs` — colourised output respecting `NO_COLOR` environment variable (principle I); Phase 3 Polish: all outputs respect `NO_COLOR` consistently |
| `inquire` | 0.7 (`crossterm`, no defaults) | Interactive Select/MultiSelect/Confirm prompts | `src/presentation/prompt.rs` — interactive plugin enable/disable/list/show; bare `tome plugin` browse flow; doctor repair confirmation; `--force` flag can skip confirmation; refuses on non-TTY (principle III) |
| `cc` (build-dep) | 1.x | C compiler driver for the vendored sqlite-vec amalgamation | `build.rs` only |

ONNX Runtime (`ort`) is a transitive dependency through `fastembed`; Tome does not link it directly. `src/embedding/runtime.rs` is a stub placeholder becoming load-bearing only if a direct dependency is added.

### Phase 3 — MCP server + workspaces + diagnostics (Phase 3 Foundational F7–F8 + Phase 3 / US1–US5)

Phase 3 introduces two new direct dependencies scoped to specific module boundaries:

| Package | Version | Purpose | Usage Scope | Notes |
|---------|---------|---------|-------------|-------|
| `rmcp` | 1.x (`transport-io`, `schemars` features) | MCP protocol and stdio server | `src/mcp/mod.rs`, `src/mcp/server.rs` — dispatches to server loop and tool handler registration; transport-io feature enables stdin/stdout channel per FR-221 | Phase 3 / US1; binary +1.10 MiB on macOS arm64 (22.04 MiB total, under 50 MB cap) |
| `tokio` | 1.x (`rt`, `macros`, `io-std`, `sync`, `signal`, `time` features) | Async runtime backing MCP server | `src/mcp/runtime.rs`, `src/mcp/mod.rs` — single-threaded `Builder::new_current_thread` only (no multi-threading for embedded model inference; research §R-2); structurally scoped via `tests/sync_boundary.rs` | Phase 3 / Foundational F8; no CLI async outside `src/mcp/` |
| `schemars` | 1.x | JSON Schema derivation for MCP tool I/O | `src/mcp/tools/{search_skills,get_skill}.rs` — `#[derive(JsonSchema)]` on input/output types per `contracts/mcp-tools.md`; contract-compliant schema generation for tool registration | Phase 3 / US1; also re-exported by rmcp's `schemars` feature |

**Feature enablement in existing dependencies:**
- `tracing-subscriber` gained `json` feature (line 22 in Cargo.toml) for MCP log subscriber support

**No new dependencies in Phase 3 / US2–US5**, US2 Polish PRs, or Phase 3 Polish PRs #51–#58. All functionality reuses Phase 1–3/US1 infrastructure.

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
| Binary Size | < 50 MB stripped on release builds (enforced by CI; revised from 10 MB ceiling in CONSTITUTION v1.2.0 after Phase 3 slice 1 measured 29.56 MB; `ort` CPU-only static linking is the load-bearing constraint; Phase 3 / US1 actual: 22.04 MiB on macOS arm64; Phase 3 Polish: maintains 22.04 MiB footprint; no growth from PRs #51–#58) |
| Output | Human-readable (default) or NDJSON (`--json`); logging to stderr only (orthogonal to `--json` stdout); colours respect `NO_COLOR` and auto-disable on non-TTY (Phase 3 Polish: consistent `NO_COLOR` coverage across all surfaces) |
| Model runtime | CPU-only ONNX Runtime (via `fastembed`); models downloaded at first use into `${XDG_DATA_HOME}/tome/models/`; fixed registry (compile-time constants) ensures bit-for-bit reproducibility |
| MCP server runtime | Single-threaded tokio with JSON-lines file logging to `${XDG_STATE_HOME}/tome/mcp.log` (10 MiB rotation cap); stdout reserved for MCP protocol only; stderr for fatal startup errors only (FR-222); Phase 3 Polish: custom `ContractEventFormat` emits contract-pinned field names (`ts`, `level`, `target`, `msg`) instead of defaults; explicit SIGTERM handler with 5s graceful-shutdown timeout (Unix-only) |
| Workspace storage | Atomic `.tome/` directories created via `tempfile::Builder::tempdir_in` (staging + POSIX rename); chmod 0700 on Unix before content lands (Phase 3 / US2); config persisted to `${WORKSPACE}/.tome/config.toml`; index DB at `${WORKSPACE}/.tome/index.db`; catalog clones in `${WORKSPACE}/.tome/catalogs/<sha>/` shared across scopes via reference-count tracking (Phase 3 / US3); Phase 3 Polish: symlink rejection hardening in `get_skill` skill walk |
| Doctor diagnostics | Subsystem health checks (models, index, workspace, drift) with optional repairs via `--fix`; harness detection at 6 known install locations (existence-only probe, no content reads); results in human-readable and `--json` output; Phase 3 Polish: orphan catalog cache detection, workspace registry status reporting, schema migration repair integration |
| Schema migrations | Forward-only migrations under advisory lock with pre-flight schema version gate; integration tests via synthetic-fixture injection in `tests/schema_migration_e2e.rs` (Phase 3 / US5) |

## Not Used (Explicitly Excluded)

- **Async runtime outside `src/mcp/`**: No `tokio`, `async-std`, or similar in Phase 1–9 main binary or any module outside `src/mcp/`. Structural test `tests/sync_boundary.rs` enforces the boundary.
- **Git library**: No `libgit2`, `git2`, or vendored Git. `std::process::Command` shells out to system `git` (constitution principle XII).
- **Direct ONNX Runtime dep**: `ort` is reached transitively through `fastembed` only; no direct linkage from Tome code.
- **Custom npm/cargo registry overrides**: All packages resolve from public registries.
- **Async database drivers** (e.g., `sqlx`): `rusqlite` is synchronous, suitable for a CLI with no concurrent connections (FR-040).
- **Serialization frameworks beyond serde**: No `protobuf`, `msgpack`, or other serialization deps; serde + serde_json cover all needs.

---

## What Does NOT Belong Here

- Directory structure → STRUCTURE.md
- System design patterns → ARCHITECTURE.md
- External service integrations → INTEGRATIONS.md
- Dev tools (linting, formatting) → CONVENTIONS.md
- Test frameworks → TESTING.md

---

*This document captures what executes in Tome v0.3.0 (Phase 3 complete). 399 tests pass across 53 suites; binary size 22.04 MiB on macOS arm64 (under 50 MB cap). Phase 3 Polish PRs #51–#58 closed with contract reconciliation, field-name pinning for MCP logs, security hardening (symlink rejection, registry validation, log 0600), and final documentation refresh.*

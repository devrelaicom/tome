# Technology Stack

> **Purpose**: Document what executes in this codebase - languages, runtimes, frameworks, and critical dependencies.
> **Generated**: 2026-05-25
> **Last Updated**: 2026-05-25 (Phase 4 Foundational F1–F11 + US1 complete; 490+ tests across 64+ suites; v0.3.0 + US1 additions)

## Languages & Runtimes

| Language | Version | Purpose |
|----------|---------|---------|
| Rust | stable (MSRV: 1.93) | Primary implementation language; synchronous outside `src/mcp/`; Phase 3 / US1 introduces single-threaded tokio in `src/mcp/` only |
| C++ | Vendored via `llama-cpp-2` | LLM inference runtime (Phase 4 summariser, Qwen2.5-0.5B-Instruct GGUF); sync API throughout |

## Frameworks

Phase 1–4 is a CLI application. Phase 3 Foundational F8 introduces MCP server scaffolding scoped to `src/mcp/`, wired in Phase 3 / US1. Phase 4 US1 adds harness integration orchestration.

| Framework | Version | Purpose |
|-----------|---------|---------|
| clap | 4.x | CLI argument parsing and help generation; bare `tome plugin` (no subcommand) dispatches to interactive flow via `Option<PluginCommand>` derive pattern; `--version` intercepted by pre-parse hook in `main.rs` to honour `--json` and include embedder/reranker/summariser identities |

## Critical Dependencies

| Package | Version | Purpose | Usage Scope |
|---------|---------|---------|-------------|
| `serde` + `serde_derive` | 1.x | Configuration and manifest (de)serialisation | All TOML parsing for `config.toml` and `tome-catalog.toml`; Tome-owned structs use `#[serde(deny_unknown_fields)]` (FR-013a boundary); Phase 4 adds multi-level settings composition (project/workspace/global) with strict parsing; emit-only types (`WorkspaceInfo`, `InitOutcome`, `BindOutcome`) carry `Serialize` only |
| `toml` | 0.8 | TOML format support | Tome-owned manifest and config file parsing; workspace init and settings file generation |
| `serde_json` | 1.x (with `preserve_order`) | JSON serialisation with preserved key order | `--json` mode formatting for stdout; `ModelManifest` serialisation; `BindOutcome` serialisation; `--json` byte-stability tests pin wire format; Phase 4 US1: preserves key order in harness MCP config output |
| `toml_edit` | 0.25 | Comment/order-preserving TOML editor | Phase 4 F1–F11 + US1: `src/settings/`, `src/harness/rules_file.rs`, `src/harness/mcp_config.rs` — read-modify-write harness MCP config files and workspace settings without losing comments/formatting |
| `thiserror` | 2.x | Typed error enums | Closed `TomeError` enum in `src/error.rs`; 28+ enumerated failure variants with dedicated exit codes (Phase 1 baseline + Phase 3 additions + Phase 4 adds codes 13–20 per FR-592 for harness/settings/summariser failures) |
| `anyhow` | 1.x | Error context chaining | Application-level error wrapping at boundaries |
| `tracing` + `tracing-subscriber` | 0.1, 0.3 | Structured logging to stderr | Diagnostic output orthogonal to `--json` stdout; Phase 3 F8 enables `json` feature for MCP log subscriber (JSON-lines to file via `src/mcp/log.rs`); custom `ContractEventFormat` renders contract-pinned field names; Phase 4 F1–F11 + US1: unchanged |
| `sha2` | 0.10 | Content-addressed cache naming and model integrity | URL hashing for `cache_dir_for()` in `src/paths.rs`; model download verification; workspace registry deduplication |
| `regex` | 1.x | Credential scrubbing patterns | Git stderr sanitisation in `src/catalog/git.rs`; model download URLs; MCP log field scrubbing; Phase 4 US1: harness rules-file marker line detection |
| `ctrlc` | 3.x | Signal handling (SIGINT) | Global cancellation handler with exit code 8; cancels in-flight git operations and model downloads; Phase 3 Polish: explicit SIGTERM handler for MCP server (Unix-only) with 5s graceful-shutdown timeout |
| `tempfile` | 3.x | Atomic file writes | Registry, per-catalog cache, models directory, manifest mutations, workspace init staging dir; `tempfile::Builder::tempdir_in` for same-filesystem POSIX-atomic rename in `src/workspace/init.rs`; Phase 4 F4: `src/util/atomic_dir.rs` promoted as reusable helper; Phase 4 US1: project `.tome/` marker landing atomic via same pattern |
| `hex` | 0.4 | Hex encoding for SHA256 digests | Cache directory naming alongside sha2; model checksum comparison |
| `semver` | 1.x | Semantic version parsing | Catalog manifest version field validation |
| `time` | 0.3 | Timestamp formatting and parsing | Logging and manifest timestamps; RFC 3339 serialisation; Phase 4 F1–F11 + US1: `CachedSummaries.generated_at` round-trip TOML datetime literals ↔ RFC 3339 strings |
| `rusqlite` | 0.32 (`bundled`) | Embedded SQLite (statically linked, no system dep) | `src/index/*` — the local skill index database; WAL mode + advisory lockfile (FR-040); workspace-scoped index in Phase 3 Foundational F1; Phase 4 F11: `workspace_catalogs` junction table (sole source of truth); Phase 4 US1: `workspace_projects` table for 1:1 binding (primary key on `project_path`) |
| `sqlite-vec` | vendored (v0.1.9) | KNN vector search extension for SQLite | `vendor/sqlite-vec/` compiled by `build.rs`; loaded via `sqlite3_auto_extension` in `src/index/vec_ext.rs` |
| `serde_yaml` | 0.9 | Lenient YAML frontmatter parsing for third-party `SKILL.md` | `src/plugin/frontmatter.rs` — parses upstream metadata leniently (FR-013a boundary); Phase 4 US1: `.tome/RULES.md` frontmatter parsed via same lenient parser |
| `fastembed` | 4.x | ONNX-backed text embedding + reranking | `src/embedding/fastembed.rs` — loads BGE models at runtime; CPU execution provider only; Phase 3 Polish: eager-load at MCP startup (FR-110) |
| `reqwest` | 0.12 (`blocking`, `rustls-tls`, no defaults) | Synchronous HTTPS for model downloads | `src/embedding/download.rs` — downloads `MODEL_REGISTRY` entries with SHA-256 verification and atomicity; credential scrubbing on error chains |
| `indicatif` | 0.17 | Progress bars + spinners (TTY-aware) | `src/presentation/progress.rs` — download/reindex progress; refuses on non-TTY; Phase 4 F6: byte-progress callback for streaming downloads |
| `comfy-table` | 7.x | Table rendering for human-mode list/show output | `src/presentation/tables.rs` — `tome plugin list`, `tome models list`, query results, doctor reports, `tome harness list` |
| `owo-colors` | 4.x | Terminal colours with native `NO_COLOR` support | `src/presentation/colour.rs` — colourised output respecting `NO_COLOR` environment variable |
| `inquire` | 0.7 (`crossterm`, no defaults) | Interactive Select/MultiSelect/Confirm prompts | `src/presentation/prompt.rs` — interactive plugin enable/disable/list/show; bare `tome plugin` browse flow; doctor repair confirmation; refusal to perform destructive ops on non-TTY; Phase 4 US1: binding confirmation prompts |
| `rmcp` | 1.x (`transport-io`, `schemars` features) | MCP protocol and stdio server | `src/mcp/mod.rs`, `src/mcp/server.rs` — tool router and handler registration; stdin/stdout channel per FR-221 |
| `tokio` | 1.x (`rt`, `macros`, `io-std`, `sync`, `signal`, `time` features) | Async runtime backing MCP server | `src/mcp/runtime.rs` — single-threaded `Builder::new_current_thread` only; scoped via `tests/sync_boundary.rs` |
| `schemars` | 1.x | JSON Schema derivation for MCP tool I/O | `src/mcp/tools/{search_skills,get_skill}.rs` — contract-compliant schema generation |
| `llama-cpp-2` | 0.1 | Rust bindings to `llama.cpp` | Phase 4 F1–F11 + US1: `src/summarise/` — LLM inference for workspace summaries (Qwen2.5-0.5B-Instruct GGUF); sync API, process-wide `LlamaBackend` singleton via `std::sync::OnceLock`; F6 ships skeleton (`StubSummariser`), US4.a will wire real summariser |

## Package Managers & Build Tools

| Tool | Version | Purpose |
|------|---------|---------|
| Cargo | (bundled with Rust) | Workspace management and builds |
| rustfmt | (pinned in rust-toolchain.toml) | Code formatting |
| clippy | (pinned in rust-toolchain.toml) | Linting with `-D warnings` (enforced in pre-commit and CI) |

## Runtime Environment

| Environment | Details |
|-------------|---------|
| **OS Targets** | Linux (ubuntu-latest) and macOS (macos-latest) — CI verified on both |
| **Deployment** | Single binary (`target/release/tome`); installed via `cargo install --path .` |
| **Binary Size** | < 50 MB stripped on release builds (enforced by CI); Phase 4 / F1–F11 projection: ~28.4 MiB on macOS arm64, ~34 MB on Linux x86_64; Phase 4 US1 adds harness module + binding surface but no new inference overhead (llama model not activated until US4) |
| **Output** | Human-readable (default) or NDJSON (`--json`); logging to stderr only; colours respect `NO_COLOR` and auto-disable on non-TTY |
| **Model runtime** | CPU-only ONNX Runtime (via `fastembed`); llama.cpp (via `llama-cpp-2`); models downloaded at first use into `${XDG_DATA_HOME}/tome/models/`; fixed registry ensures reproducibility |
| **MCP server runtime** | Single-threaded tokio with JSON-lines file logging to `${XDG_STATE_HOME}/tome/mcp.log` (10 MiB rotation cap); stdout reserved for MCP protocol only; stderr for fatal startup errors (FR-222); SIGTERM handler with 5s graceful-shutdown timeout (Unix-only) |
| **Workspace storage** | Atomic `.tome/` directories via `tempfile::Builder::tempdir_in` + POSIX rename; Phase 4 F4: `src/util/atomic_dir.rs` promoted as reusable helper; config at `${WORKSPACE}/.tome/config.toml`; index DB at `${WORKSPACE}/.tome/index.db`; catalog clones in `${WORKSPACE}/.tome/catalogs/<sha>/` with reference-count tracking; Phase 4 F11: enrolment moved to `workspace_catalogs` junction table (sole source of truth per FR-360) |
| **Project binding** | Phase 4 US1: atomic `.tome/` marker directory inside project root (e.g. `~/my-project/.tome/config.toml`); binding records the workspace name and project path in central DB under advisory lock; marker landing atomic via same `tempfile::Builder::tempdir_in` + rename pattern |
| **Configuration** | Global: `${XDG_CONFIG_HOME}/tome/settings.toml` (Phase 4 F8+); Workspace: `${WORKSPACE}/.tome/settings.toml`; Project: `${PROJECT}/.tome/config.toml` (Phase 4 US1); Phase 4 F1–F11 + US1: settings composition framework via `src/settings/` with multi-level resolver + layer precedence (project > workspace > global) |

## Strictness & Isolation Boundaries

| Boundary | Principle |
|----------|-----------|
| **Tome-owned inputs** | Strict parsing (`#[serde(deny_unknown_fields)]`): config, model manifests, index schema, multi-level settings (project/workspace/global) |
| **Third-party inputs** | Lenient parsing: plugin manifests, SKILL.md frontmatter, project `.tome/RULES.md` frontmatter — forward-compatible |
| **Async isolation** | All async code confined to `src/mcp/` module; structural test `tests/sync_boundary.rs` enforces boundary |
| **Sync enforcement** | Pre-commit hook runs `cargo test` with sync-boundary test; CI gates all PRs on boundary enforcement; Phase 4 US1: harness module stays sync-only |

## Feature Enablement

- `serde_json` gained `preserve_order` feature (Phase 4 F5) to maintain key ordering in all JSON output including harness config
- `toml_edit` enables comment/order preservation for harness MCP config and workspace settings read-modify-write (Phase 4 F1+)
- `tracing-subscriber` uses `json` feature for MCP log formatting
- Phase 4 US1: no new feature flags required; harness sync uses `StubHarness` for tests, production harness modules dispatch statically

---

## What Does NOT Belong Here

- Directory structure → STRUCTURE.md
- System design patterns → ARCHITECTURE.md
- External service integrations → INTEGRATIONS.md
- Dev tools (linting, formatting) → CONVENTIONS.md
- Test frameworks → TESTING.md

---

*This document captures what executes in Tome at Phase 4 Foundational F1–F11 + US1 complete. Phase 4 adds `llama-cpp-2`, `toml_edit`, and `serde_json/preserve_order`; harness module abstraction with five concrete impls (Claude Code, Codex, Cursor, Gemini, OpenCode); project binding infrastructure; multi-level settings composition framework. Binary size projection remains ~28–34 MB, well under the 50 MB cap.*

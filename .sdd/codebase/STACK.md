# Technology Stack

> **Purpose**: Document what executes in this codebase - languages, runtimes, frameworks, and critical dependencies.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-27 (Phase 5 / US2 shipped; substitution Stage 1 + Stage 2 with COMBINED_RE single-sweep design + data-dir lazy creation + workspace rename relocation)

## Languages & Runtimes

| Language | Version | Purpose |
|----------|---------|---------|
| Rust | stable (MSRV: 1.93) | Primary implementation language; synchronous outside `src/mcp/`; Phase 3 / US1 introduces single-threaded tokio in `src/mcp/` only |
| C++ | Vendored via `llama-cpp-2` | LLM inference runtime (Phase 4 summariser, Qwen2.5-0.5B-Instruct GGUF); sync API throughout |

## Frameworks

Phase 1–5 is a CLI application. Phase 3 Foundational F8 introduces MCP server scaffolding scoped to `src/mcp/`, wired in Phase 3 / US1. Phase 4 extends with project binding, workspace lifecycle, harness integration orchestration, workspace summarisation, and diagnostic subsystem categorization. Phase 5 / US1 extends MCP with `prompts` capability exposing user-invocable entries (skills + commands) as slash-prompts with variable substitution. Phase 5 / US2 wires substitution Stage 1 (built-ins) + Stage 2 (env passthrough) end-to-end via a unified COMBINED_RE regex sweep, with data-dir scaffolding and workspace rename relocation.

| Framework | Version | Purpose |
|-----------|---------|---------|
| clap | 4.x | CLI argument parsing and help generation; bare `tome plugin` (no subcommand) dispatches to interactive flow via `Option<PluginCommand>` derive pattern; `--version` intercepted by pre-parse hook in `main.rs` to honour `--json` and include embedder/reranker/summariser identities |

## Critical Dependencies

| Package | Version | Purpose | Usage Scope |
|---------|---------|---------|-------------|
| `serde` + `serde_derive` | 1.x | Configuration and manifest (de)serialisation | All TOML parsing for `config.toml`, `tome-catalog.toml`, workspace/project settings, and `.tome/RULES.md` frontmatter; Tome-owned structs use `#[serde(deny_unknown_fields)]` (FR-013a boundary); Phase 5: EntryKind enum serialised in schema migrations + prompt name collision tracking |
| `toml` | 0.8 | TOML format support | Tome-owned manifest and config file parsing; workspace init and settings file generation; Phase 5: unchanged (substitution context doesn't require new TOML support) |
| `serde_json` | 1.x (with `preserve_order`) | JSON serialisation with preserved key order | `--json` mode formatting for stdout; ModelManifest serialisation; BindOutcome serialisation; `--json` byte-stability tests pin wire format; Phase 5: preserves prompt list ordering |
| `toml_edit` | 0.25 | Comment/order-preserving TOML editor | Phase 4 US4: harness MCP config + workspace settings; Phase 5 US2: workspace rename relocation via surgical `[bound_workspace]` field update (no new toml_edit usage in substitution layer) |
| `thiserror` | 2.x | Typed error enums | Closed `TomeError` enum in `src/error.rs`; 30+ enumerated failure variants plus Phase 5 additions: `WorkspaceDataDirWriteFailed` (26), `PluginDataDirWriteFailed` (9), `PromptArgumentMismatch` (28), `EntryNotFound` (27), `SubstitutionFailed` (29), `InvalidArgumentFrontmatter` (25) per contracts/exit-codes-p5.md |
| `anyhow` | 1.x | Error context chaining | Application-level error wrapping at boundaries |
| `tracing` + `tracing-subscriber` | 0.1, 0.3 | Structured logging to stderr | Diagnostic output orthogonal to `--json` stdout; Phase 5 US2: includes substitution warnings (failed data-dir creation, argument count mismatches, workspace rename relocation errors), collision detection warnings |
| `sha2` | 0.10 | Content-addressed cache naming and model integrity | URL hashing for `cache_dir_for()` in `src/paths.rs`; model download verification; workspace registry deduplication; Phase 5: content-hash for prompt name collision tracking and substitution context caching |
| `regex` | 1.x | Credential scrubbing patterns and substitution | Git stderr sanitisation; model URLs; Phase 5 US1: substitution engine compiles regex patterns for built-ins and env via `src/substitution/regex_sets.rs` with `OnceLock`-cached compiled sets; Phase 5 US2: unified COMBINED_RE for single-sweep Stage 1 + Stage 2 (built-ins + env) to enforce no-rescan invariant (NFR-007) |
| `ctrlc` | 3.x | Signal handling (SIGINT) | Global cancellation handler with exit code 8; Phase 5: unchanged (substitution stays synchronous) |
| `tempfile` | 3.x | Atomic file writes | Registry, per-catalog cache, models directory, manifest mutations, workspace init staging dir; Phase 5 US2: unchanged (substitution context is in-memory; data-dir creation uses `std::fs::create_dir_all` non-atomically per design) |
| `hex` | 0.4 | Hex encoding for SHA256 digests | Cache directory naming alongside sha2; Phase 5: digest comparison in collision tracking |
| `semver` | 1.x | Semantic version parsing | Catalog manifest version field validation; Phase 5: unchanged |
| `time` | 0.3 | Timestamp formatting and parsing | Logging and manifest timestamps; RFC 3339 serialisation; Phase 5 US2: clock injection seam for `{{TOME_CLOCK_*}}` substitution (deterministic testing via `SUBSTITUTION_CLOCK_OVERRIDE` slot) |
| `rusqlite` | 0.32 (`bundled`) | Embedded SQLite (statically linked, no system dep) | `src/index/*` — the local skill index database; WAL mode + advisory lockfile (FR-040); Phase 5: schema v3 introduces `kind` discriminator column (Skill vs Command) and `entries` unified table replacing per-kind tables per contracts/schema-migration-p5.md; prompts/list + prompts/get run read-only queries |
| `sqlite-vec` | vendored (v0.1.9) | KNN vector search extension for SQLite | `vendor/sqlite-vec/` compiled by `build.rs`; loaded via `sqlite3_auto_extension` in `src/index/vec_ext.rs` |
| `serde_yaml` | 0.9 | Lenient YAML frontmatter parsing for third-party inputs | `src/plugin/frontmatter.rs` — parses upstream SKILL.md metadata leniently (FR-013a boundary); Phase 5 US2: parses `arguments` frontmatter field leniently (unknown subfields forward-compatible) on both skills and commands; frontmatter parser wired into substitution context builder |
| `fastembed` | 4.x | ONNX-backed text embedding + reranking | `src/embedding/fastembed.rs` — loads BGE models at runtime; CPU execution provider only; Phase 5: unchanged (inference runtimes orthogonal to prompts/substitution) |
| `reqwest` | 0.12 (`blocking`, `rustls-tls`, no defaults) | Synchronous HTTPS for model downloads | `src/embedding/download.rs` — downloads `MODEL_REGISTRY` entries; Phase 5: unchanged |
| `indicatif` | 0.17 | Progress bars + spinners (TTY-aware) | `src/presentation/progress.rs` — download/reindex progress; Phase 5: unchanged (prompts/substitution stay interactive-free) |
| `comfy-table` | 7.x | Table rendering for human-mode list/show output | `src/presentation/tables.rs`; Phase 5: unchanged |
| `owo-colors` | 4.x | Terminal colours with native `NO_COLOR` support | `src/presentation/colour.rs`; Phase 5: unchanged |
| `inquire` | 0.7 (`crossterm`, no defaults) | Interactive Select/MultiSelect/Confirm prompts | `src/presentation/prompt.rs` — interactive plugin enable/disable/list/show; bare `tome plugin` browse flow; Phase 5: unchanged (prompt execution stays command-line, not interactive) |
| `rmcp` | 1.x (`transport-io`, `schemars` features) | MCP protocol and stdio server | `src/mcp/mod.rs`, `src/mcp/server.rs` — tool router and handler registration; Phase 5 / US1: extends with `PromptRouter` + `prompts/list` + `prompts/get` handlers per contracts/mcp-prompts.md; Phase 5 / US2: prompts/get invokes substitution render with Stages 1–2 (built-ins + env) |
| `tokio` | 1.x (`rt`, `macros`, `io-std`, `sync`, `signal`, `time` features) | Async runtime backing MCP server | `src/mcp/runtime.rs` — single-threaded `Builder::new_current_thread` only; scoped via `tests/sync_boundary.rs`; Phase 5: unchanged |
| `schemars` | 1.x | JSON Schema derivation for MCP tool I/O | `src/mcp/tools/` — contract-compliant schema generation; Phase 5: unchanged (prompt I/O uses rmcp wire shapes directly) |
| `llama-cpp-2` | =0.1.146 (exact-pinned) | Rust bindings to `llama.cpp` | Phase 4 F6–US4: `src/summarise/` — LLM inference for workspace summaries; Phase 5: unchanged |
| `encoding_rs` | 0.8 | Character encoding for LLM tokenization | Phase 4 US4: `src/summarise/llama.rs` — required by `LlamaModel::token_to_piece`; Phase 5: unchanged |
| `filetime` | 0.2 (dev-only) | File modification time manipulation | Phase 4 US5.a: tests for orphan cleanup; Phase 5: unchanged |

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
| **Binary Size** | < 50 MB stripped on release builds (enforced by CI); Phase 5 projection: **~27 MiB on macOS arm64** (Phase 4 Polish baseline 26.32 MiB + Phase 5 substitution engine overhead < 1 MiB); zero new top-level dependencies; `regex` promoted from transitive to direct (no net size change — already in dep tree via reqwest + llama-cpp-2) |
| **Output** | Human-readable (default) or NDJSON (`--json`); logging to stderr only; colours respect `NO_COLOR` and auto-disable on non-TTY; Phase 5: prompt list JSON includes description truncation per DESCRIPTION_MAX_CHARS (300 chars per FR-066) |
| **Model runtime** | CPU-only ONNX Runtime (via `fastembed`, embedder + reranker); `llama.cpp` via `llama-cpp-2` (summariser, Qwen2.5-0.5B-Instruct GGUF); all three downloaded at first use into `${XDG_DATA_HOME}/tome/models/`; Phase 5: unchanged |
| **MCP server runtime** | Single-threaded tokio with JSON-lines file logging; Phase 5 / US1: extended with `prompts/list` and `prompts/get` handlers; prompt router built dynamically from enabled-and-user-invocable entries at startup; `listChanged: false` per rmcp contract (prompts change only on plugin enable/disable/reindex, not at runtime); Phase 5 / US2: prompts/get invokes substitution render with Stages 1–2 (built-ins + env) via unified COMBINED_RE sweep |
| **Workspace storage** | Atomic `.tome/` directories via `tempfile::Builder::tempdir_in` + POSIX rename; Phase 5 US2: workspace rename relocation of bound projects via `toml_edit` surgical updates (one `[bound_workspace]` field rewrite per project marker) |
| **Project binding** | Phase 4 US1: atomic `.tome/` marker directory inside project root; Phase 5 US2: bound_workspace name relocation on `tome workspace rename` via surgical TOML edits |
| **Configuration** | Global: `${XDG_CONFIG_HOME}/tome/settings.toml`; Workspace: `${WORKSPACE}/.tome/settings.toml` with `[summaries]` table; Project: `${PROJECT}/.tome/config.toml` (binding marker); Phase 5: no new config layers (substitution parameters passed in-process via `SubstitutionContext` struct) |
| **Harness configuration** | Per-harness files (e.g., `~/.claude/.mcp.json`, `~/.codex/.mcp.toml`); Phase 5: unchanged (harness MCP config independent of prompts/substitution) |
| **Schema migrations** | v2 (Phase 4 final) → v3 (Phase 5 F2): introduces `kind` discriminator column + unified `entries` table replacing per-kind schema; backfill defaults per contracts/schema-migration-p5.md; forward-only migration under advisory lock |
| **Substitution parameters** | Phase 5 / US1–US3: four-stage pipeline (built-ins → env → arguments → ARGUMENTS tail); Phase 5 / US2: Stage 1 (built-ins) + Stage 2 (env) wired via unified COMBINED_RE single-sweep design per `src/substitution/regex_sets.rs` (enforces NFR-007 no-rescan invariant; closes exfiltration vector); Stage 3 + 4 land in US3 |
| **Data-dir lazy creation** | Phase 5 / US2: plugin + workspace data dirs created on first `{{TOME_PLUGIN_DATA}}` / `{{TOME_WORKSPACE_DATA}}` variable reference during prompt execution via `src/substitution/data_dir.rs` (non-atomic `std::fs::create_dir_all`, recoverable via re-run); failure → `WorkspaceDataDirWriteFailed` (26) or `PluginDataDirWriteFailed` (9) |
| **Clock injection** | Phase 5 / US2: `{{TOME_CLOCK_*}}` built-ins hook into wall-clock via `src/substitution::current_clock()`, which honours `SUBSTITUTION_CLOCK_OVERRIDE` slot for deterministic testing (via `ClockOverrideGuard` RAII helper in `tests/common/mod.rs`) |

## Strictness & Isolation Boundaries

| Boundary | Principle |
|----------|-----------|
| **Tome-owned inputs** | Strict parsing (`#[serde(deny_unknown_fields)]`): config, model manifests, index schema, multi-level settings, cached summaries; Phase 5: includes `kind` column enum (Skill/Command) in unified entries table, collision records, substitution argument frontmatter |
| **Third-party inputs** | Lenient parsing: plugin manifests, SKILL.md frontmatter, command.json frontmatter, project `.tome/RULES.md` frontmatter — forward-compatible; Phase 5: `arguments` frontmatter field parsed leniently on both skills and commands |
| **Async isolation** | All async code confined to `src/mcp/`; structural test `tests/sync_boundary.rs` enforces boundary; Phase 5: substitution layer stays sync-only (all four pipeline stages are sync, no async-await) |
| **Sync enforcement** | Pre-commit hook runs `cargo test` with sync-boundary test; Phase 5: unchanged |
| **Substitution no-rescan invariant** | Phase 5 / US2: unified COMBINED_RE ensures Stages 1 + 2 are scanned in a single pass; resolved values never re-enter the scanner (closes exfiltration vector per NFR-007 / FR-051) |

## Feature Enablement

- `serde_json` gained `preserve_order` feature (Phase 4 F5) to maintain key ordering in JSON output; Phase 5: preserves prompt list ordering
- `toml_edit` enables comment/order preservation for harness MCP config, workspace settings (Phase 4 F1+); Phase 5 US2: used for workspace rename relocation (surgical `[bound_workspace]` field update)
- `tracing-subscriber` uses `json` feature for MCP log formatting (Phase 3 Polish); Phase 5: unchanged
- Phase 5: no new feature flags required; substitution uses stable `regex` without optional features; Phase 5 F2 promotes `regex` from transitive to direct (no net feature change)

---

## What Does NOT Belong Here

- Directory structure → STRUCTURE.md
- System design patterns → ARCHITECTURE.md
- External service integrations → INTEGRATIONS.md
- Dev tools (linting, formatting) → CONVENTIONS.md
- Test frameworks → TESTING.md

---

*This document captures what executes in Tome at Phase 5 / US2 (substitution Stage 1 + Stage 2 with COMBINED_RE single-sweep design shipped). Phase 5 US2 adds: unified regex sweep for built-ins + env via COMBINED_RE per `src/substitution/regex_sets.rs` (enforces no-rescan invariant / closes exfiltration vector); data-dir lazy creation on first `{{TOME_*_DATA}}` reference; workspace rename relocation via toml_edit surgical updates; clock injection seam + test override; 5 exit codes (25–29) wired for data-dir creation failures, argument mismatches, missing entries, substitution failures, invalid frontmatter. Zero new top-level dependencies. Binary size: **~27 MiB on macOS arm64**, well under the 50 MB cap. US1 ships prompts capability (MCP exposure + prompt naming + collision tracking). US2 ships substitution Stage 1 (built-ins) + Stage 2 (env passthrough) with no-rescan enforcement. US3 will wire argument substitution stages. Phase 5 / US4 ships CLI slash-commands as first-class entries (EntryKind discriminator).*

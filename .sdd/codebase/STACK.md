# Technology Stack

> **Purpose**: Document what executes in this codebase - languages, runtimes, frameworks, and critical dependencies.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-29 (Phase 6 Polish / v0.6.0 cut; version bump only, zero new dependencies; binary size stable)

## Languages & Runtimes

| Language | Version | Purpose |
|----------|---------|---------|
| Rust | stable (MSRV: 1.93) | Primary implementation language; synchronous outside `src/mcp/`; Phase 3 / US1 introduces single-threaded tokio in `src/mcp/` only |
| C++ | Vendored via `llama-cpp-2` | LLM inference runtime (Phase 4 summariser, Qwen2.5-0.5B-Instruct GGUF); sync API throughout |

## Frameworks

Phase 1–5 is a CLI application. Phase 3 Foundational F8 introduces MCP server scaffolding scoped to `src/mcp/`, wired in Phase 3 / US1. Phase 4 extends with project binding, workspace lifecycle, harness integration orchestration, workspace summarisation, and diagnostic subsystem categorization. Phase 5 / US1 extends MCP with `prompts` capability exposing user-invocable entries (skills + commands) as slash-prompts with variable substitution. Phase 5 / US2–US3 complete the 4-stage substitution pipeline (built-ins, env, arguments, ARGUMENTS-footer) via unified COMBINED_RE single-sweep regex. Phase 5 / US4 ships middle-tier discovery tool `get_skill_info` + extends `search_skills`. Phase 5 / US5 completes: per-entry invocability matrix end-to-end, `plugin show` Phase 5 surfaces, doctor extensions (PromptsReport, OrphanDataDirReport, EntryCountsByKind, pending_re_embedding heuristic). Phase 6 / US1–US5 shipped (hooks + agents). Phase 6 Polish v0.6.0: version bump, zero new dependencies.

| Framework | Version | Purpose |
|-----------|---------|---------|
| clap | 4.x | CLI argument parsing and help generation; bare `tome plugin` (no subcommand) dispatches to interactive flow via `Option<PluginCommand>` derive pattern; `--version` intercepted by pre-parse hook in `main.rs` to honour `--json` and include embedder/reranker/summariser identities |

## Critical Dependencies

| Package | Version | Purpose | Usage Scope |
|---------|---------|---------|-------------|
| `serde` + `serde_derive` | 1.x | Configuration and manifest (de)serialisation | All TOML parsing for `config.toml`, `tome-catalog.toml`, workspace/project settings, and `.tome/RULES.md` frontmatter; Tome-owned structs use `#[serde(deny_unknown_fields)]` (FR-013a boundary); Phase 5: EntryKind enum serialised in schema migrations + prompt name collision tracking + doctor reports |
| `toml` | 0.8 | TOML format support | Tome-owned manifest and config file parsing; workspace init and settings file generation; Phase 5: unchanged (substitution context doesn't require new TOML support) |
| `serde_json` | 1.x (with `preserve_order`) | JSON serialisation with preserved key order | `--json` mode formatting for stdout; ModelManifest serialisation; BindOutcome serialisation; `--json` byte-stability tests pin wire format; Phase 5: doctor reports (PromptsReport, OrphanDataDirReport, EntryCountsByKind) + plugin show output |
| `toml_edit` | 0.25 | Comment/order-preserving TOML editor | Phase 4 US4: harness MCP config + workspace settings; Phase 5 US2: workspace rename relocation via surgical `[bound_workspace]` field update (no new toml_edit usage in US3–US5) |
| `thiserror` | 2.x | Typed error enums | Closed `TomeError` enum in `src/error.rs`; 30+ enumerated failure variants plus Phase 5 additions: `WorkspaceDataDirWriteFailed` (26), `PluginDataDirWriteFailed` (9), `PromptArgumentMismatch` (28), `EntryNotFound` (27), `SubstitutionFailed` (29), `InvalidArgumentFrontmatter` (25) per contracts/exit-codes-p5.md; Phase 5 US5: no new variants (all pre-allocated in F1) |
| `anyhow` | 1.x | Error context chaining | Application-level error wrapping at boundaries |
| `tracing` + `tracing-subscriber` | 0.1, 0.3 | Structured logging to stderr | Diagnostic output orthogonal to `--json` stdout; Phase 5 US2: substitution warnings; Phase 5 US5: doctor report warnings (orphaned data-dirs, pending re-embeddings), plugin show entry annotation warnings |
| `sha2` | 0.10 | Content-addressed cache naming and model integrity | URL hashing for `cache_dir_for()` in `src/paths.rs`; model download verification; workspace registry deduplication; Phase 5: content-hash for prompt name collision tracking |
| `regex` | 1.x | Credential scrubbing patterns and substitution | Git stderr sanitisation; model URLs; Phase 5 US1: substitution engine compiles regex patterns for built-ins and env via `src/substitution/regex_sets.rs` with `OnceLock`-cached compiled sets; Phase 5 US2–US3: unified COMBINED_RE for single-sweep all-stage substitution (enforces NFR-007 no-rescan by structural single-pass design) |
| `ctrlc` | 3.x | Signal handling (SIGINT) | Global cancellation handler with exit code 8; Phase 5: unchanged (substitution stays synchronous) |
| `tempfile` | 3.x | Atomic file writes | Registry, per-catalog cache, models directory, manifest mutations, workspace init staging dir; Phase 5 US2–US5: unchanged (substitution context is in-memory; data-dir creation uses `std::fs::create_dir_all` non-atomically per design) |
| `hex` | 0.4 | Hex encoding for SHA256 digests | Cache directory naming alongside sha2; Phase 5: digest comparison in collision tracking |
| `semver` | 1.x | Semantic version parsing | Catalog manifest version field validation; Phase 5: unchanged |
| `time` | 0.3 | Timestamp formatting and parsing | Logging and manifest timestamps; RFC 3339 serialisation; Phase 5 US2–US3: clock injection seam for `{{TOME_CLOCK_*}}` substitution (deterministic testing via `SUBSTITUTION_CLOCK_OVERRIDE` slot) |
| `rusqlite` | 0.32 (`bundled`) | Embedded SQLite (statically linked, no system dep) | `src/index/*` — the local skill index database; WAL mode + advisory lockfile (FR-040); Phase 5: schema v3 introduces `kind` discriminator column (Skill vs Command) and unified `entries` table replacing per-kind tables per contracts/schema-migration-p5.md; `searchable` filter enforced in queries; when_to_use field indexed; Phase 5 US5: doctor queries for entry counts by kind, pending re-embeddings detection |
| `sqlite-vec` | vendored (v0.1.9) | KNN vector search extension for SQLite | `vendor/sqlite-vec/` compiled by `build.rs`; loaded via `sqlite3_auto_extension` in `src/index/vec_ext.rs`; Phase 5 US4–US5: unchanged (KNN applies to embedding_text field which includes when_to_use) |
| `serde_yaml` | 0.9 | Lenient YAML frontmatter parsing for third-party inputs | `src/plugin/frontmatter.rs` — parses upstream SKILL.md metadata leniently (FR-013a boundary); Phase 5 US2–US5: parses `arguments`, `user_invocable`, `when_to_use` frontmatter fields leniently on both skills and commands (unknown subfields forward-compatible) |
| `fastembed` | 4.x | ONNX-backed text embedding + reranking | `src/embedding/fastembed.rs` — loads BGE models at runtime; CPU execution provider only; Phase 5 US5: unchanged (inference runtimes orthogonal to invocability + doctor features) |
| `reqwest` | 0.12 (`blocking`, `rustls-tls`, no defaults) | Synchronous HTTPS for model downloads | `src/embedding/download.rs` — downloads `MODEL_REGISTRY` entries; Phase 5: unchanged |
| `indicatif` | 0.17 | Progress bars + spinners (TTY-aware) | `src/presentation/progress.rs` — download/reindex progress; Phase 5: unchanged (prompts/substitution/doctor stay interactive-free for progress) |
| `comfy-table` | 7.x | Table rendering for human-mode list/show output | `src/presentation/tables.rs`; Phase 5 US5: `plugin show` renders Skills + Commands sections with entry annotations |
| `owo-colors` | 4.x | Terminal colours with native `NO_COLOR` support | `src/presentation/colour.rs`; Phase 5: unchanged |
| `inquire` | 0.7 (`crossterm`, no defaults) | Interactive Select/MultiSelect/Confirm prompts | `src/presentation/prompt.rs` — interactive plugin enable/disable/list/show; bare `tome plugin` browse flow; Phase 5: unchanged (prompt execution stays command-line, not interactive) |
| `rmcp` | 1.x (`transport-io`, `schemars` features) | MCP protocol and stdio server | `src/mcp/mod.rs`, `src/mcp/server.rs` — tool router and handler registration; Phase 5 / US1: extends with `PromptRouter` + `prompts/list` + `prompts/get` handlers; Phase 5 / US4: adds third tool `get_skill_info` with resource enumeration; Phase 5 / US5: unchanged (3-tool stable) |
| `tokio` | 1.x (`rt`, `macros`, `io-std`, `sync`, `signal`, `time` features) | Async runtime backing MCP server | `src/mcp/runtime.rs` — single-threaded `Builder::new_current_thread` only; scoped via `tests/sync_boundary.rs`; Phase 5: unchanged |
| `schemars` | 1.x | JSON Schema derivation for MCP tool I/O | `src/mcp/tools/` — contract-compliant schema generation; Phase 5 / US4: extends to `get_skill_info` Input + Output types; Phase 5: unchanged |
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
| **Binary Size** | < 50 MB stripped on release builds (enforced by CI); Phase 6 Polish projection: **~27 MiB on macOS arm64** (Phase 5 baseline 27 MiB + Phase 6 hooks/agents layers negligible); zero new top-level dependencies; `regex` promoted from transitive to direct (no net size change) |
| **Output** | Human-readable (default) or NDJSON (`--json`); logging to stderr only; colours respect `NO_COLOR` and auto-disable on non-TTY; Phase 5 US5: doctor reports include entry counts by kind (Skill vs Command from schema v3) + orphaned data-dir list + pending re-embeddings warning + prompt list (via PromptsReport) |
| **Model runtime** | CPU-only ONNX Runtime (via `fastembed`, embedder + reranker); `llama.cpp` via `llama-cpp-2` (summariser, Qwen2.5-0.5B-Instruct GGUF); all three downloaded at first use into `${XDG_DATA_HOME}/tome/models/`; Phase 5: unchanged |
| **MCP server runtime** | Single-threaded tokio with JSON-lines file logging; Phase 5 / US1–US4: 3-tool stable surface (search_skills, get_skill, get_skill_info); Phase 5 / US5: unchanged (doctor extensions independent of MCP runtime) |
| **Workspace storage** | Atomic `.tome/` directories via `tempfile::Builder::tempdir_in` + POSIX rename; Phase 5 US2: workspace rename relocation of bound projects via `toml_edit` surgical updates; Phase 5 US5: unchanged |
| **Project binding** | Phase 4 US1: atomic `.tome/` marker directory inside project root; Phase 5 US2–US5: bound_workspace name relocation on `tome workspace rename` via surgical TOML edits |
| **Configuration** | Global: `${XDG_CONFIG_HOME}/tome/settings.toml`; Workspace: `${WORKSPACE}/.tome/settings.toml` with `[summaries]` table; Project: `${PROJECT}/.tome/config.toml` (binding marker); Phase 5: no new config layers (substitution context in-process, doctor extension settings via command flags); Phase 6: no new config layers |
| **Harness configuration** | Per-harness files (e.g., `~/.claude/.mcp.json`, `~/.codex/.mcp.toml`); Phase 5: unchanged (harness MCP config independent of prompts/discovery/doctor) |
| **Schema migrations** | v2 (Phase 4 final) → v3 (Phase 5 F2): introduces `kind` discriminator column + unified `entries` table replacing per-kind schema + `searchable` filter column + `when_to_use` text field (indexed); backfill defaults per contracts/schema-migration-p5.md; forward-only migration under advisory lock; Phase 5 US5: unchanged (schema stable); Phase 6: no schema changes |
| **Substitution parameters** | Phase 5 / US1–US3: four-stage pipeline (built-ins → env → arguments → ARGUMENTS tail) via unified COMBINED_RE single-sweep design per `src/substitution/regex_sets.rs`; all 4 stages scanned once (enforces NFR-007 no-rescan invariant); Phase 5 US4–US5: unchanged (discovery + doctor independent of substitution); Phase 6: unchanged |
| **Data-dir lazy creation** | Phase 5 / US2–US5: plugin + workspace data dirs created on first `{{TOME_PLUGIN_DATA}}` / `{{TOME_WORKSPACE_DATA}}` variable reference during prompt execution via `src/substitution/data_dir.rs` (non-atomic `std::fs::create_dir_all`, recoverable via re-run); Phase 5 US5: doctor reports orphaned data-dirs via `OrphanDataDirReport`; Phase 6: unchanged |
| **Clock injection** | Phase 5 / US2–US5: `{{TOME_CLOCK_*}}` built-ins hook into wall-clock via `src/substitution::current_clock()`, which honours `SUBSTITUTION_CLOCK_OVERRIDE` slot for deterministic testing; Phase 6: unchanged |
| **Discovery tools** (Phase 5 / US4–US5) | MCP `search_skills` — KNN query + reranking over embedding space (includes when_to_use text); filters to `searchable = 1` entries; truncates descriptions per caller-supplied `description_max_chars` (default 150); returns `kind` field discriminating skills from commands. New MCP `get_skill_info` — middle-tier query returning full description + when_to_use + resource enumeration (parent directory file listing + subdirectories) capped per `PER_DIRECTORY_CAP` (5). Phase 5 US5: unchanged (discovery tools independent of invocability + doctor features); Phase 6: unchanged |
| **Entry invocability** (Phase 5 / US5) | Per-entry `user_invocable` boolean flag (defaults to true for skills, false for commands per data-model); workspace settings can override per workspace; Phase 5 US1 prompt router filters by `user_invocable: true`; Phase 5 US5: exposed in `plugin show` + invocability matrix test confirms end-to-end wiring; Phase 6: unchanged |
| **Doctor extensions** (Phase 5 / US5) | New report types: `PromptsReport` (reuses prompt registry build), `OrphanDataDirReport` (walks data-dir tree), `EntryCountsByKind` (wraps two SELECTs in unchecked_transaction for snapshot consistency); pending_re_embedding heuristic detects stale content_hash vs actual file hash via `index::skills::pending_re_embeddings_for_workspace` helper; Phase 6: unchanged |
| **Helper consolidation** (Phase 5 Polish) | New `src/mcp/substitution_helpers.rs` module exposes `build_context_for_entry` shared by both `prompts/get` and `get_skill` handlers (eliminates near-duplicate context builders; single SSOT per `src/mcp/substitution_helpers.rs`); new `pub(crate) fn validate_db_stored_path` in `src/index/skills.rs` centralises S-H1 boundary check (path traversal defence; consumed by both `resolve_entry_body_path` and `commands/plugin/show.rs::list_entries`); `truncate_description` pattern propagated to prompts module matching discovery tool implementation (bounded `char_indices` walk per US4.d HIGH fix); Phase 6: unchanged |

## Strictness & Isolation Boundaries

| Boundary | Principle |
|----------|-----------|
| **Tome-owned inputs** | Strict parsing (`#[serde(deny_unknown_fields)]`): config, model manifests, index schema, multi-level settings, cached summaries; Phase 5: includes `kind` column enum (Skill/Command) in unified entries table, collision records, substitution argument frontmatter, doctor report types; Phase 6: unchanged |
| **Third-party inputs** | Lenient parsing: plugin manifests, SKILL.md frontmatter, command.json frontmatter, project `.tome/RULES.md` frontmatter — forward-compatible; Phase 5 US2–US5: `arguments`, `user_invocable`, `when_to_use` frontmatter fields parsed leniently (unknown sub-fields forward-compatible); Phase 6: unchanged |
| **Async isolation** | All async code confined to `src/mcp/`; structural test `tests/sync_boundary.rs` enforces boundary; Phase 5: substitution + doctor layers stay sync-only; Phase 6: unchanged |
| **Sync enforcement** | Pre-commit hook runs `cargo test` with sync-boundary test; Phase 5: unchanged; Phase 6: unchanged |
| **Substitution no-rescan invariant** | Phase 5 / US2–US3: unified COMBINED_RE ensures all 4 stages scanned once; resolved values never re-enter the scanner (closes exfiltration vector per NFR-007 / FR-051); structural enforcement via single-sweep regex design; Phase 5 US4–US5: unchanged; Phase 6: unchanged |
| **Path traversal defence** | Phase 5 Polish: `validate_db_stored_path(stored_path)` in `src/index/skills.rs` checks for `..` components and absolute paths; consumed by both `resolve_entry_body_path` (MCP tool S-H1 boundary) and `commands/plugin/show.rs::list_entries` (CLI surface); single SSOT prevents duplicated validation logic; Phase 6: unchanged |

## Feature Enablement

- `serde_json` gained `preserve_order` feature (Phase 4 F5) to maintain key ordering in JSON output; Phase 5: doctor reports + plugin show output preserve ordering; Phase 6: unchanged
- `toml_edit` enables comment/order preservation for harness MCP config, workspace settings (Phase 4 F1+); Phase 5 US2–US5: used for workspace rename relocation; Phase 6: unchanged
- `tracing-subscriber` uses `json` feature for MCP log formatting (Phase 3 Polish); Phase 5: unchanged; Phase 6: unchanged
- Phase 5: no new feature flags required; substitution uses stable `regex` without optional features; Phase 5 F2 promotes `regex` from transitive to direct; Phase 6: unchanged

---

## What Does NOT Belong Here

- Directory structure → STRUCTURE.md
- System design patterns → ARCHITECTURE.md
- External service integrations → INTEGRATIONS.md
- Dev tools (linting, formatting) → CONVENTIONS.md
- Test frameworks → TESTING.md

---

*This document captures what executes in Tome at Phase 6 Polish / v0.6.0 (version bump only, zero new dependencies, binary size stable at ~27 MiB on macOS arm64). Phase 5 complete features: per-entry invocability matrix wired end-to-end (user_invocable flag defaults: skills=true, commands=false per data-model; prompt router filters by flag; invocability matrix test covers both skill + command paths); `plugin show` extended with Skills + Commands sections, per-entry annotations + [dormant] markers, entry count format `<n> skills, <m> commands` (kind-aware); doctor extended with three new report types (PromptsReport reusing prompt registry build logic, OrphanDataDirReport walking filesystem tree capped at 10K entries for safety, EntryCountsByKind wrapping two SELECTs in unchecked_transaction for snapshot consistency); pending_re_embedding heuristic detects when stored skill content_hash doesn't match current body hash (via index::skills::pending_re_embeddings_for_workspace helper). Phase 5 Polish ships v0.5.0 (version bump) + new helper consolidation: `build_context_for_entry` in `src/mcp/substitution_helpers.rs` (shared by prompts/get + get_skill); `validate_db_stored_path` SSOT in `src/index/skills.rs` (path traversal defence); description truncation pattern propagated across prompts + discovery tools (bounded walk matching US4.d HIGH fix). Phase 6 Polish ships v0.6.0 (version bump only). Zero new top-level dependencies across entire Phase 6. Binary size: **~27 MiB on macOS arm64**, well under the 50 MB cap. No production code changes in Polish phase. Phase 6 / US1–US5 shipped all feature work (hooks + agents). Phase 6 Polish Polish (P8) = v0.6.0 cut + version sync across manifests.*

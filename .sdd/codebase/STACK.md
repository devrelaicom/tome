# Technology Stack

> **Purpose**: Document what executes in this codebase - languages, runtimes, frameworks, and critical dependencies.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-26 (Phase 4 US5.a complete; 894+ tests across 122+ suites; v0.4.0 release)

## Languages & Runtimes

| Language | Version | Purpose |
|----------|---------|---------|
| Rust | stable (MSRV: 1.93) | Primary implementation language; synchronous outside `src/mcp/`; Phase 3 / US1 introduces single-threaded tokio in `src/mcp/` only |
| C++ | Vendored via `llama-cpp-2` | LLM inference runtime (Phase 4 summariser, Qwen2.5-0.5B-Instruct GGUF); sync API throughout |

## Frameworks

Phase 1–4 is a CLI application. Phase 3 Foundational F8 introduces MCP server scaffolding scoped to `src/mcp/`, wired in Phase 3 / US1. Phase 4 extends with project binding, workspace lifecycle, harness integration orchestration, workspace summarisation, and diagnostic subsystem categorization.

| Framework | Version | Purpose |
|-----------|---------|---------|
| clap | 4.x | CLI argument parsing and help generation; bare `tome plugin` (no subcommand) dispatches to interactive flow via `Option<PluginCommand>` derive pattern; `--version` intercepted by pre-parse hook in `main.rs` to honour `--json` and include embedder/reranker/summariser identities |

## Critical Dependencies

| Package | Version | Purpose | Usage Scope |
|---------|---------|---------|-------------|
| `serde` + `serde_derive` | 1.x | Configuration and manifest (de)serialisation | All TOML parsing for `config.toml`, `tome-catalog.toml`, workspace/project settings, and `.tome/RULES.md` frontmatter; Tome-owned structs use `#[serde(deny_unknown_fields)]` (FR-013a boundary); Phase 4 US4: project `.tome/RULES.md` frontmatter + `CachedSummaries` parsed via serde; US5.a: `Subsystem` enum custom Serialize/Deserialize preserving wire format |
| `toml` | 0.8 | TOML format support | Tome-owned manifest and config file parsing; workspace init and settings file generation; Phase 4 US4: workspace settings composition with `[summaries]` table caching short/long summaries + generated_at timestamp |
| `serde_json` | 1.x (with `preserve_order`) | JSON serialisation with preserved key order | `--json` mode formatting for stdout; `ModelManifest` serialisation; `BindOutcome` serialisation; `--json` byte-stability tests pin wire format; Phase 4 US4: preserves key order in harness MCP config output and workspace summary outcome serialisation; US5.a: DoctorReport includes Subsystem-typed enum serialised with byte-identical wire shape to Phase 3 |
| `toml_edit` | 0.25 | Comment/order-preserving TOML editor | Phase 4 US4: `src/settings/`, `src/harness/rules_file.rs`, `src/harness/mcp_config.rs`, `src/workspace/regen_summary.rs` — read-modify-write harness MCP config files, workspace settings files, and project `.tome/RULES.md` without losing comments/formatting; US4 workspace regenerate-summary updates `[summaries]` table in workspace settings |
| `thiserror` | 2.x | Typed error enums | Closed `TomeError` enum in `src/error.rs`; 30+ enumerated failure variants with dedicated exit codes (Phase 1 baseline + Phase 3 additions + Phase 4 adds codes 13–20, 24 per FR-592 for harness/settings/summariser failures); Phase 4 US4: adds `SummariserFailure` (24) with `SummariserFailureKind` enum for OutputEmpty / BackendInitFailed / InferenceFailure / ModelNotFound / ModelCorrupt |
| `anyhow` | 1.x | Error context chaining | Application-level error wrapping at boundaries |
| `tracing` + `tracing-subscriber` | 0.1, 0.3 | Structured logging to stderr | Diagnostic output orthogonal to `--json` stdout; Phase 3 F8 enables `json` feature for MCP log subscriber (JSON-lines to file via `src/mcp/log.rs`); custom `ContractEventFormat` renders contract-pinned field names; Phase 4 US4: includes summarisation progress, model download/verify operations, backend init steps; US5.a: doctor subsystem diagnostics logged at debug/warn levels |
| `sha2` | 0.10 | Content-addressed cache naming and model integrity | URL hashing for `cache_dir_for()` in `src/paths.rs`; model download verification; workspace registry deduplication; Phase 4 US4: content-hash diffing in summariser cache invalidation logic (input determinism matters for cache keys) |
| `regex` | 1.x | Credential scrubbing patterns | Git stderr sanitisation in `src/catalog/git.rs`; model download URLs; MCP log field scrubbing; Phase 4 US4: project path scrubbing in harness rules-file block insertion; US5.a: pattern matching in binding drift detection and harness rules/MCP config parsing |
| `ctrlc` | 3.x | Signal handling (SIGINT) | Global cancellation handler with exit code 8; cancels in-flight git operations, model downloads, and summarisation; Phase 3 Polish: explicit SIGTERM handler for MCP server (Unix-only) with 5s graceful-shutdown timeout; Phase 4 US4: interrupts in-flight LLM inference cleanly |
| `tempfile` | 3.x | Atomic file writes | Registry, per-catalog cache, models directory, manifest mutations, workspace init staging dir, project marker creation; `tempfile::Builder::tempdir_in` for same-filesystem POSIX-atomic rename; Phase 4 US4: `src/workspace/regen_summary` writes atomic temporary TOML before persisting workspace settings; US5.a: `.tome.tmp.*` orphan cleanup with filetime backdating for test isolation |
| `hex` | 0.4 | Hex encoding for SHA256 digests | Cache directory naming alongside sha2; model checksum comparison; Phase 4 US4: digest comparison in summariser model verification; US5.a: drift detection hex comparisons |
| `semver` | 1.x | Semantic version parsing | Catalog manifest version field validation; Phase 4 US4: model manifest version (for future schema evolution) |
| `time` | 0.3 | Timestamp formatting and parsing | Logging and manifest timestamps; RFC 3339 serialisation; Phase 4 US4: `CachedSummaries.generated_at` round-trip TOML datetime literals ↔ RFC 3339 strings; workspace summary outcome timestamps; US5.a: doctor timestamps in subsystem diagnostics |
| `rusqlite` | 0.32 (`bundled`) | Embedded SQLite (statically linked, no system dep) | `src/index/*` — the local skill index database; WAL mode + advisory lockfile (FR-040); workspace-scoped index in Phase 3 Foundational F1; Phase 4 US4: schema includes model metadata + workspace summary cache invalidation tracking; US5.a: binding state queries from workspace_projects table |
| `sqlite-vec` | vendored (v0.1.9) | KNN vector search extension for SQLite | `vendor/sqlite-vec/` compiled by `build.rs`; loaded via `sqlite3_auto_extension` in `src/index/vec_ext.rs` |
| `serde_yaml` | 0.9 | Lenient YAML frontmatter parsing for third-party inputs | `src/plugin/frontmatter.rs` — parses upstream SKILL.md metadata leniently (FR-013a boundary); Phase 4 US4: parses project `.tome/RULES.md` frontmatter leniently (project context metadata); US5.a: doctor binding-rules-copy drift detection via frontmatter parse |
| `fastembed` | 4.x | ONNX-backed text embedding + reranking | `src/embedding/fastembed.rs` — loads BGE models at runtime; CPU execution provider only; Phase 3 Polish: eager-load at MCP startup (FR-110); Phase 4 US4: unchanged (three inference runtimes — embedder, reranker, summariser — coexist); US5.a: model state included in doctor subsystem reporting |
| `reqwest` | 0.12 (`blocking`, `rustls-tls`, no defaults) | Synchronous HTTPS for model downloads | `src/embedding/download.rs` — downloads `MODEL_REGISTRY` entries (embedder, reranker, summariser) with SHA-256 verification and atomicity; credential scrubbing on error chains; US5.a: doctor can verify all three model artefacts in one pass |
| `indicatif` | 0.17 | Progress bars + spinners (TTY-aware) | `src/presentation/progress.rs` — download/reindex progress; refuses on non-TTY; Phase 4 US4: byte-progress on summariser model download; spinners during inference wait; US5.a: orphan cleanup progress when `--fix` is run |
| `comfy-table` | 7.x | Table rendering for human-mode list/show output | `src/presentation/tables.rs` — `tome plugin list`, `tome models list`, query results, doctor reports, `tome workspace list`, Phase 4 US4: `tome workspace regen-summary` progress table; US5.a: doctor subsystem table with health + suggested fixes |
| `owo-colors` | 4.x | Terminal colours with native `NO_COLOR` support | `src/presentation/colour.rs` — colourised output respecting `NO_COLOR` environment variable; Phase 4 US4: summarisation status colouring; US5.a: doctor subsystem health glyphs (✓/✗/⚠/→) colourised per status |
| `inquire` | 0.7 (`crossterm`, no defaults) | Interactive Select/MultiSelect/Confirm prompts | `src/presentation/prompt.rs` — interactive plugin enable/disable/list/show; bare `tome plugin` browse flow; doctor repair confirmation; refusal to perform destructive ops on non-TTY; Phase 4 US4: `workspace regen-summary` confirmation prompt if cache is stale; US5.a: doctor `--fix` confirmation per subsystem before applying repairs |
| `rmcp` | 1.x (`transport-io`, `schemars` features) | MCP protocol and stdio server | `src/mcp/mod.rs`, `src/mcp/server.rs` — tool router and handler registration; stdin/stdout channel per FR-221; Phase 4 US4: unchanged (tools remain read-only; summarisation is triggerable only via CLI commands, not MCP); US5.a: MCP input length cap on query (4096 chars) via `MAX_QUERY_CHARS` constant |
| `tokio` | 1.x (`rt`, `macros`, `io-std`, `sync`, `signal`, `time` features) | Async runtime backing MCP server | `src/mcp/runtime.rs` — single-threaded `Builder::new_current_thread` only; scoped via `tests/sync_boundary.rs`; Phase 4 US4: unchanged (summariser stays sync-only, no async); US5.a: no async changes (doctor stays sync throughout) |
| `schemars` | 1.x | JSON Schema derivation for MCP tool I/O | `src/mcp/tools/{search_skills,get_skill}.rs` — contract-compliant schema generation; Phase 4 US4: unchanged (tool I/O schemas don't expose summarisation state); US5.a: `MAX_QUERY_CHARS` constraint exported for schema introspection |
| `llama-cpp-2` | 0.1 (exact-pinned at `=0.1.146`) | Rust bindings to `llama.cpp` | Phase 4 F6–US4: `src/summarise/` — LLM inference for workspace summaries (Qwen2.5-0.5B-Instruct GGUF); sync API, process-wide `LlamaBackend` singleton via `std::sync::OnceLock` + Mutex-guarded init; US4.a ships production `LlamaSummariser`, US4.d-1 completes model SHA-256 pinning + cache integration + trigger wiring; US5.a: model state included in doctor diagnostics |
| `encoding_rs` | 0.8 | Character encoding for LLM tokenization | Phase 4 US4: `src/summarise/llama.rs` — `LlamaModel::token_to_piece` requires `&mut encoding_rs::Decoder` argument (not re-exported by `llama-cpp-2`); promotes from transitive to direct dep to ensure version alignment across dependency tree; US5.a: no encoding changes (doctor doesn't invoke summariser) |

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
| **Binary Size** | < 50 MB stripped on release builds (enforced by CI); Phase 4 / F1–F11 + US1–US4 projection: ~28.8 MiB on macOS arm64, ~34.5 MB on Linux x86_64; Phase 4 US4 adds llama-cpp-2 LLM inference (C++ static lib + Qwen2.5-0.5B GGUF at runtime) with minimal overhead due to static linking + thin LTO; US5.a adds doctor subsystem enums (no new binary weight — type promotion only) |
| **Output** | Human-readable (default) or NDJSON (`--json`); logging to stderr only; colours respect `NO_COLOR` and auto-disable on non-TTY; Phase 4 US4: `tome workspace regen-summary` output includes progress + status glyphs; harness sync outcomes rendered with status; US5.a: `tome doctor` extended to report five Phase 4 subsystems with health + fixes |
| **Model runtime** | CPU-only ONNX Runtime (via `fastembed`, embedder + reranker); `llama.cpp` via `llama-cpp-2` (summariser, Qwen2.5-0.5B-Instruct GGUF); models downloaded at first use into `${XDG_DATA_HOME}/tome/models/`; fixed registry ensures reproducibility; Phase 4 US4: all three models pre-downloaded or verified before first sync/summary operation; US5.a: doctor can verify all three in one pass with `--verify` |
| **MCP server runtime** | Single-threaded tokio with JSON-lines file logging to `${XDG_STATE_HOME}/tome/mcp.log` (10 MiB rotation cap); stdout reserved for MCP protocol only; stderr for fatal startup errors (FR-222); SIGTERM handler with 5s graceful-shutdown timeout (Unix-only); US5.a: input length cap (4096 chars on search_skills.query) enforced at handler entry |
| **Workspace storage** | Atomic `.tome/` directories via `tempfile::Builder::tempdir_in` + POSIX rename; config at `${WORKSPACE}/.tome/config.toml`; settings at `${WORKSPACE}/.tome/settings.toml` with `[summaries]` table (Phase 4 US4 caches short/long summaries + RFC 3339 generated_at timestamp); index DB at `${WORKSPACE}/.tome/index.db`; catalog clones in `${WORKSPACE}/.tome/catalogs/<sha>/`; Phase 4 US4: summary cache invalidation tied to plugin enable/disable/reindex/catalog update triggers; US5.a: doctor reports binding state + detected orphaned bindings |
| **Project binding** | Phase 4 US1: atomic `.tome/` marker directory inside project root (e.g. `~/my-project/.tome/config.toml` containing binding identity); binding records workspace name and project path in central DB under advisory lock; marker landing atomic via `tempfile::Builder::tempdir_in` + rename pattern; US5.a: doctor detects binding drift + rules-copy state, classifies as Degraded/Unhealthy per FR-561 |
| **Configuration** | Global: `${XDG_CONFIG_HOME}/tome/settings.toml` (Phase 4 F8+); Workspace: `${WORKSPACE}/.tome/settings.toml` (Phase 4 F8+, includes `[summaries]` table per US4); Project: `${PROJECT}/.tome/config.toml` (Phase 4 US1, binding marker); Phase 4 US4: project `.tome/RULES.md` with YAML frontmatter + Markdown body (context + rules for summarisation); US5.a: doctor validates composition resolver output (effective harness list) |
| **Harness configuration** | Per-harness files (e.g., `~/.claude/.mcp.json`, `~/.codex/.mcp.toml`); Phase 4 US3: read-modify-write via `HarnessModule` trait dispatch; atomic writes via `toml_edit` (Codex) or `serde_json` (others); Phase 4 US4: unchanged (harness sync stays independent of summarisation); US5.a: doctor detects per-harness rules-file + MCP-config drift; classifies UserOwned (user modified) separately from Drift (Tome expects different state) |
| **Summariser caching** | Phase 4 US4: per-workspace `[summaries]` table in `${WORKSPACE}/.tome/settings.toml` with `short_summary`, `long_summary`, `generated_at` (RFC 3339 datetime literal); regenerated via triggers (enable/disable/reindex/catalog update/explicit `regen-summary` command) or on-demand via `tome workspace regen-summary`; US5.a: doctor reports cache presence + staleness (content-hash mismatch → Degraded) |

## Strictness & Isolation Boundaries

| Boundary | Principle |
|----------|-----------|
| **Tome-owned inputs** | Strict parsing (`#[serde(deny_unknown_fields)]`): config, model manifests, index schema, multi-level settings (project/workspace/global), cached summaries table; Phase 4 US4: includes `CachedSummaries` struct in workspace settings; US5.a: ProjectBindingState, EffectiveHarnessList, HarnessSubsystemReport all strict |
| **Third-party inputs** | Lenient parsing: plugin manifests, SKILL.md frontmatter, project `.tome/RULES.md` frontmatter — forward-compatible; Phase 4 US4: `.tome/RULES.md` body is Markdown (not validated, any frontmatter unknown fields ignored); US5.a: binding drift detection handles lenient frontmatter gracefully |
| **Async isolation** | All async code confined to `src/mcp/`; structural test `tests/sync_boundary.rs` enforces boundary; Phase 4 US4: summariser (sync throughout) stays outside async; US5.a: doctor (sync throughout) stays outside async |
| **Sync enforcement** | Pre-commit hook runs `cargo test` with sync-boundary test; CI gates all PRs on boundary enforcement; Phase 4 US4: workspace summary regeneration stays sync-only (llama-cpp-2 is sync, calls happen in blocking spawn if ever inside MCP); US5.a: doctor passes and repairs (no async) |

## Feature Enablement

- `serde_json` gained `preserve_order` feature (Phase 4 F5) to maintain key ordering in all JSON output including harness config; US5.a: preserves Subsystem variant serialisation order
- `toml_edit` enables comment/order preservation for harness MCP config, workspace settings, and project `.tome/RULES.md` read-modify-write (Phase 4 F1+); Phase 4 US4: workspace `regen-summary` uses `toml_edit` to preserve `[summaries]` table structure and surrounding comments; US5.a: no new toml_edit usage (doctor is read-only except `--fix` repairs which land in US5.b)
- `tracing-subscriber` uses `json` feature for MCP log formatting; US5.a: doctor diagnostic logs at debug level for subsystem checks
- Phase 4 US4: no new feature flags required; summarisation uses stable `llama-cpp-2` without optional features; US5.a: no new feature flags (Subsystem enum is type promotion, not new deps)

---

## What Does NOT Belong Here

- Directory structure → STRUCTURE.md
- System design patterns → ARCHITECTURE.md
- External service integrations → INTEGRATIONS.md
- Dev tools (linting, formatting) → CONVENTIONS.md
- Test frameworks → TESTING.md

---

*This document captures what executes in Tome at Phase 4 Foundational F1–F11 + US1–US5.a complete (v0.4.0 release). Phase 4 adds `llama-cpp-2`, `toml_edit`, and `serde_json/preserve_order` for summarisation, project binding infrastructure, multi-level settings composition framework (fully wired), workspace lifecycle with atomic marker relocation, harness module abstraction with five concrete impls, full sync algorithm, workspace summary caching, and doctor subsystem categorization. Phase 4 US5.a ships production `LlamaSummariser` with Qwen2.5-0.5B-Instruct GGUF model (SHA-256 pinned 2026-05-26), integrated into workspace summary regeneration pipeline with cache invalidation tied to plugin/catalog lifecycle triggers. US5.a extends `tome doctor` to report five Phase 4 subsystems (project binding, binding-rules-copy, summariser model, harness-rules per harness, harness-mcp per harness) with typed `Subsystem` enum maintaining byte-identical JSON wire format to Phase 3. MCP `search_skills` tool enforces 4096-char input length cap. Binary size projection remains ~28–34 MB, well under the 50 MB cap. Test count: 490 → 894 across 64 → 122 suites.*

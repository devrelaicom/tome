# Technology Stack

> **Purpose**: Document what executes in this codebase - languages, runtimes, frameworks, and critical dependencies.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13 (Phase 5 User Story 2 — `tome plugin disable` subcommand + re-enable verification)

## Languages & Runtimes

| Language | Version | Purpose |
|----------|---------|---------|
| Rust | stable (MSRV: 1.93) | Primary implementation language; synchronous (no async runtime in Phase 1–5) |

## Frameworks

Phase 1–5 is a CLI application, not a web framework-based project.

| Framework | Version | Purpose |
|-----------|---------|---------|
| clap | 4.x | CLI argument parsing and help/version generation; bare `tome plugin` (no subcommand) dispatches to interactive flow via `Option<PluginCommand>` derive pattern |

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

- **Async runtime**: No `tokio`, `async-std`, or similar. Phase 1–5 remains synchronous (`reqwest::blocking`, `rusqlite`, `fastembed`); the MCP server is the future forcing function.
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

*This document captures only what executes. It reflects the actual Cargo.toml, Cargo.lock, and Phase 1–5 source code.*

# Technology Stack

> **Purpose**: Document what executes in this codebase - languages, runtimes, frameworks, and critical dependencies.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-11

## Languages & Runtimes

| Language | Version | Purpose |
|----------|---------|---------|
| Rust | stable (MSRV: 1.93) | Primary implementation language; synchronous (no async runtime in Phase 1) |

## Frameworks

Phase 1 is a CLI application, not a web framework-based project.

| Framework | Version | Purpose |
|-----------|---------|---------|
| clap | 4.x | CLI argument parsing and help/version generation |

## Critical Dependencies

| Package | Version | Purpose | Usage Scope |
|---------|---------|---------|-------------|
| `serde` + `serde_derive` | 1.x | Configuration and manifest (de)serialisation | All TOML parsing for `config.toml` and `tome-catalog.toml` |
| `toml` | 0.8 | TOML format support | Manifest and config file parsing |
| `thiserror` | 2.x | Typed error enums | Closed `TomeError` enum in `src/error.rs` (all fallible operations) |
| `anyhow` | 1.x | Error context chaining | Application-level error wrapping at boundaries |
| `tracing` + `tracing-subscriber` | 0.1, 0.3 | Structured logging to stderr | Diagnostic output orthogonal to `--json` stdout |
| `sha2` | 0.10 | Content-addressed cache naming | URL hashing for `cache_dir_for()` in `src/paths.rs` |
| `regex` | 1.x | Credential scrubbing patterns | Git stderr sanitisation in `src/catalog/git.rs` (4 regex patterns) |
| `ctrlc` | 3.x | Signal handling (SIGINT) | Global cancellation handler with exit code 8 |
| `tempfile` | 3.x | Atomic file writes | Registry and per-catalog cache mutations |
| `hex` | 0.4 | Hex encoding for SHA256 digests | Cache directory naming alongside sha2 |
| `semver` | 1.x | Semantic version parsing | Catalog manifest version field validation |
| `time` | 0.3 | Timestamp formatting and parsing | Logging and manifest timestamps |
| `serde_json` | 1.x | JSON serialisation (NDJSON output) | `--json` mode formatting for stdout |

## Package Managers & Build Tools

| Tool | Version | Purpose |
|------|---------|---------|
| Cargo | (bundled with Rust) | Workspace management and builds |
| rustfmt | (pinned in rust-toolchain.toml) | Code formatting |
| clippy | (pinned in rust-toolchain.toml) | Linting with `-D warnings` |

## Runtime Environment

| Environment | Details |
|-------------|---------|
| OS Targets | Linux (ubuntu-latest) and macOS (macos-latest) — CI verified on both |
| Deployment | Single binary (`target/release/tome`); installed via `cargo install --path .` |
| Binary Size | < 10 MB stripped (enforced by CI; see `.github/workflows/ci.yml`) |
| Output | Human-readable (default) or NDJSON (`--json`); logging to stderr only (orthogonal to stdout) |

## Not Used (Explicitly Excluded)

- **Async runtime**: No `tokio`, `async-std`, or similar. Phase 1 is synchronous; async deferred to Phase 2 (MCP server).
- **Git library**: No `libgit2`, `git2`, or vendored Git. `std::process::Command` shells out to system `git` (constitution principle XII).
- **HTTP client**: Not needed in Phase 1. Future HTTP integration (e.g., remote catalog sources) deferred to Phase 2.
- **Database**: No SQLite, PostgreSQL, or embedded database in Phase 1. Phase 2 will introduce `sqlite-vec` and `fastembed-rs` for embeddings.

---

## What Does NOT Belong Here

- Directory structure → STRUCTURE.md
- System design patterns → ARCHITECTURE.md
- External service integrations → INTEGRATIONS.md
- Dev tools (linting, formatting) → CONVENTIONS.md
- Test frameworks → TESTING.md

---

*This document captures only what executes. It reflects the actual Cargo.toml, Cargo.lock, and Phase 1 source code.*

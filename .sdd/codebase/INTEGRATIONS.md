# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13 (Phase 5 slice 2 — `tome plugin disable` subcommand; no new external service integrations)
> **Updated**: 2026-05-13 (Phase 6 slice 1–2 — explicit model management CLI; no new external integrations)
> **Updated**: 2026-05-13 (Phase 7 slices 1–3 — reindex library + `tome catalog update` cascade + `tome reindex` CLI; no new external integrations)
> **Updated**: 2026-05-13 (Phase 8 slices 1–2 — `tome status [--verify]` health check + version pre-parse hook; no new external integrations)
> **Updated**: 2026-05-13 (Phase 9 slice 1 — `tome catalog remove` Phase 2 extensions; cascade-disable orchestrator; no new external integrations)
> **Updated**: 2026-05-14 (Foundational F7–F8 — schema migration framework + MCP server scaffolding; `src/mcp/` is now the live boundary for `tokio` + `rmcp` deps)

## Databases & Data Stores

### Local SQLite Index

| Service | Type | Purpose | Location |
|---------|------|---------|----------|
| SQLite 3 | Embedded relational DB | Local skill index — metadata, embeddings, reranker scores | `${XDG_DATA_HOME}/tome/index.db` (WAL mode); schema in `src/index/schema.rs` |

### Connection Patterns

- **Statically linked**: `rusqlite` with `bundled` feature — no system SQLite dependency, no version mismatch risk.
- **Concurrency model**: Single advisory lockfile (`index.lock`) ensures Phase 3–9 foreground operations are serialised; WAL mode allows readers during writes (future MCP server consideration).
- **ORM/Query builder**: Direct SQL via `rusqlite` — prepared statements, parameterised queries.
- **Migration approach**: Forward-only migrations under advisory lock in `src/index/migrations.rs` (rewritten in Foundational F7 with function-pointer-based `Migration` struct; see STACK.md Foundational F7 section); drift detection in `src/index/meta.rs`.

### Cache Structure

- **Catalog cache**: Each remote catalog source is content-addressed by `sha256(url)` in `${XDG_DATA_HOME}/tome/catalogs/<sha256>/` — Git working tree, refreshed on `tome catalog update`.
- **Model cache**: Downloaded model ONNX artefacts stored in `${XDG_DATA_HOME}/tome/models/<model-name>/` with per-model `manifest.json` (strict JSON, `#[serde(deny_unknown_fields)]`); managed explicitly via `tome models {download,list,remove}` (Phase 6).
- **Atomic writes**: `tempfile` crate (rename-based) prevents corruption on SIGINT; `.partial/` directories ensure no half-extracted state visible to concurrent processes.

---

## Authentication & Authorization

Phase 1–9 has no explicit application-layer authentication.

- **Git operations**: Inherit system SSH keys and HTTP credential helpers (if configured in `~/.gitconfig`).
- **Hugging Face model downloads**: No API key required; public `https://huggingface.co/` URLs are freely accessible (MODEL_REGISTRY pinned to MIT-licensed BGE variants).
- **Plugin manifest ownership**: File system permissions validate catalog ownership (email field in `tome-catalog.toml` is metadata only).
- **Credential scrubbing**: All Git stderr and model download error chains pass through `scrub_credentials()` before logging (principle XIII; extended in Phase 3 to cover HF URLs).

---

## External APIs

### First-Party APIs

None in Phase 1–9. Future phases may include:
- Remote catalog registries (HTTP/HTTPS URLs in catalog sources)
- Plugin resolver APIs (not specified)

### Third-Party APIs

#### Hugging Face Model Registry

| Provider | Purpose | SDK/Client | Configuration |
|----------|---------|------------|---------------|
| Hugging Face (`huggingface.co`) | ONNX model downloads (embedder + reranker) | `reqwest::blocking` (direct HTTPS) | `src/embedding/registry.rs` — `MODEL_REGISTRY` (compile-time constants) |

**Details**:
- **Embedder**: `bge-small-en-v1.5` INT8 (~66 MB) from `qdrant/bge-small-en-v1.5-onnx-Q`
- **Reranker**: `bge-reranker-base` INT8 (~280 MB) from `onnx-community/bge-reranker-base-ONNX` (source moved in Phase 3 from BAAI — they no longer host quantised ONNX)
- **Integrity**: Pinned SHA-256 + size_bytes verified post-download (no checksum endpoint; hashes are real upstream digests verified at Phase 3 slice 1 start)
- **Network**: HTTPS only via `rustls-tls` (no system OpenSSL)
- **Failure mode**: Network error → `TomeError::Io` (exit 7); checksum mismatch → `TomeError::ModelChecksumMismatch` (exit 32); corrupted registry → `TomeError::ModelCorrupt` (exit 31)
- **Explicit management**: Phase 6 wires `tome models {download,list,remove}` to let users explicitly manage artefacts; `tome models list --verify` invokes SHA-256 per-file validation via `embedding::download::sha256_file()`
- **Status visibility**: Phase 8 adds `tome status [--verify]` to audit model directory state without triggering downloads; per-model validation only runs when `--verify` is set

---

## Message Queues & Event Systems

None in Phase 1–9. Deferred to Phase 6+ (MCP server event streaming).

---

## Caching

| Service | Purpose | TTL / Eviction | Configuration |
|---------|---------|----------------|---------------|
| Filesystem (XDG) | Catalog Git working trees | Explicit `tome catalog remove` (user-managed); persistent | `${XDG_DATA_HOME}/tome/catalogs/` — git-based, refreshed on `tome catalog update` |
| Filesystem (XDG) | Downloaded model artefacts | Explicit `tome models remove` (user-managed); persistent | `${XDG_DATA_HOME}/tome/models/` — one dir per model with manifest + ONNX files; Phase 6 adds explicit user-facing commands |

No TTL-based eviction. Phase 1–9 uses explicit user commands for cleanup (principle VI: KISS).

---

## Monitoring & Observability

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| Structured logging (via `tracing`) | Diagnostic tracing to stderr (CLI) and JSON-lines to file (MCP server) | CLI: `RUST_LOG` or `TOME_LOG` environment variables; independent of `--json` stdout mode. MCP: JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log` per `contracts/log-format.md`; 10 MiB rotation cap; stderr reserved for fatal startup errors only per FR-222 |
| Exit codes | Scriptable error handling | 18+ enumerated codes (Phase 2: 0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 13, 14, 30, 31, 32); Phase 3 adds codes 60–61 (MCP), 70–75 (workspace/schema including exit 73 for write-path schema version too new); documented in `specs/002-phase-2-plugins-index/contracts/exit-codes.md` and `specs/003-phase-3-mcp-workspaces/contracts/exit-codes-p3.md` |
| Status checks | Per-subsystem health via `tome status` | Phase 8 — health report includes models, index, drift state; lazy validation with `--verify` flag |

---

## File Storage

| Service | Purpose | Configuration |
|---------|---------|---------------|
| XDG-compliant filesystem | Configuration, catalogs, models, index | `${XDG_CONFIG_HOME}`, `${XDG_DATA_HOME}`, `${XDG_STATE_HOME}` (principle XII: inherit, don't reimplement); Phase 6 adds explicit model lifecycle commands; Phase 8 adds read-only audit via `tome status [--verify]`; Phase 9 extends catalog removal with cascade-disable index cleanup; Foundational F8 adds MCP log to `${XDG_STATE_HOME}/tome/mcp.log` |

---

## Email & Notifications

None in Phase 1–9.

---

## Environment Variables

| Variable | Required | Purpose | Example | Updated Phase |
|----------|----------|---------|---------|---------------|
| `HOME` | Yes | Base directory for XDG path resolution | `/Users/aaronbassett` | — |
| `XDG_CONFIG_HOME` | No (defaults to `~/.config`) | Override config directory | `/opt/etc` | — |
| `XDG_DATA_HOME` | No (defaults to `~/.local/share`) | Override data directory (models, catalogs, index.db) | `/opt/var` | — |
| `XDG_STATE_HOME` | No (defaults to `~/.local/state`) | Override state directory (MCP log) | `/opt/state` | Foundational F8 |
| `TOME_LOG` | No | Custom log filter (overrides `RUST_LOG`) | `debug`, `info`, `tome=trace` | — |
| `RUST_LOG` | No | Standard Rust log filter | `info`, `warn` | — |
| `NO_COLOR` | No | Disable coloured output (per CLICOLOR spec) | (presence enables) | phase 3: extended to cover presentation layers (`owo-colors` native support, `inquire` respects it); phase 4: interactive browse flow respects `NO_COLOR`; phase 5: disable subcommand respects `NO_COLOR`; phase 6: models commands respect `NO_COLOR`; phase 8: status report respects `NO_COLOR`; phase 9: cascade-disable output respects `NO_COLOR` |
| `CLICOLOR` | No | Disable coloured output (alternate) | `0` to disable | — |

---

## System Dependencies

### Required

- `git` (system binary) — for catalog cloning/updating/checkout
- `libc` — standard C library (bundled with system)

### Optional

- **SSH keys** (`~/.ssh/id_*`) — if catalogs use SSH URLs
- **Git credential helper** — if catalogs use HTTPS URLs without embedded credentials

### Not Required

- System OpenSSL (Tome uses `rustls` — statically linked)
- System SQLite (Tome uses `rusqlite bundled` — statically linked)
- ONNX Runtime shared library (Tome uses static `ort` via `fastembed` — bundled in binary)
- `libtokio` or system async libraries (Foundational F8 brings in `tokio`, which is statically linked; scoped to `src/mcp/` only)

---

## Git Integration Details

| Aspect | Details |
|--------|---------|
| **Cloning** | `git clone <url> <path>` — full shallow or full history depends on catalog source |
| **Fetching** | `git fetch origin` — refreshes cached remote refs |
| **Checking out** | `git checkout <ref>` — pins catalog to specific commit/tag/branch |
| **Resetting** | `git reset --hard HEAD` — discards local changes (on `tome catalog update`) |
| **Credential flow** | SSH: SSH agent or `~/.ssh/id_*` keys; HTTPS: `git credential` helper or inline auth (if present in URL) |
| **Signal handling** | SIGINT (Ctrl+C) kills child `git` process; sets exit code 8; no zombie procs (reaps via `std::process::wait()`) |
| **Error scrubbing** | Captured stderr passed through `scrub_credentials()` before logging — covers URLs, tokens, SSH keys, long hex strings (principle XIII) |

---

## Third-Party Manifest Parsing

| Format | Location | Strictness | Purpose |
|--------|----------|-----------|---------|
| `plugin.json` | Catalog plugin dirs | Lenient (unknown fields ignored) | Third-party plugin metadata (FR-013a boundary) |
| SKILL.md YAML frontmatter | Upstream plugin repos | Lenient (unknown fields ignored) | Third-party skill/agent/command/hook metadata; parsed by `serde_yaml` without validation |
| `tome-catalog.toml` | Catalog root | Strict (`deny_unknown_fields`) | Tome-owned manifest; validates all fields |
| `config.toml` | `${XDG_CONFIG_HOME}/tome/` | Strict (`deny_unknown_fields`) | Tome-owned user config; rejects typos early |

---

## MCP Server Integration (Phase 3 Foundational F8)

**Status:** Scaffolding complete; server loop + tool registration lands in US1 (T076).

| Aspect | Details |
|--------|---------|
| **Protocol** | `rmcp` (1.x) — Model Context Protocol stdio server per `contracts/mcp-server.md` |
| **Runtime** | Single-threaded `tokio` (`Builder::new_current_thread`) backing async surfaces in `src/mcp/` (research §R-2) |
| **Process model** | Stdio: stdin = MCP protocol messages, stdout = MCP responses; stderr reserved for fatal startup errors only (FR-222) |
| **Logging** | JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log` at application level; rotation at 10 MiB with backoff to `mcp.log.1` per `contracts/log-format.md`; tracing subscriber with `json` feature enabled in Cargo.toml |
| **Pre-flight** | FR-110 startup pipeline (schema check → drift detect → SHA-256 verify → eager-load FastembedEmbedder) scoped to `src/mcp/preflight.rs`; currently landing as surfaces only (implementation lands US1) |
| **Tool integration** | Embedder + reranker loaded once at startup, shared across tool calls; no per-call model reloads |
| **Error handling** | Fatal startup errors (schema too new, drift, embedder load fail) emitted to stderr + log, exit code 60 (`McpStartupFailed`) or 61 (`McpProtocolIo`) |
| **Sync boundary** | All async/tokio code lives strictly in `src/mcp/`; main CLI stays synchronous with boundary enforced by `tests/sync_boundary.rs` (structural test) |

---

## What Does NOT Belong Here

- Internal code architecture → ARCHITECTURE.md
- Testing infrastructure → TESTING.md
- Security policies → SECURITY.md
- Dependency versions → STACK.md

---

*This document maps external service dependencies and failure modes. Updated for Foundational F7–F8: schema migration framework rewrite + MCP server scaffolding scoped to `src/mcp/` with `tokio` + `rmcp` dependencies.*

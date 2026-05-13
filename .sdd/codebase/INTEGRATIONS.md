# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-13 (Phase 3 slice 1 — model registry real digests, query command, plugin lifecycle)

## Databases & Data Stores

### Local SQLite Index

| Service | Type | Purpose | Location |
|---------|------|---------|----------|
| SQLite 3 | Embedded relational DB | Local skill index — metadata, embeddings, reranker scores | `${XDG_DATA_HOME}/tome/index.db` (WAL mode); schema in `src/index/schema.rs` |

### Connection Patterns

- **Statically linked**: `rusqlite` with `bundled` feature — no system SQLite dependency, no version mismatch risk.
- **Concurrency model**: Single advisory lockfile (`index.lock`) ensures Phase 3–4 foreground operations are serialised; WAL mode allows readers during writes (future MCP server consideration).
- **ORM/Query builder**: Direct SQL via `rusqlite` — prepared statements, parameterised queries.
- **Migration approach**: Forward-only migrations under advisory lock in `src/index/migrations.rs`; drift detection in `src/index/meta.rs`.

### Cache Structure

- **Catalog cache**: Each remote catalog source is content-addressed by `sha256(url)` in `${XDG_DATA_HOME}/tome/catalogs/<sha256>/` — Git working tree, refreshed on `tome catalog update`.
- **Model cache**: Downloaded model ONNX artefacts stored in `${XDG_DATA_HOME}/tome/models/<model-name>/` with per-model `manifest.json` (strict JSON, `#[serde(deny_unknown_fields)]`).
- **Atomic writes**: `tempfile` crate (rename-based) prevents corruption on SIGINT; `.partial/` directories ensure no half-extracted state visible to concurrent processes.

---

## Authentication & Authorization

Phase 1–3 has no explicit application-layer authentication.

- **Git operations**: Inherit system SSH keys and HTTP credential helpers (if configured in `~/.gitconfig`).
- **Hugging Face model downloads**: No API key required; public `https://huggingface.co/` URLs are freely accessible (MODEL_REGISTRY pinned to MIT-licensed BGE variants).
- **Plugin manifest ownership**: File system permissions validate catalog ownership (email field in `tome-catalog.toml` is metadata only).
- **Credential scrubbing**: All Git stderr and model download error chains pass through `scrub_credentials()` before logging (principle XIII; extended in Phase 3 to cover HF URLs).

---

## External APIs

### First-Party APIs

None in Phase 1–3. Future phases may include:
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

---

## Message Queues & Event Systems

None in Phase 1–3. Deferred to Phase 5+ (MCP server event streaming).

---

## Caching

| Service | Purpose | TTL / Eviction | Configuration |
|---------|---------|----------------|---------------|
| Filesystem (XDG) | Catalog Git working trees | Explicit `tome catalog remove` (user-managed); persistent | `${XDG_DATA_HOME}/tome/catalogs/` — git-based, refreshed on `tome catalog update` |
| Filesystem (XDG) | Downloaded model artefacts | Explicit `tome models remove` (user-managed); persistent | `${XDG_DATA_HOME}/tome/models/` — one dir per model with manifest + ONNX files |

No TTL-based eviction. Phase 1–3 uses explicit user commands for cleanup (principle VI: KISS).

---

## Monitoring & Observability

| Service | Purpose | Configuration |
|---------|---------|---------------|
| Structured logging (via `tracing`) | Diagnostic tracing to stderr | `RUST_LOG` or `TOME_LOG` environment variables; independent of `--json` stdout mode |
| Exit codes | Scriptable error handling | 18+ enumerated codes (0, 2, 3, 4, 5, 7, 8, 9, 10, 13, 14, 30, 31, 32); documented in `specs/002-phase-2-plugins-index/contracts/exit-codes.md` |

---

## File Storage

| Service | Purpose | Configuration |
|---------|---------|---------------|
| XDG-compliant filesystem | Configuration, catalogs, models | `${XDG_CONFIG_HOME}`, `${XDG_DATA_HOME}` (principle XII: inherit, don't reimplement) |

---

## Email & Notifications

None in Phase 1–3.

---

## Environment Variables

| Variable | Required | Purpose | Example | Updated Phase 3 |
|----------|----------|---------|---------|-----------------|
| `HOME` | Yes | Base directory for XDG path resolution | `/Users/aaronbassett` | — |
| `XDG_CONFIG_HOME` | No (defaults to `~/.config`) | Override config directory | `/opt/etc` | — |
| `XDG_DATA_HOME` | No (defaults to `~/.local/share`) | Override data directory (models, catalogs, index.db) | `/opt/var` | — |
| `TOME_LOG` | No | Custom log filter (overrides `RUST_LOG`) | `debug`, `info`, `tome=trace` | — |
| `RUST_LOG` | No | Standard Rust log filter | `info`, `warn` | — |
| `NO_COLOR` | No | Disable coloured output (per CLICOLOR spec) | (presence enables) | phase 3: extended to cover presentation layers (`owo-colors` native support, `inquire` respects it) |
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

## What Does NOT Belong Here

- Internal code architecture → ARCHITECTURE.md
- Testing infrastructure → TESTING.md
- Security policies → SECURITY.md
- Dependency versions → STACK.md

---

*This document maps external service dependencies and failure modes. Updated for Phase 3 slice 1 with real model SHA-256 digests and query/plugin lifecycle integrations.*

# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-05-11
> **Last Updated**: 2026-05-14 (Phase 3 / US2 — `tome workspace info` + `tome workspace init` commands; workspace-scoped paths + registry file)

## Databases & Data Stores

### Local SQLite Index

| Service | Type | Purpose | Location |
|---------|------|---------|----------|
| SQLite 3 | Embedded relational DB | Local skill index — metadata, embeddings, reranker scores | Global: `${XDG_DATA_HOME}/tome/index.db` (WAL mode); Workspace: `${WORKSPACE}/.tome/index.db` (Phase 3 Foundational F1); schema in `src/index/schema.rs` |

### Connection Patterns

- **Statically linked**: `rusqlite` with `bundled` feature — no system SQLite dependency, no version mismatch risk.
- **Concurrency model**: Single advisory lockfile (`index.lock` — global or workspace-scoped per Phase 3 Foundational F1) ensures Phase 3–9 foreground operations are serialised; WAL mode allows readers during writes (MCP server uses read-only open per FR-056).
- **ORM/Query builder**: Direct SQL via `rusqlite` — prepared statements, parameterised queries.
- **Migration approach**: Forward-only migrations under advisory lock in `src/index/migrations.rs` (rewritten in Foundational F7 with function-pointer-based `Migration` struct; see STACK.md Foundational F7 section); drift detection in `src/index/meta.rs`.

### Cache Structure

- **Catalog cache**: Each remote catalog source is content-addressed by `sha256(url)` in `${XDG_DATA_HOME}/tome/catalogs/<sha256>/` (global scope) or `${WORKSPACE}/.tome/catalogs/<sha256>/` (workspace scope) — Git working tree, refreshed on `tome catalog update`.
- **Model cache**: Downloaded model ONNX artefacts stored in `${XDG_DATA_HOME}/tome/models/<model-name>/` (global, shared across scopes) with per-model `manifest.json` (strict JSON, `#[serde(deny_unknown_fields)]`); managed explicitly via `tome models {download,list,remove}` (Phase 6).
- **Atomic writes**: `tempfile` crate (rename-based) prevents corruption on SIGINT; `.partial/` directories ensure no half-extracted state visible to concurrent processes; workspace `init` uses `tempfile::Builder::tempdir_in(workspace_root)` for POSIX-atomic staging-to-final rename (Phase 3 / US2).

### Workspace Registry (Phase 3 / US2)

- **File**: `${XDG_DATA_HOME}/tome/workspaces.txt` — opt-in (never created unless explicitly requested)
- **Format**: Line-delimited absolute paths to workspace roots, one per line; dedupe by exact-path match
- **Semantics**: Informational only — tracks which workspaces have been initialized via `--inherit-global` or explicit `append_if_registry_exists` call (research §R-15)
- **Usage**: Client harnesses can read this file to discover initialized workspaces; Tome never requires it to exist

---

## Authentication & Authorization

Phase 1–9 has no explicit application-layer authentication. Phase 3 / US1 MCP server similarly has no auth mechanism — it is stdio-based (embedding in Claude Code harness provides transport-level auth). Phase 3 / US2 adds workspace scoping but no auth model changes.

- **Git operations**: Inherit system SSH keys and HTTP credential helpers (if configured in `~/.gitconfig`).
- **Hugging Face model downloads**: No API key required; public `https://huggingface.co/` URLs are freely accessible (MODEL_REGISTRY pinned to MIT-licensed BGE variants).
- **Plugin manifest ownership**: File system permissions validate catalog ownership (email field in `tome-catalog.toml` is metadata only).
- **Workspace ownership**: Implicitly owned by the user who runs `tome workspace init`; no explicit permission model (Phase 3 / US2).
- **Credential scrubbing**: All Git stderr and model download error chains pass through `scrub_credentials()` before logging (principle XIII; extended in Phase 3 to cover HF URLs).
- **MCP server identity** (Phase 3 / US1): Identified by `server_info { name: "tome", version: "0.x" }` in the MCP handshake; no per-call authentication.

---

## External APIs

### First-Party APIs

None in Phase 1–9. Phase 3 / US1 introduces internal library APIs:
- `commands::query::pipeline(args, deps) -> Result<QueryOutcome, TomeError>` — silent compute path reused by MCP `search_skills` tool (refactored from `run()` to avoid stdout/stderr emit)

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
- **Scope**: Models are global (shared across all workspaces and global scope); downloaded to `${XDG_DATA_HOME}/tome/models/` regardless of active scope (Phase 3 / US2)

---

## Message Queues & Event Systems

None in Phase 1–9. Phase 3 / US1 MCP server is stdio-based (single request/response at a time); no async event streaming.

---

## Caching

| Service | Purpose | TTL / Eviction | Configuration |
|---------|---------|----------------|---------------|
| Filesystem (XDG) | Catalog Git working trees | Explicit `tome catalog remove` (user-managed); persistent | Global: `${XDG_DATA_HOME}/tome/catalogs/` — git-based, refreshed on `tome catalog update`; Workspace: `${WORKSPACE}/.tome/catalogs/` (Phase 3 Foundational F1) |
| Filesystem (XDG) | Downloaded model artefacts | Explicit `tome models remove` (user-managed); persistent | `${XDG_DATA_HOME}/tome/models/` — one dir per model with manifest + ONNX files; Phase 6 adds explicit user-facing commands; shared across all scopes (global) |

No TTL-based eviction. Phase 1–9 uses explicit user commands for cleanup (principle VI: KISS).

---

## Monitoring & Observability

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| Structured logging (via `tracing`) | Diagnostic tracing to stderr (CLI) and JSON-lines to file (MCP server) | CLI: `RUST_LOG` or `TOME_LOG` environment variables; independent of `--json` stdout mode. MCP: JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log` per `contracts/log-format.md`; 10 MiB rotation cap; stderr reserved for fatal startup errors only per FR-222 |
| Exit codes | Scriptable error handling | 18+ enumerated codes (Phase 2: 0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 13, 14, 30, 31, 32); Phase 3 adds codes 60–61 (MCP), 70–75 (workspace/schema including exit 73 for write-path schema version too new); documented in `specs/002-phase-2-plugins-index/contracts/exit-codes.md` and `specs/003-phase-3-mcp-workspaces/contracts/exit-codes-p3.md` |
| Status checks | Per-subsystem health via `tome status` | Phase 8 — health report includes models, index, drift state; lazy validation with `--verify` flag; Phase 3 / US2 — `tome workspace info` reports scope identity + catalog/plugin/skill counts + embedder identity; no health semantics (informational only) |

---

## File Storage

| Service | Purpose | Configuration |
|---------|---------|---------------|
| XDG-compliant filesystem | Configuration, catalogs, models, index | Global: `${XDG_CONFIG_HOME}/tome/config.toml`, `${XDG_DATA_HOME}/tome/catalogs/<sha>/`, `${XDG_DATA_HOME}/tome/models/`, `${XDG_DATA_HOME}/tome/index.db`; Workspace: `${WORKSPACE}/.tome/config.toml`, `${WORKSPACE}/.tome/catalogs/<sha>/`, `${WORKSPACE}/.tome/index.db` (Phase 3 Foundational F1); Phase 6 adds explicit model lifecycle commands; Phase 8 adds read-only audit via `tome status [--verify]`; Phase 9 extends catalog removal with cascade-disable index cleanup; Foundational F8 adds MCP log to `${XDG_STATE_HOME}/tome/mcp.log`; Phase 3 / US1 MCP server operates on same index + models + config per scope; Phase 3 / US2 adds `${XDG_DATA_HOME}/tome/workspaces.txt` opt-in registry (research §R-15) and atomic `.tome/` dir creation via tempfile staging (staging dir inside workspace root for POSIX-atomic rename, chmod 0700 before content lands) |

---

## Email & Notifications

None in Phase 1–9.

---

## Environment Variables

| Variable | Required | Purpose | Example | Updated Phase |
|----------|----------|---------|---------|---------------|
| `HOME` | Yes | Base directory for XDG path resolution | `/Users/aaronbassett` | — |
| `XDG_CONFIG_HOME` | No (defaults to `~/.config`) | Override config directory | `/opt/etc` | — |
| `XDG_DATA_HOME` | No (defaults to `~/.local/share`) | Override data directory (models, catalogs, index.db, workspaces.txt) | `/opt/var` | — (US2 adds workspaces.txt location) |
| `XDG_STATE_HOME` | No (defaults to `~/.local/state`) | Override state directory (MCP log) | `/opt/state` | Foundational F8 |
| `TOME_LOG` | No | Custom log filter (overrides `RUST_LOG`) | `debug`, `info`, `tome=trace` | — |
| `RUST_LOG` | No | Standard Rust log filter | `info`, `warn` | — |
| `NO_COLOR` | No | Disable coloured output (per CLICOLOR spec) | (presence enables) | phase 3: extended to cover presentation layers (`owo-colors` native support, `inquire` respects it); phase 4: interactive browse flow respects `NO_COLOR`; phase 5: disable subcommand respects `NO_COLOR`; phase 6: models commands respect `NO_COLOR`; phase 8: status report respects `NO_COLOR`; phase 9: cascade-disable output respects `NO_COLOR`; phase 3/US1: MCP stdout is protocol-only (no color possible); phase 3/US2: workspace info respects `NO_COLOR` |
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
| `config.toml` | Global: `${XDG_CONFIG_HOME}/tome/`; Workspace: `${WORKSPACE}/.tome/` (Phase 3 / US2) | Strict (`deny_unknown_fields`) | Tome-owned user config; rejects typos early |

---

## MCP Server Integration (Phase 3 Foundational F8 + Phase 3 / US1)

**Status:** Server loop + tool registration landed (US1); live entry point is `tome mcp` CLI command.

| Aspect | Details |
|--------|---------|
| **Protocol** | `rmcp` (1.x) — Model Context Protocol stdio server per `contracts/mcp-server.md` |
| **Runtime** | Single-threaded `tokio` (`Builder::new_current_thread`) backing async surfaces in `src/mcp/` (research §R-2); runs on harness' blocking thread (no async in CLI main loop) |
| **Process model** | Stdio: stdin = MCP protocol messages (from harness), stdout = MCP responses; stderr reserved for fatal startup errors only (FR-222) |
| **Tools advertised** | Two: `search_skills` (perform semantic skill search via KNN + optional reranking) and `get_skill` (retrieve single skill detail by ID) |
| **Logging** | JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log` at application level; rotation at 10 MiB with backoff to `mcp.log.1` per `contracts/log-format.md`; tracing subscriber with `json` feature enabled in Cargo.toml |
| **Pre-flight** | FR-110 startup pipeline (schema check → drift detect → SHA-256 verify → eager-load FastembedEmbedder) scoped to `src/mcp/preflight.rs`; executed once on server startup |
| **Tool integration** | Embedder loaded once at startup (pre-flight), shared across tool calls; reranker lazily loaded on first tool call that requires ranking (via `tokio::sync::OnceCell`); no per-call model reloads (FR-005) |
| **Tool I/O schemas** | Input/output types in `src/mcp/tools/{search_skills,get_skill}.rs` use `#[derive(JsonSchema)]` from `schemars` crate to generate MCP-compliant schemas |
| **Index access** | Read-only; `src/mcp/tools/get_skill.rs` uses `index::skills::get_one_skill()` library helper; `search_skills.rs` delegates to refactored `commands::query::pipeline()` (silent compute path) |
| **Error handling** | Fatal startup errors (schema too new, drift, embedder load fail) emitted to stderr + log, exit code 60 (`McpStartupFailed`) or 61 (`McpProtocolIo`); tool errors mapped to MCP error responses |
| **Sync boundary** | All async/tokio code lives strictly in `src/mcp/`; CLI dispatches to MCP via `mcp::run(scope, paths)` which builds the runtime internally; structural test `tests/sync_boundary.rs` enforces boundary |
| **CLI entry** | `tome mcp` — new `Command::Mcp(McpArgs)` variant dispatched in `main.rs` before tracing/ctrlc init (special-case dispatch per FR-221) |

### Tool Details

#### `search_skills`

| Aspect | Details |
|--------|---------|
| **Purpose** | Semantic skill search: KNN embedding distance + optional reranking |
| **Input** | `SearchSkillsInput { query: String, limit: u32, force_strict: bool, ... }` — see `contracts/mcp-tools.md` for full schema |
| **Output** | `SearchSkillsOutput { skills: Vec<SkillResult>, ... }` — each result includes skill ID, name, catalog, match score, snippet |
| **Handler** | `pub async fn handle(input, state) -> Result<SearchSkillsOutput, impl Error>` in `src/mcp/tools/search_skills.rs` |
| **Reuse** | Delegates to `commands::query::pipeline(args, deps)` — the silent compute path (refactored from `run()` to avoid stdout/stderr) |
| **Reranker** | Lazily loaded on first invocation (unless `force_strict=true` disables ranking); shared across subsequent calls |

#### `get_skill`

| Aspect | Details |
|--------|---------|
| **Purpose** | Retrieve single skill full detail by ID |
| **Input** | `GetSkillInput { id: String }` — skill ID as `<catalog>/<plugin>/<skill-name>` |
| **Output** | `GetSkillOutput { skill: Option<SkillDetail>, ... }` — full text, metadata, or `None` if not found |
| **Handler** | `pub async fn handle(input, state) -> Result<GetSkillOutput, impl Error>` in `src/mcp/tools/get_skill.rs` |
| **Query** | Read-only index lookup via `index::skills::get_one_skill(id, conn)` library helper |

---

## Workspace Scope Integration (Phase 3 / US2)

**Status:** Workspace info + init commands landed (US2); scope-aware path resolution per Foundational F1.

| Aspect | Details |
|--------|---------|
| **Scope types** | Global (default, uses XDG paths) or Workspace (per `.tome/` directory); resolved via `Paths::resolve()` which walks `cwd` up the tree looking for `.tome/` marker or uses global on failure (research §R-15) |
| **Path model** | Per-scope `Paths` accessor methods: `Paths::config_file_for(&Scope)`, `Paths::index_db_for(&Scope)`, `Paths::index_lock_for(&Scope)` added in Foundational F1 to support both global + workspace scopes; existing Phase 1 fields retained for backward compat (convention F1) |
| **Config location** | Global: `${XDG_CONFIG_HOME}/tome/config.toml`; Workspace: `${WORKSPACE}/.tome/config.toml` (parse errors → `WorkspaceMalformed` exit 70 per US2.a) |
| **Index location** | Global: `${XDG_DATA_HOME}/tome/index.db`; Workspace: `${WORKSPACE}/.tome/index.db` (same WAL + advisory lock model; read-only on MCP server per FR-056) |
| **Info command** | `tome workspace info` (US2.a) — read-only scope report via `WorkspaceInfo` wire record; catalog/plugin/skill counts come from config + index; `ScopeSource` enum (flag/global-flag/env/cwd-walk/global-fallback) serialised with snake_case (e.g., `GlobalFlag` → `"global_flag"` in JSON) |
| **Init command** | `tome workspace init [<path>] [--inherit-global] [--force]` (US2.b) — atomic `.tome/` creation; staging dir inside workspace root (same FS) via `tempfile::Builder::tempdir_in`; POSIX-atomic final rename; `--inherit-global` seeds config with global catalogs (no enablement copy); `--force` replaces existing atomically (aside to `.tome.old/`, best-effort cleanup); pre-check refuses without `--force` (exit 4 / `CatalogAlreadyExists` shared with Phase 1 catalog case) |
| **Registry file** | `${XDG_DATA_HOME}/tome/workspaces.txt` — opt-in (created on-demand, never mandatory); line-delimited workspace roots; deduped by exact-path match; informational only (client harnesses can discover initialized workspaces; Tome doesn't require it) |
| **CLI wiring** | New `Command::Workspace(WorkspaceArgs)` + `WorkspaceCommand::{Info, Init}` variants in `src/cli.rs`; scope resolved once in `commands/workspace/mod.rs` dispatcher and threaded through each subcommand |

---

## What Does NOT Belong Here

- Internal code architecture → ARCHITECTURE.md
- Testing infrastructure → TESTING.md
- Security policies → SECURITY.md
- Dependency versions → STACK.md

---

*This document maps external service dependencies and failure modes. Updated for Phase 3 Foundational F7–F8 + Phase 3 / US1 + Phase 3 / US2: schema migration framework rewrite + MCP server scaffolding scoped to `src/mcp/` + workspace info/init commands with scope-aware path resolution and opt-in workspace registry file.*

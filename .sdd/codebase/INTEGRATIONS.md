# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-05-23
> **Last Updated**: 2026-05-23 (Phase 4 Foundational F1–F11 complete; 490 tests across 64 suites; v0.3.0 baseline + F1–F11 additions)

## Databases & Data Stores

### Local SQLite Index

| Service | Type | Purpose | Location |
|---------|------|---------|----------|
| SQLite 3 | Embedded relational DB | Local skill index — metadata, embeddings, reranker scores | Global: `${XDG_DATA_HOME}/tome/index.db` (WAL mode); Workspace: `${WORKSPACE}/.tome/index.db` (Phase 3 Foundational F1); schema in `src/index/schema.rs` |

### Connection Patterns

- **Statically linked**: `rusqlite` with `bundled` feature — no system SQLite dependency.
- **Concurrency model**: Single advisory lockfile (`index.lock` — global or workspace-scoped) serialises writes; WAL mode allows readers during writes; MCP server uses read-only open per FR-056; Phase 3 Polish: validators gate entry paths (malformed config / unopenable index → `WorkspaceMalformed` exit 70).
- **ORM/Query builder**: Direct SQL via `rusqlite` — prepared statements, parameterised queries.
- **Migration approach**: Forward-only migrations under advisory lock in `src/index/migrations.rs`; Phase 3 / US5 adds integration tests via synthetic-fixture injection in `tests/schema_migration_e2e.rs`; drift detection in `src/index/meta.rs`.

### Cache Structure

- **Catalog cache**: Each remote catalog source content-addressed by `sha256(url)` in `${XDG_DATA_HOME}/tome/catalogs/<sha256>/` (global) or `${WORKSPACE}/.tome/catalogs/<sha256>/` (workspace) — Git working tree, refreshed on `tome catalog update`. Multiple scopes can reference the same URL; shared via reference-count tracking — deleted only when no scope references it (Phase 3 / US3); Phase 4 F11: enrolment moved to `workspace_catalogs` junction table (sole source of truth per FR-360).
- **Model cache**: Downloaded model ONNX artefacts stored in `${XDG_DATA_HOME}/tome/models/<model-name>/` (global, shared across scopes) with per-model `manifest.json` (strict JSON, `#[serde(deny_unknown_fields)]`); Phase 6 adds explicit `tome models {download,list,remove}` commands; Phase 8 adds read-only audit via `tome status [--verify]`; Phase 4 F1: summariser model (Qwen2.5-0.5B-Instruct GGUF) added to registry alongside embedder/reranker.
- **Atomic writes**: `tempfile` crate (rename-based) prevents corruption on SIGINT; workspace `init` uses `tempfile::Builder::tempdir_in(workspace_root)` for POSIX-atomic staging-to-final rename (Phase 3 / US2); Phase 4 F4: `src/util/atomic_dir.rs` promoted as reusable helper for atomic-populated-directory operations.

### Workspace Registry (Phase 3 / US2, load-bearing in Phase 3 / US3)

- **File**: `${XDG_DATA_HOME}/tome/workspaces.txt` — opt-in (never created unless explicitly requested)
- **Format**: Line-delimited absolute paths to workspace roots; dedupe by exact-path match and canonicalize
- **Size cap**: 1 MiB; entry cap 10k (Phase 3 Polish hardening); no NUL or `..` path traversal sequences
- **Semantics**: Informational in US2; load-bearing in US3 — tracks which workspaces have been initialized via `--inherit-global`. US3 `catalog remove` consults this file to enumerate all scopes for reference-counting.
- **Usage**: Client harnesses can read this file to discover initialized workspaces; Tome treats absence as "no workspace scopes" (global scope only)

---

## Authentication & Authorization

Phase 1–4 has no explicit application-layer authentication. Phase 3 / US1 MCP server is stdio-based (embedding in harness provides transport-level security). Phase 4 / Foundational F1–F11 maintains the same posture.

- **Git operations**: Inherit system SSH keys and HTTP credential helpers (if configured in `~/.gitconfig`).
- **Hugging Face model downloads**: No API key required; public HTTPS URLs freely accessible (MODEL_REGISTRY pinned to MIT-licensed BGE variants + Apache-2.0 Qwen2.5).
- **Plugin manifest ownership**: File system permissions validate catalog ownership (email field in `tome-catalog.toml` is metadata only).
- **Workspace ownership**: Implicitly owned by the user who runs `tome workspace init`; no explicit permission model.
- **Credential scrubbing**: All Git stderr and model download error chains pass through `scrub_credentials()` before logging (principle XIII; extended to HF URLs and MCP log fields).
- **MCP server identity** (Phase 3 / US1): Identified by `server_info { name: "tome", version: "0.x" }` in the MCP handshake; no per-call authentication.
- **Doctor read-only access** (Phase 3 / US4): Diagnostics are read-only; repairs (`--fix`) require interactive confirmation.

---

## External APIs

### First-Party APIs

- `commands::query::pipeline(args, deps) -> Result<QueryOutcome, TomeError>` — silent compute path reused by MCP `search_skills` tool (Phase 3 / US1.b)
- Phase 4 F1–F11 continues to reuse library-level APIs without new external surfaces

### Third-Party APIs

#### Hugging Face Model Registry

| Provider | Purpose | SDK/Client | Configuration |
|----------|---------|------------|---------------|
| Hugging Face (`huggingface.co`) | ONNX + GGUF model downloads (embedder, reranker, summariser) | `reqwest::blocking` (direct HTTPS) | `src/embedding/registry.rs` — `MODEL_REGISTRY` (compile-time constants) |

**Details**:
- **Embedder**: `bge-small-en-v1.5` INT8 (~66 MB) from quantised variant
- **Reranker**: `bge-reranker-base` INT8 (~280 MB) from `onnx-community/bge-reranker-base-ONNX` (source moved Phase 3 slice 1)
- **Summariser** (Phase 4 F1+): `qwen2.5-0.5b-instruct` GGUF (~400 MB placeholder, real digest in US4) from `Qwen/Qwen2.5-0.5B-Instruct-GGUF`; Phase 4 F6 adds F1 placeholder with all-zero checksum guard (downloads refused until real digest landed in US4)
- **Integrity**: Pinned SHA-256 + size_bytes verified post-download; no checksum endpoint (hashes are real upstream digests verified at Phase 3 slice 1 start)
- **Network**: HTTPS only via `rustls-tls` (no system OpenSSL)
- **Failure modes**: Network error → `TomeError::Io` (exit 7); checksum mismatch → `TomeError::ModelChecksumMismatch` (exit 32); corrupted registry → `TomeError::ModelCorrupt` (exit 31); missing model → `TomeError::ModelMissing` (exit 30); embedder drift → `TomeError::EmbedderNameDrift` (exit 41); summariser placeholder → `TomeError::ModelCorrupt` (exit 31, per F1 design to surface as explicit failure)
- **Explicit management**: Phase 6 wires `tome models {download,list,remove}` to manage artefacts; `tome models list --verify` validates SHA-256 per-file via `embedding::download::sha256_file()`
- **Status visibility**: Phase 8 adds `tome status [--verify]` for read-only audit without triggering downloads
- **Doctor integration** (Phase 3 / US4): `tome doctor` reports model health with optional repair via `--fix`; Phase 3 Polish: specific exit codes for name mismatch vs missing
- **Scope**: Models are global (shared across all workspaces); downloaded to `${XDG_DATA_HOME}/tome/models/` regardless of active scope

---

## Message Queues & Event Systems

None. Phase 3 / US1 MCP server is stdio-based (single request/response); Phase 4 Foundational adds no async event infrastructure. Phase 3 Polish: explicit SIGTERM handler for graceful shutdown (Unix-only) with 5s timeout.

---

## Caching

| Service | Purpose | TTL / Eviction | Configuration |
|---------|---------|----------------|-----------------|
| Filesystem (XDG) | Catalog Git working trees | Explicit `tome catalog remove` (user-managed); persistent; shared across scopes via refcount (Phase 3 / US3) | Global: `${XDG_DATA_HOME}/tome/catalogs/`; Workspace: `${WORKSPACE}/.tome/catalogs/` (Phase 3 Foundational F1); same URL reused — clone deleted only when all scopes drop it (Phase 3 / US3); Phase 3 Polish: orphan clones reported by doctor |
| Filesystem (XDG) | Downloaded model artefacts | Explicit `tome models remove` (user-managed); persistent | `${XDG_DATA_HOME}/tome/models/` — one dir per model with manifest + ONNX/GGUF files; shared across all scopes (global); Phase 3 / US4 doctor can remove corrupt models |

No TTL-based eviction. Explicit user commands for cleanup (principle VI). Phase 3 Polish: doctor provides advisory cleanup candidates.

---

## Monitoring & Observability

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| Structured logging (via `tracing`) | Diagnostic tracing to stderr (CLI) and JSON-lines to file (MCP server) | CLI: `RUST_LOG` or `TOME_LOG` environment variables; independent of `--json` stdout. MCP: JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log` per `contracts/log-format.md`; 10 MiB rotation cap; stderr reserved for fatal startup errors only (FR-222); Phase 3 Polish: custom `ContractEventFormat` emits contract-pinned field names (`ts`, `level`, `target`, `msg`); log file 0600 mode (Unix-only); credential scrubbing on `workspace_path` and `error_message` fields |
| Exit codes | Scriptable error handling | 20+ enumerated codes: Phase 2 baseline + Phase 3 (60/61 MCP, 70/71/72/73/74/75 workspace/schema); documented in `contracts/exit-codes.md` and `contracts/exit-codes-p3.md`; Phase 4 F1–F11: adds 8 new codes (13–20 per FR-592) for harness + settings + summariser failures |
| Status checks | Per-subsystem health via `tome status` | Phase 8 — models, index, drift state with lazy `--verify` flag; Phase 3 / US2 — `tome workspace info` reports scope identity + counts |
| Doctor diagnostics | Subsystem health assessment + harness discovery + repair | Phase 3 / US4 — `tome doctor [--fix]` reports model/index/workspace/drift/harness health; Phase 3 Polish: orphan clone detection, registry status; Phase 4 F1–F11: extended to cover summariser state, harness MCP config state, settings composition diagnostics |

---

## File Storage

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| XDG-compliant filesystem | Configuration, catalogs, models, index, logs | Global: `${XDG_CONFIG_HOME}/tome/config.toml`, `${XDG_DATA_HOME}/tome/catalogs/<sha>/`, `${XDG_DATA_HOME}/tome/models/`, `${XDG_DATA_HOME}/tome/index.db`, `${XDG_STATE_HOME}/tome/mcp.log`; Workspace: `${WORKSPACE}/.tome/config.toml`, `${WORKSPACE}/.tome/catalogs/<sha>/`, `${WORKSPACE}/.tome/index.db` (Phase 3 F1); opt-in registry at `${XDG_DATA_HOME}/tome/workspaces.txt` (Phase 3 / US2); Phase 4 F1–F11: adds `src/settings/` composition framework for multi-level config resolution (project > workspace > global) |

---

## Email & Notifications

None in Phase 1–4 foundational.

---

## Agentic Coding Harness Integration (Phase 3 / US4, extended Phase 4 F1–F11)

Phase 3 / US4 adds harness discovery; Phase 4 Foundational extends to harness-specific MCP config integration and settings composition.

| Harness | Install Location | Discovery | Purpose | Phase 4 Additions |
|---------|------------------|-----------|---------|-------------------|
| Claude Code | `~/.claude` | Existence only | First-party harness | F7: `src/harness/` module with `HarnessModule` trait; F8: harness-specific MCP config read-modify-write via `toml_edit`; settings composition resolver consults harness config |
| Codex | `~/.codex` | Existence only | Third-party harness | Same as above |
| Cursor | `~/.cursor` | Existence only | Third-party harness | Same as above |
| Gemini CLI | `~/.gemini` | Existence only | Third-party harness | Same as above |
| OpenCode | `~/.opencode` | Existence only | Third-party harness | Same as above |
| Continue | `~/.continue` | Existence only | Third-party harness | Same as above |

**Discovery semantics (research §R-7, FR-167, Phase 4 R-9/R-11):**
- **Probe timing**: At startup, `doctor`, or harness commands; scans `$HOME` for each harness directory
- **Scope**: Fixed compile-time list — no dynamic discovery
- **Content read**: Phase 3 — existence only; Phase 4 F1–F11 — extends to harness-specific MCP config inspection (comment-preserving read via `toml_edit`)
- **Report shape**: `HarnessPresence { name, path, present: bool }` per contract; Phase 4: extended with optional `mcp_config_present: bool`
- **Update path**: Code change + contract update (not user-configurable)

---

## Settings Composition (Phase 4 F1–F11)

Phase 4 Foundational F8 + F10 introduces multi-level settings composition framework reused by both CLI and MCP server.

| Level | Location | Purpose | Precedence | Phase |
|-------|----------|---------|-----------|-------|
| **Project** | `.tome/RULES.md` (parsed + serialised to transient state) | Project-specific context + rules for summarisation | Highest | F1 (skeleton), US4 (real parsing) |
| **Workspace** | `${WORKSPACE}/.tome/settings.toml` (per-workspace, strict) | Workspace-local enablement, harness overrides, tool preferences | Medium | F8 |
| **Global** | `${XDG_CONFIG_HOME}/tome/settings.toml` (global, strict) | User-wide defaults, catalog list, model preferences | Lowest | F8 |

**Composition resolver** (`src/settings/resolver.rs`):
- Loads all applicable layers (project optional; workspace optional; global required)
- Merges in precedence order (project > workspace > global)
- Returns unified `ComposedSettings` struct
- Validation per layer (Tome-owned → strict `deny_unknown_fields`)
- Phase 4 F1–F11: composition logic completed; real tests pending Phase 4 / US3

**Harness-specific MCP config** (Phase 4 F8+):
- Location: `~/.harness/.mcp.json` (e.g., `~/.claude/.mcp.json`)
- Format: JSON array of tool descriptors per MCP spec
- Edit pattern: Tome reads, parses into struct, validates, modifies, writes back with comment preservation via `toml_edit` (even though JSON, the *principle* of order + comment preservation applies)
- Integration: Doctor reports harness MCP config state; settings composition resolver can inject Tome tools into harness config atomically

---

## Environment Variables

| Variable | Required | Purpose | Example | Updated Phase |
|----------|----------|---------|---------|---------------|
| `HOME` | Yes | Base directory for XDG path resolution | `/Users/aaronbassett` | — |
| `XDG_CONFIG_HOME` | No (defaults to `~/.config`) | Override config directory | `/opt/etc` | — |
| `XDG_DATA_HOME` | No (defaults to `~/.local/share`) | Override data directory (models, catalogs, index.db, workspaces.txt) | `/opt/var` | Phase 3 / US2 (workspaces.txt); Phase 4 / F1–F11 (settings composition) |
| `XDG_STATE_HOME` | No (defaults to `~/.local/state`) | Override state directory (MCP log) | `/opt/state` | Phase 3 Foundational F8 |
| `TOME_LOG` | No | Custom log filter (overrides `RUST_LOG`) | `debug`, `info`, `tome=trace` | — |
| `RUST_LOG` | No | Standard Rust log filter | `info`, `warn` | — |
| `NO_COLOR` | No | Disable coloured output (per CLICOLOR spec) | (presence enables) | Phase 3 Polish (consistent coverage); Phase 4 F1–F11 (maintained) |
| `CLICOLOR` | No | Disable coloured output (alternate) | `0` to disable | — |

---

## System Dependencies

### Required

- `git` (system binary) — for catalog cloning/updating/checkout (inherited from Phase 1 constitution principle XII)
- `libc` — standard C library (bundled with system)

### Optional

- **SSH keys** (`~/.ssh/`) — if catalogs use SSH URLs; inherits from git credential helper
- **Git credential helper** — if catalogs use HTTPS URLs without embedded credentials

### Not Required

- System OpenSSL (Tome uses `rustls` — statically linked)
- System SQLite (Tome uses `rusqlite bundled` — statically linked)
- ONNX Runtime shared library (Tome uses static `ort` via `fastembed` — bundled)
- `llama.cpp` shared library (Tome vendors + statically links via `llama-cpp-2`)

---

## Git Integration Details

| Aspect | Details |
|--------|---------|
| **Cloning** | `git clone <url> <path>` — full history by default |
| **Fetching** | `git fetch origin` — refreshes cached remote refs |
| **Checking out** | `git checkout <ref>` — pins catalog to specific commit/tag/branch |
| **Resetting** | `git reset --hard HEAD` — discards local changes on `tome catalog update` |
| **Credential flow** | SSH: SSH agent or `~/.ssh/id_*` keys; HTTPS: `git credential` helper or inline auth |
| **Signal handling** | SIGINT (Ctrl+C) kills child `git` process; exit code 8; reaps child via `std::process::wait()` |
| **Error scrubbing** | Captured stderr passed through `scrub_credentials()` before logging — covers URLs, tokens, SSH keys, long hex strings (principle XIII); Phase 3 Polish: extended to MCP log field scrubbing |

---

## Third-Party Manifest Parsing

| Format | Location | Strictness | Purpose |
|--------|----------|-----------|---------|
| `plugin.json` | Catalog plugin dirs | Lenient (unknown fields ignored) | Third-party plugin metadata (FR-013a boundary) |
| SKILL.md YAML frontmatter | Upstream plugin repos | Lenient (unknown fields ignored) | Third-party skill/agent/command/hook metadata |
| `tome-catalog.toml` | Catalog root | Strict (`deny_unknown_fields`) | Tome-owned manifest; rejects typos early |
| `config.toml` | Global: `${XDG_CONFIG_HOME}/tome/`; Workspace: `${WORKSPACE}/.tome/` (Phase 3 / US2) | Strict (`deny_unknown_fields`) | Tome-owned user config; Phase 4 F1–F11: extends to `settings.toml` for composition |
| `RULES.md` | `.tome/RULES.md` (Phase 4 / US4) | YAML frontmatter (lenient) + Markdown body | Project-specific context and rules for summarisation; parsed on-demand by summariser |

---

## MCP Server Integration (Phase 3 / US1, hardened Phase 3 Polish, extended Phase 4 F1–F11)

**Status:** Server loop + tool registration (Phase 3 / US1); Phase 4 / F1–F11 adds harness-specific config integration + extended error semantics.

| Aspect | Details |
|--------|---------|
| **Protocol** | `rmcp` (1.x) — Model Context Protocol stdio server per `contracts/mcp-server.md` |
| **Runtime** | Single-threaded `tokio` backing `src/mcp/` (Phase 3 Foundational F8); scoped via `tests/sync_boundary.rs` |
| **Process model** | Stdio: stdin = MCP messages, stdout = MCP responses; stderr for fatal startup errors only (FR-222); SIGTERM handler (Unix-only) with 5s graceful-shutdown timeout |
| **Tools advertised** | Two: `search_skills` (semantic KNN + optional reranking) and `get_skill` (retrieve skill detail by ID) |
| **Logging** | JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log`; 10 MiB rotation; Phase 3 Polish: custom `ContractEventFormat` for contract-pinned field names; log file 0600 (Unix-only); credential scrubbing on `workspace_path` and `error_message` |
| **Pre-flight** | FR-110 startup pipeline (schema check → drift detect → SHA-256 verify → eager-load FastembedEmbedder) scoped to `src/mcp/preflight.rs`; Phase 4 F1–F11: extended to harness MCP config validation, summariser placeholder check (exit 31 if all-zero checksum) |
| **Tool integration** | Embedder loaded once at startup; reranker lazily on first ranking call; Phase 4 F1–F11: summariser lazily on first project-context request (not yet wired in tools, but infrastructure ready) |
| **Tool I/O schemas** | `#[derive(JsonSchema)]` from `schemars` crate per `contracts/mcp-tools.md` |
| **Index access** | Read-only; Phase 3 Polish: symlink rejection hardening in skill walk |
| **Error handling** | Fatal startup errors (schema too new, drift, embedder load) → stderr + log + exit 60 (`McpStartupFailed`) or 61 (`McpProtocolIo`); Phase 4 F1–F11: adds 8 new exit codes (13–20) for harness + settings + summariser failures per FR-592; tool errors mapped to MCP error responses |
| **Sync boundary** | All async/tokio strictly in `src/mcp/`; structural test `tests/sync_boundary.rs` enforces |
| **CLI entry** | `tome mcp` — new `Command::Mcp(McpArgs)` dispatched before tracing/ctrlc init (FR-221) |
| **Phase 4 extensions** | Harness-specific MCP config integration via `src/harness/` module (F7); settings composition resolver in `src/settings/` (F8); project context loading from `.tome/RULES.md` (US4); summariser skeleton in `src/summarise/` (F6) |

### Tool Details

#### `search_skills`

| Aspect | Details |
|--------|---------|
| **Purpose** | Semantic skill search: KNN embedding distance + optional reranking |
| **Input** | `SearchSkillsInput { query, limit, force_strict, ... }` per `contracts/mcp-tools.md` |
| **Output** | `SearchSkillsOutput { skills, ... }` — each result includes ID, name, catalog, score, snippet |
| **Handler** | `pub async fn handle(input, state)` in `src/mcp/tools/search_skills.rs` |
| **Reuse** | Delegates to `commands::query::pipeline(args, deps)` — silent compute path |
| **Reranker** | Lazily loaded; shared across calls |

#### `get_skill`

| Aspect | Details |
|--------|---------|
| **Purpose** | Retrieve single skill full detail by ID |
| **Input** | `GetSkillInput { id: String }` — `<catalog>/<plugin>/<skill-name>` |
| **Output** | `GetSkillOutput { skill: Option<SkillDetail>, ... }` |
| **Handler** | `pub async fn handle(input, state)` in `src/mcp/tools/get_skill.rs` |
| **Query** | Read-only index lookup; Phase 3 Polish: symlink rejection hardening |

---

## Workspace Scope Integration (Phase 3 / US2–US3, extended Phase 4 F1–F11)

**Status:** Workspace info + init landed (Phase 3 / US2); scope-aware paths (Foundational F1); reference-counted catalog sharing (US3). Phase 4 / F1–F11: extends scope model with WorkspaceName + project binding + settings composition.

| Aspect | Details |
|--------|---------|
| **Scope types** | Global (default, uses XDG paths) or Workspace (per `.tome/` directory); resolved via `Paths::resolve()` which walks `cwd` up the tree looking for `.tome/` marker (Phase 3 / Foundational F1); Phase 4 / F1–F11: `Scope` becomes `WorkspaceName` newtype + `Scope(WorkspaceName)` tuple struct (F10) |
| **Path model** | Per-scope `Paths` accessor methods: `Paths::config_file_for(&Scope)`, etc. (Phase 3 Foundational F1); Phase 4 / F11: scope model simplified (deleted `Scope::Global` and `Scope::Workspace(PathBuf)` variants) |
| **Config location** | Global: `${XDG_CONFIG_HOME}/tome/config.toml`; Workspace: `${WORKSPACE}/.tome/config.toml` (parse errors → `WorkspaceMalformed` exit 70) |
| **Index location** | Global: `${XDG_DATA_HOME}/tome/index.db`; Workspace: `${WORKSPACE}/.tome/index.db` (same WAL + advisory lock model) |
| **Catalog cache location** | Global: `${XDG_DATA_HOME}/tome/catalogs/<sha>/`; Workspace: `${WORKSPACE}/.tome/catalogs/<sha>/`; Phase 4 / F11: enrolment moved to `workspace_catalogs` junction table (sole source of truth per FR-360); source of truth no longer in `Config.catalogs` |
| **Reference counting (Phase 3 / US3)** | `catalog::store::reference_count(url, paths) -> Vec<Scope>` enumerates scopes that reference a URL; Phase 4 / F11: extended to junction-table query via `src/index/workspace_catalogs.rs` (302 LOC) |
| **Info command** | `tome workspace info` (Phase 3 / US2.a) — read-only scope report; Phase 4 / F1–F11: no new changes to info output |
| **Init command** | `tome workspace init [<path>] [--inherit-global] [--force]` (Phase 3 / US2.b) — atomic `.tome/` creation; Phase 4 / F1–F11: no new changes to init semantics; RULES.md creation happens on first bind (Phase 4 / US1) |
| **Registry file** | `${XDG_DATA_HOME}/tome/workspaces.txt` — opt-in; Phase 3 / US3 makes it load-bearing for refcount enumeration; Phase 4 / F1–F11: continues same semantics |
| **CLI wiring** | `Command::Workspace(WorkspaceArgs)` + `WorkspaceCommand::{Info, Init}` (Phase 3 / US2); Phase 4 / F1–F11: scope resolution integrated into all commands via `Paths::resolve()` |

---

## Schema Migration Integration (Phase 3 / US5, extended Phase 4 F1–F11)

**Status:** Forward-migration framework (Phase 3 Foundational F7); integration test coverage (Phase 3 / US5). Phase 4 / F1–F11: extends schema to v2 with new `workspace_catalogs` and `workspace_projects` tables.

| Aspect | Details |
|--------|---------|
| **Framework** | `src/index/migrations.rs` — `Migration` struct with function-pointer apply hooks; `apply_pending(conn, current, target)` three-arg signature; `MIGRATIONS_OVERRIDE` test-injection point |
| **Schema versions** | v0 (Phase 2 bootstrap), v1 (Phase 3 baseline), v2 (Phase 4 / F1 introduces new tables for workspace + project binding) |
| **Test coverage** | `tests/schema_migration_e2e.rs` — integration tests via synthetic-fixture injection; Phase 4 / F1–F11: adds tests for v1→v2 migration once first real migration is registered |
| **Test fixtures** | `tests/common/mod.rs::write_index_db_with_schema_version` helper fabricates old-version DBs |
| **Atomicity** | All migrations run under advisory lock; rollback on error; no partial state visible to readers |
| **Version semantics** | Write-path checks schema version, emits `SchemaVersionTooNew` (exit 73) if too new; read-path retains legacy `SchemaTooNew` (exit 52) for backward compat |
| **Production migrations** | Compile-time `MIGRATIONS` array (empty in Phase 3, first real migration in Phase 4 / F1 with v1→v2 schema expansion) |
| **Doctor integration** | `tome doctor` can repair schema via `--fix`; Phase 4 / F1–F11: extended to validate workspace_catalogs junction table consistency |

---

## Index Schema Changes (Phase 4 / F1–F11)

Phase 4 / F1 introduces schema v2 with structural-only changes (no data migration needed, new tables are optional until Phase 4 / US1 wires project binding).

### New Tables (v2)

| Table | Purpose | Load-bearing Phase |
|-------|---------|-------------------|
| `workspace_catalogs` | Junction table: workspace scopes × catalog URLs; replaces `Config.catalogs` as sole source of truth per FR-360 | F11 (moved enrolment to table) |
| `workspace_projects` | 1:1 binding: workspace → project directory; primary key on `project_path` alone (FR-598) | US1 (first real usage when binding a project) |

### Primary Key Changes

- `workspace_projects.project_path`: Unique constraint (1:1 binding to one workspace)
- `workspace_catalogs`: Composite key on `(workspace_id, catalog_url)` for uniqueness across scopes

---

## What Does NOT Belong Here

- Internal code architecture → ARCHITECTURE.md
- Testing infrastructure → TESTING.md
- Security policies → SECURITY.md
- Dependency versions → STACK.md

---

*This document maps external service dependencies and integration points in Tome at Phase 4 Foundational F1–F11 complete. 490 tests pass across 64 suites. Phase 4 adds harness-specific MCP config integration, multi-level settings composition, project binding infrastructure, and schema v2 with workspace-scoped catalog enrolment. Binary size projection remains ~28–34 MB, well under the 50 MB cap.*

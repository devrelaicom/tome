# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-05-25
> **Last Updated**: 2026-05-25 (Phase 4 Foundational F1–F11 + US1 complete; 490+ tests across 64+ suites; v0.3.0 + US1 additions)

## Databases & Data Stores

### Local SQLite Index

| Service | Type | Purpose | Location |
|---------|------|---------|----------|
| SQLite 3 | Embedded relational DB | Local skill index — metadata, embeddings, reranker scores, workspace bindings | Global: `${XDG_DATA_HOME}/tome/index.db` (WAL mode); Workspace: `${WORKSPACE}/.tome/index.db` (Phase 3 Foundational F1); schema in `src/index/schema.rs` |

### Connection Patterns

- **Statically linked**: `rusqlite` with `bundled` feature — no system SQLite dependency.
- **Concurrency model**: Single advisory lockfile (`index.lock` — global or workspace-scoped) serialises writes; WAL mode allows readers during writes; MCP server uses read-only open per FR-056; Phase 3 Polish: validators gate entry paths (malformed config / unopenable index → `WorkspaceMalformed` exit 70); Phase 4 US1: binding uses advisory lock for atomic workspace_projects UPSERT + marker landing.
- **ORM/Query builder**: Direct SQL via `rusqlite` — prepared statements, parameterised queries.
- **Migration approach**: Forward-only migrations under advisory lock in `src/index/migrations.rs`; Phase 4 F1 introduces schema v2 with `workspace_catalogs` (F11 live) + `workspace_projects` (US1 live) tables; drift detection in `src/index/meta.rs`.

### Cache Structure

- **Catalog cache**: Each remote catalog source content-addressed by `sha256(url)` in `${XDG_DATA_HOME}/tome/catalogs/<sha256>/` (global) or `${WORKSPACE}/.tome/catalogs/<sha256>/` (workspace) — Git working tree, refreshed on `tome catalog update`. Multiple scopes can reference the same URL; shared via reference-count tracking — deleted only when no scope references it (Phase 3 / US3); Phase 4 F11: enrolment moved to `workspace_catalogs` junction table (sole source of truth per FR-360).
- **Model cache**: Downloaded model ONNX artefacts stored in `${XDG_DATA_HOME}/tome/models/<model-name>/` (global, shared across scopes) with per-model `manifest.json` (strict JSON, `#[serde(deny_unknown_fields)]`); Phase 6 adds explicit `tome models {download,list,remove}` commands; Phase 8 adds read-only audit via `tome status [--verify]`; Phase 4 F1: summariser model (Qwen2.5-0.5B-Instruct GGUF) added to registry alongside embedder/reranker.
- **Atomic writes**: `tempfile` crate (rename-based) prevents corruption on SIGINT; workspace `init` uses `tempfile::Builder::tempdir_in(workspace_root)` for POSIX-atomic staging-to-final rename (Phase 3 / US2); Phase 4 US1: project binding uses same atomic-dir pattern for `.tome/` marker; Phase 4 F4: `src/util/atomic_dir.rs` promoted as reusable helper.

### Workspace Registry (Phase 3 / US2, load-bearing in Phase 3 / US3)

- **File**: `${XDG_DATA_HOME}/tome/workspaces.txt` — opt-in (never created unless explicitly requested)
- **Format**: Line-delimited absolute paths to workspace roots; dedupe by exact-path match and canonicalize
- **Size cap**: 1 MiB; entry cap 10k (Phase 3 Polish hardening); no NUL or `..` path traversal sequences
- **Semantics**: Informational in US2; load-bearing in US3 — tracks which workspaces have been initialized via `--inherit-global`. US3 `catalog remove` consults this file to enumerate all scopes for reference-counting.
- **Usage**: Client harnesses can read this file to discover initialized workspaces; Tome treats absence as "no workspace scopes" (global scope only); Phase 4 US1: unused by binding algorithm (central DB is source of truth for workspace_projects)

---

## Authentication & Authorization

Phase 1–4 has no explicit application-layer authentication. Phase 3 / US1 MCP server is stdio-based (embedding in harness provides transport-level security). Phase 4 / F1–F11 + US1 maintains the same posture.

- **Git operations**: Inherit system SSH keys and HTTP credential helpers (if configured in `~/.gitconfig`).
- **Hugging Face model downloads**: No API key required; public HTTPS URLs freely accessible (MODEL_REGISTRY pinned to MIT-licensed BGE variants + Apache-2.0 Qwen2.5).
- **Plugin manifest ownership**: File system permissions validate catalog ownership (email field in `tome-catalog.toml` is metadata only).
- **Workspace ownership**: Implicitly owned by the user who runs `tome workspace init`; no explicit permission model.
- **Project binding ownership**: Implicitly owned by the user who runs `tome workspace use`; binding record stored in central DB + `.tome/` marker created in project root with restricted permissions (no explicit ACL).
- **Credential scrubbing**: All Git stderr and model download error chains pass through `scrub_credentials()` before logging (principle XIII; extended to HF URLs and MCP log fields; Phase 4 US1: harness sync paths included in scrubbing).
- **MCP server identity** (Phase 3 / US1): Identified by `server_info { name: "tome", version: "0.x" }` in the MCP handshake; no per-call authentication.
- **Doctor read-only access** (Phase 3 / US4): Diagnostics are read-only; repairs (`--fix`) require interactive confirmation.
- **Harness config access** (Phase 4 US1): Direct filesystem access to harness-owned `.mcp.json` / `.mcp.toml` files; no permission model beyond OS-level file permissions.

---

## External APIs

### First-Party APIs

- `commands::query::pipeline(args, deps) -> Result<QueryOutcome, TomeError>` — silent compute path reused by MCP `search_skills` tool (Phase 3 / US1.b)
- `workspace::binding::bind_project(project_root, workspace_name, deps)` — project-to-workspace binding orchestrator (Phase 4 US1.a)
- `harness::sync::sync_for_project_root(project_root, scope, deps)` — harness MCP config + rules-file syncer (Phase 4 US1.b — skeleton; full wiring pending)
- Phase 4 F1–F11 + US1 continues to reuse library-level APIs without new external surfaces

### Third-Party APIs

#### Hugging Face Model Registry

| Provider | Purpose | SDK/Client | Configuration |
|----------|---------|------------|---------------|
| Hugging Face (`huggingface.co`) | ONNX + GGUF model downloads (embedder, reranker, summariser) | `reqwest::blocking` (direct HTTPS) | `src/embedding/registry.rs` — `MODEL_REGISTRY` (compile-time constants) |

**Details**:
- **Embedder**: `bge-small-en-v1.5` INT8 (~66 MB) from quantised variant
- **Reranker**: `bge-reranker-base` INT8 (~280 MB) from `onnx-community/bge-reranker-base-ONNX` (source moved Phase 3 slice 1)
- **Summariser** (Phase 4 F1+): `qwen2.5-0.5b-instruct` GGUF (~400 MB placeholder, real digest in US4) from `Qwen/Qwen2.5-0.5B-Instruct-GGUF`; Phase 4 F6 adds placeholder with all-zero checksum guard (downloads refused until real digest landed in US4); Phase 4 US1: summariser model infrastructure complete but not actively used (pending US4.a wiring)
- **Integrity**: Pinned SHA-256 + size_bytes verified post-download; no checksum endpoint (hashes are real upstream digests verified at Phase 3 slice 1 start)
- **Network**: HTTPS only via `rustls-tls` (no system OpenSSL)
- **Failure modes**: Network error → `TomeError::Io` (exit 7); checksum mismatch → `TomeError::ModelChecksumMismatch` (exit 32); corrupted registry → `TomeError::ModelCorrupt` (exit 31); missing model → `TomeError::ModelMissing` (exit 30); embedder drift → `TomeError::EmbedderNameDrift` (exit 41); summariser placeholder → `TomeError::ModelCorrupt` (exit 31); Phase 4 US1: adds harness-specific failure codes (13–20 per FR-592) for harness module errors
- **Explicit management**: Phase 6 wires `tome models {download,list,remove}` to manage artefacts; `tome models list --verify` validates SHA-256 per-file via `embedding::download::sha256_file()`
- **Status visibility**: Phase 8 adds `tome status [--verify]` for read-only audit without triggering downloads
- **Doctor integration** (Phase 3 / US4): `tome doctor` reports model health with optional repair via `--fix`; Phase 3 Polish: specific exit codes for name mismatch vs missing; Phase 4 US1: extended to summariser placeholder check
- **Scope**: Models are global (shared across all workspaces); downloaded to `${XDG_DATA_HOME}/tome/models/` regardless of active scope

---

## Message Queues & Event Systems

None. Phase 3 / US1 MCP server is stdio-based (single request/response); Phase 4 F1–F11 + US1 adds no async event infrastructure. Phase 3 Polish: explicit SIGTERM handler for graceful shutdown (Unix-only) with 5s timeout.

---

## Caching

| Service | Purpose | TTL / Eviction | Configuration |
|---------|---------|----------------|-----------------|
| Filesystem (XDG) | Catalog Git working trees | Explicit `tome catalog remove` (user-managed); persistent; shared across scopes via refcount (Phase 3 / US3) | Global: `${XDG_DATA_HOME}/tome/catalogs/`; Workspace: `${WORKSPACE}/.tome/catalogs/` (Phase 3 Foundational F1); same URL reused — clone deleted only when all scopes drop it; Phase 3 Polish: orphan clones reported by doctor |
| Filesystem (XDG) | Downloaded model artefacts | Explicit `tome models remove` (user-managed); persistent | `${XDG_DATA_HOME}/tome/models/` — one dir per model with manifest + ONNX/GGUF files; shared across all scopes (global); Phase 3 / US4 doctor can remove corrupt models |
| Filesystem (Project) | Cached summaries | Explicit `tome workspace regen-summary` (user-managed, pending US4); persistent | `.tome/settings.toml` — `[summaries]` table with short + long + generated_at timestamp; regenerated on enable / disable / reindex / explicit regen triggers (Phase 4 US4 wiring) |

No TTL-based eviction. Explicit user commands for cleanup (principle VI). Phase 3 Polish: doctor provides advisory cleanup candidates.

---

## Monitoring & Observability

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| Structured logging (via `tracing`) | Diagnostic tracing to stderr (CLI) and JSON-lines to file (MCP server) | CLI: `RUST_LOG` or `TOME_LOG` environment variables; independent of `--json` stdout. MCP: JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log` per `contracts/log-format.md`; 10 MiB rotation cap; stderr reserved for fatal startup errors only (FR-222); Phase 3 Polish: custom `ContractEventFormat` emits contract-pinned field names (`ts`, `level`, `target`, `msg`); log file 0600 mode (Unix-only); credential scrubbing on `workspace_path` and `error_message` fields; Phase 4 US1: harness sync operations included in scrubbing |
| Exit codes | Scriptable error handling | 28+ enumerated codes: Phase 2 baseline + Phase 3 additions + Phase 4 F1–F11 (13–20 per FR-592 for harness/settings/summariser); documented in `contracts/exit-codes.md` and incremental updates |
| Status checks | Per-subsystem health via `tome status` | Phase 8 — models, index, drift state with lazy `--verify` flag; Phase 4 F1–F11: extended to summariser state, harness MCP config state; Phase 4 US1: project binding status (central DB presence + marker validity) |
| Doctor diagnostics | Subsystem health assessment + harness discovery + repair | Phase 3 / US4 — `tome doctor [--fix]` reports model/index/workspace/drift/harness health; Phase 3 Polish: orphan clone detection, registry status; Phase 4 F1–F11: extended to summariser state, harness config state, settings composition; Phase 4 US1: project binding state (orphaned binding markers, mismatched DB/filesystem) |

---

## File Storage

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| XDG-compliant filesystem | Configuration, catalogs, models, index, logs | Global: `${XDG_CONFIG_HOME}/tome/settings.toml` (Phase 4 F8+), `${XDG_DATA_HOME}/tome/catalogs/<sha>/`, `${XDG_DATA_HOME}/tome/models/`, `${XDG_DATA_HOME}/tome/index.db`, `${XDG_STATE_HOME}/tome/mcp.log`; Workspace: `${WORKSPACE}/.tome/config.toml`, `${WORKSPACE}/.tome/settings.toml` (Phase 4 F8+), `${WORKSPACE}/.tome/catalogs/<sha>/`, `${WORKSPACE}/.tome/index.db`; Project: `${PROJECT}/.tome/config.toml` (Phase 4 US1), `${PROJECT}/.tome/RULES.md` (Phase 4 US4, skeleton from binding); opt-in registry at `${XDG_DATA_HOME}/tome/workspaces.txt` (Phase 3 / US2) |

---

## Email & Notifications

None in Phase 1–4.

---

## Agentic Coding Harness Integration (Phase 3 / US4, extended Phase 4 F1–F11 + US1)

Phase 3 / US4 adds harness discovery; Phase 4 Foundational extends to harness-specific MCP config integration and settings composition; Phase 4 US1 adds project binding + rules-file + MCP config sync.

| Harness | Install Location | Discovery | Purpose | Phase 4 Additions |
|---------|------------------|-----------|---------|-------------------|
| Claude Code | `~/.claude` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection | First-party harness | F7: `src/harness/claude_code.rs` impl; F8: MCP config read-modify-write; US1: sync algorithm wired (rules-file + MCP config update) |
| Codex | `~/.codex` | Existence only → Phase 4 F1+ extends to `.mcp.toml` inspection | Third-party harness | F7: `src/harness/codex.rs` impl; F8: TOML-specific read-modify-write via `toml_edit`; US1: sync algorithm wired |
| Cursor | `~/.cursor` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection | Third-party harness | F7: `src/harness/cursor.rs` impl; F8: JSON + standalone rules-file support; US1: sync algorithm wired |
| Gemini CLI | `~/.gemini` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection | Third-party harness | F7: `src/harness/gemini.rs` impl; F8: MCP config + block rules-file; US1: sync algorithm wired |
| OpenCode | `~/.opencode` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection | Third-party harness | F7: `src/harness/opencode.rs` impl; F8: MCP config + block rules-file; US1: sync algorithm wired |

**Discovery semantics (research §R-7, FR-167, Phase 4 R-9/R-11):**
- **Probe timing**: At startup, `doctor`, or harness commands; scans `$HOME` for each harness directory; Phase 4 US1: also called during binding workflow to sync harness config
- **Scope**: Fixed compile-time list in `src/harness/mod.rs::SUPPORTED_HARNESSES` — no dynamic discovery
- **Content read**: Phase 3 — existence only; Phase 4 F1–F11 — extends to harness-specific MCP config inspection (comment-preserving read via `toml_edit` for `Codex`, `serde_json` for others); Phase 4 US1: harness sync reads MCP config, validates, modifies, writes back atomically
- **Report shape**: `HarnessPresence { name, path, present: bool }` per contract; Phase 4: extended with optional `mcp_config_present: bool`; Phase 4 US1: binding outcome includes per-harness sync result
- **Update path**: Harness module trait dispatch (`HarnessModule` impl per harness); code change + contract update (not user-configurable)

**Harness module architecture (Phase 4 F7+):**
- **Trait**: `HarnessModule` — defines home dir, MCP config format (JSON / TOML), rules-file strategy (Block / Standalone), parent key, read/write/delete operations
- **Registry**: `SUPPORTED_HARNESSES: &[&dyn HarnessModule]` in `mod.rs`; lookup by name via `harness::lookup(&str) -> Option<&'dyn HarnessModule>`
- **Implementations**: Five concrete impls (`claude_code`, `codex`, `cursor`, `gemini`, `opencode`); each pins format + path decisions per contract + upstream harness docs
- **Test injection**: `HARNESS_MODULES_OVERRIDE` thread-local for test-injecting `StubHarness` (Phase 4 US1+ test discipline)
- **Sync algorithm**: `src/harness/sync.rs::sync_for_project_root(project_root, scope, deps) -> SyncOutcome` orchestrates read/modify/write per harness (Phase 4 US1.b skeleton)
- **Rules file strategy**: `BlockInExistingFile` (Claude Code, Codex, Gemini, OpenCode) or `StandaloneFile` (Cursor); implemented in `src/harness/rules_file.rs`
- **MCP config strategy**: JSON (Claude Code, Cursor, Gemini, OpenCode) or TOML (Codex); implemented in `src/harness/mcp_config.rs`

---

## Settings Composition (Phase 4 F1–F11 + US1)

Phase 4 Foundational F8 introduces multi-level settings composition framework reused by both CLI and MCP server. Phase 4 US1 extends with project-level config.

| Level | Location | Purpose | Precedence | Phase |
|-------|----------|---------|-----------|-------|
| **Project** | `${PROJECT}/.tome/config.toml` (strict, Tome-owned) | Project-specific settings: harness overrides, tool preferences, project binding identity | Highest | F1 (skeleton), US1 (binding record) |
| **Project (alt)** | `.tome/RULES.md` (lenient frontmatter + Markdown body) | Project-specific context + rules for summarisation; parsed on-demand by summariser | — | F1 (skeleton), US4 (real parsing) |
| **Workspace** | `${WORKSPACE}/.tome/settings.toml` (strict, Tome-owned) | Workspace-local enablement, harness overrides, tool preferences | Medium | F8 |
| **Global** | `${XDG_CONFIG_HOME}/tome/settings.toml` (strict, Tome-owned) | User-wide defaults, catalog list, model preferences | Lowest | F8 |

**Composition resolver** (`src/settings/resolver.rs`):
- Loads all applicable layers (project optional; workspace optional; global required)
- Merges in precedence order (project > workspace > global)
- Returns unified `ComposedSettings` struct
- Validation per layer (Tome-owned → strict `deny_unknown_fields`)
- Phase 4 F1–F11: composition logic completed; Phase 4 US3: resolver fully wired into CLI + MCP
- Phase 4 US1: project config loaded during binding to validate project-level harness list

**Harness-specific MCP config** (Phase 4 F8+, live in US1):
- Location: `~/.harness/.mcp.json` or `.mcp.toml` (e.g., `~/.claude/.mcp.json`, `~/.codex/.mcp.toml`)
- Format: JSON array of tool descriptors (most harnesses) or TOML table (Codex) per MCP spec + harness-specific convention
- Edit pattern: Tome reads, parses into struct, validates, modifies, writes back with comment/order preservation (Phase 4 F8 via `toml_edit` + `serde_json`); Phase 4 US1: sync algorithm calls harness module to update MCP config atomically
- Integration: Doctor reports harness MCP config state; settings composition resolver can query harness config; Phase 4 US1: binding sync populates harness MCP config with Tome tools + project context
- **Project-level harness overrides** (Phase 4 US1): `${PROJECT}/.tome/config.toml` can declare harness-specific settings that override workspace/global for that project's context

---

## Project Binding Integration (Phase 4 US1)

Phase 4 / US1 introduces `tome workspace use` — one-way binding from a project directory to a workspace, enabling project-scoped skill indexing and summarisation.

| Aspect | Details |
|--------|---------|
| **Binding semantics** | User runs `tome workspace use <workspace-name>` from a project directory; Tome records the binding in the central DB (`workspace_projects` table, PK on project_path) and creates an atomic `${PROJECT}/.tome/` marker directory |
| **Storage** | Central: `workspace_projects` table in `${XDG_DATA_HOME}/tome/index.db` (1:1 mapping project_path → workspace_id); Project-local: `${PROJECT}/.tome/config.toml` (contains workspace name for verification) |
| **Atomicity** | `bind_project` acquires advisory lock, UPSERTs DB row, lands marker dir via `tempfile::Builder::tempdir_in + rename`, releases lock. Failure between UPSERT + marker creates orphan (recoverable by re-running same bind command). |
| **Discovery** | Doctor scans for orphaned markers (DB row absent, filesystem present); orphaned markers are advisory — can be manually removed or recovered via re-bind |
| **Scope inference** | When a project is bound, `Paths::resolve()` can return the project's workspace scope if the marker is present + DB record matches. CWD walk sequence: cwd → ancestors → found `.tome/` marker → verify binding in DB → return `Scope(workspace_name)` |
| **CLI entry** | `tome workspace use [<workspace-name>] [--workspace <override>]` — new `WorkspaceCommand::Use` (Phase 4 US1.a); interactive selection if no workspace-name given |
| **Harness sync** | Phase 4 US1.b: `commands::harness::sync_for_project_root(project_root)` called post-binding to sync harness MCP config + rules-file; currently skeleton (returns stub outcome), full wiring pending |
| **Failure modes** | Non-existent workspace → error; project already bound to different workspace → confirm + rebind; CWD not a project dir (no .git / pyproject.toml / etc.) → error; binding record stale (workspace deleted, marker orphaned) → doctor repair or manual cleanup |

---

## Workspace Scope Integration (Phase 3 / US2–US3, extended Phase 4 F1–F11 + US1)

**Status:** Workspace info + init landed (Phase 3 / US2); scope-aware paths (Foundational F1); reference-counted catalog sharing (US3); project binding (US1). Phase 4 / F1–F11 + US1: extends scope model with WorkspaceName + project binding + settings composition.

| Aspect | Details |
|--------|---------|
| **Scope types** | Global (default, uses XDG paths) or Workspace (per `.tome/` directory); resolved via `Paths::resolve()` which walks `cwd` up the tree looking for `.tome/` marker; Phase 4 F1–F11: `Scope` becomes `WorkspaceName` newtype + `Scope(WorkspaceName)` tuple struct (F10); Phase 4 US1: extended with project marker detection + binding verification |
| **Path model** | Per-scope `Paths` accessor methods: `Paths::config_file_for(&Scope)`, etc. (Phase 3 Foundational F1); Phase 4 F11: scope model simplified (deleted enum variants, all scopes now use WorkspaceName); Phase 4 US1: project paths resolved via binding lookup when available |
| **Config location** | Global: `${XDG_CONFIG_HOME}/tome/settings.toml` (Phase 4 F8+); Workspace: `${WORKSPACE}/.tome/settings.toml` (Phase 4 F8+); Project: `${PROJECT}/.tome/config.toml` (Phase 4 US1, binding marker) |
| **Index location** | Global: `${XDG_DATA_HOME}/tome/index.db`; Workspace: `${WORKSPACE}/.tome/index.db` (same WAL + advisory lock model); Phase 4 US1: project-scoped queries use workspace-scoped index (the workspace that owns the binding) |
| **Catalog cache location** | Global: `${XDG_DATA_HOME}/tome/catalogs/<sha>/`; Workspace: `${WORKSPACE}/.tome/catalogs/<sha>/`; Phase 4 F11: enrolment moved to `workspace_catalogs` junction table (sole source of truth); Phase 4 US1: project inherits workspace's catalogs (no project-local catalog list) |
| **Reference counting (Phase 3 / US3)** | `catalog::store::reference_count(url, paths) -> Vec<Scope>` enumerates scopes that reference a URL; Phase 4 F11: extended to junction-table query via `src/index/workspace_catalogs.rs`; Phase 4 US1: unchanged |
| **Info command** | `tome workspace info` (Phase 3 / US2.a) — read-only scope report; Phase 4 F1–F11 + US1: no new changes to info output |
| **Init command** | `tome workspace init [<path>] [--inherit-global] [--force]` (Phase 3 / US2.b) — atomic `.tome/` creation; Phase 4 F1–F11 + US1: no new changes to init semantics |
| **Use command** | `tome workspace use [<workspace-name>]` (Phase 4 US1) — bind project to workspace with atomic marker + DB record; harness sync follows |
| **Registry file** | `${XDG_DATA_HOME}/tome/workspaces.txt` — opt-in; Phase 3 / US3 makes it load-bearing for refcount enumeration; Phase 4 F1–F11 + US1: continues same semantics (unused by binding algorithm) |
| **CLI wiring** | `Command::Workspace(WorkspaceArgs)` + `WorkspaceCommand::{Info, Init, Use}` (Phase 4 US1 adds `Use`); scope resolution integrated into all commands via `Paths::resolve()` |

---

## Schema Migration Integration (Phase 3 / US5, extended Phase 4 F1–F11)

**Status:** Forward-migration framework (Phase 3 Foundational F7); integration test coverage (Phase 3 / US5); v2 schema (Phase 4 F1+). Phase 4 / F1–F11 + US1: extends schema with project binding table.

| Aspect | Details |
|--------|---------|
| **Framework** | `src/index/migrations.rs` — `Migration` struct with function-pointer apply hooks; `apply_pending(conn, current, target)` three-arg signature; `MIGRATIONS_OVERRIDE` test-injection point |
| **Schema versions** | v0 (Phase 2 bootstrap), v1 (Phase 3 baseline), v2 (Phase 4 / F1 introduces `workspace_catalogs` + `workspace_projects` tables, structural-only, no data migration) |
| **Test coverage** | `tests/schema_migration_e2e.rs` — integration tests via synthetic-fixture injection; Phase 4 F1–F11 + US1: v1→v2 migration passes (tables are structural-only, safe to create empty) |
| **Test fixtures** | `tests/common/mod.rs::write_index_db_with_schema_version` helper fabricates old-version DBs |
| **Atomicity** | All migrations run under advisory lock; rollback on error; no partial state visible to readers |
| **Version semantics** | Write-path checks schema version, emits `SchemaVersionTooNew` (exit 73) if too new; read-path retains legacy `SchemaTooNew` (exit 52) for backward compat |
| **Production migrations** | Compile-time `MIGRATIONS` array (Phase 4 F1: v1→v2 structural-only migration registered, adds `workspace_catalogs` + `workspace_projects` tables) |
| **Doctor integration** | `tome doctor` can repair schema via `--fix`; Phase 4 F1–F11: extended to validate workspace_catalogs junction table + workspace_projects binding consistency |

---

## Index Schema Changes (Phase 4 / F1–F11 + US1)

Phase 4 / F1 introduces schema v2 with structural-only changes (no data migration needed, new tables are optional until load-bearing phases).

### New Tables (v2)

| Table | Purpose | Load-bearing Phase | Phase 4 Additions in US1 |
|-------|---------|-------------------|------------------------|
| `workspace_catalogs` | Junction table: workspace scopes × catalog URLs; replaces `Config.catalogs` as sole source of truth per FR-360 | F11 (moved enrolment to table) | Unchanged |
| `workspace_projects` | 1:1 binding: project_path → workspace_id; primary key on `project_path` alone (FR-598) | US1 (first real usage when binding a project) | Added in v1→v2 migration; populated by `bind_project` |

### Primary Key Changes

- `workspace_projects.project_path`: Unique constraint (1:1 binding to one workspace)
- `workspace_catalogs`: Composite key on `(workspace_id, catalog_url)` for uniqueness across scopes

---

## Environment Variables

| Variable | Required | Purpose | Example | Updated Phase |
|----------|----------|---------|---------|---------------|
| `HOME` | Yes | Base directory for XDG path resolution and harness home probe | `/Users/aaronbassett` | — |
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
| **Error scrubbing** | Captured stderr passed through `scrub_credentials()` before logging — covers URLs, tokens, SSH keys, long hex strings (principle XIII); Phase 3 Polish: extended to MCP log field scrubbing; Phase 4 US1: extended to harness sync error paths |

---

## Third-Party Manifest Parsing

| Format | Location | Strictness | Purpose |
|--------|----------|-----------|---------|
| `plugin.json` | Catalog plugin dirs | Lenient (unknown fields ignored) | Third-party plugin metadata (FR-013a boundary) |
| SKILL.md YAML frontmatter | Upstream plugin repos | Lenient (unknown fields ignored) | Third-party skill/agent/command/hook metadata |
| `tome-catalog.toml` | Catalog root | Strict (`deny_unknown_fields`) | Tome-owned manifest; rejects typos early |
| `.tome/config.toml` (workspace) | `${WORKSPACE}/.tome/` | Strict (`deny_unknown_fields`) | Workspace marker identity; created on init |
| `.tome/config.toml` (project) | `${PROJECT}/.tome/` | Strict (`deny_unknown_fields`) | Project binding identity; created on bind (Phase 4 US1) |
| `settings.toml` (workspace) | `${WORKSPACE}/.tome/settings.toml` | Strict (`deny_unknown_fields`) | Workspace-level settings; Phase 4 F8+ |
| `settings.toml` (global) | `${XDG_CONFIG_HOME}/tome/` | Strict (`deny_unknown_fields`) | User-wide settings; Phase 4 F8+ |
| `.tome/RULES.md` frontmatter + body | Project root (Phase 4 US4) | YAML frontmatter (lenient) + Markdown body | Project context + rules for summarisation; auto-created on first bind; Phase 4 US1: skeleton landing |

---

## MCP Server Integration (Phase 3 / US1, hardened Phase 3 Polish, extended Phase 4 F1–F11 + US1)

**Status:** Server loop + tool registration (Phase 3 / US1); Phase 4 / F1–F11 adds harness-specific config integration + extended error semantics; Phase 4 US1: project binding infrastructure complete.

| Aspect | Details |
|--------|---------|
| **Protocol** | `rmcp` (1.x) — Model Context Protocol stdio server per `contracts/mcp-server.md` |
| **Runtime** | Single-threaded `tokio` backing `src/mcp/` (Phase 3 Foundational F8); scoped via `tests/sync_boundary.rs` |
| **Process model** | Stdio: stdin = MCP messages, stdout = MCP responses; stderr for fatal startup errors only (FR-222); SIGTERM handler (Unix-only) with 5s graceful-shutdown timeout |
| **Tools advertised** | Two: `search_skills` (semantic KNN + optional reranking) and `get_skill` (retrieve skill detail by ID); Phase 4 US4: pending third tool for project context / summaries |
| **Logging** | JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log`; 10 MiB rotation; Phase 3 Polish: custom `ContractEventFormat` for contract-pinned field names; log file 0600 (Unix-only); credential scrubbing on `workspace_path` and `error_message` fields; Phase 4 US1: binding operations included in scrubbing |
| **Pre-flight** | FR-110 startup pipeline (schema check → drift detect → SHA-256 verify → eager-load FastembedEmbedder) scoped to `src/mcp/preflight.rs`; Phase 4 F1–F11 + US1: extended to harness MCP config validation, project binding check, summariser placeholder check (exit 31 if all-zero checksum) |
| **Tool integration** | Embedder loaded once at startup; reranker lazily on first ranking call; Phase 4 F1–F11 + US1: summariser lazily on first project-context request (not yet wired in tools, but infrastructure ready); project scope inferred from binding if present, else global |
| **Tool I/O schemas** | `#[derive(JsonSchema)]` from `schemars` crate per `contracts/mcp-tools.md` |
| **Index access** | Read-only; Phase 3 Polish: symlink rejection hardening in skill walk; Phase 4 US1: project-scoped skill search uses workspace-scoped index if binding present |
| **Error handling** | Fatal startup errors (schema too new, drift, embedder load) → stderr + log + exit 60 (`McpStartupFailed`) or 61 (`McpProtocolIo`); Phase 4 F1–F11 + US1: adds 8 new exit codes (13–20) for harness + settings + summariser failures per FR-592; tool errors mapped to MCP error responses |
| **Sync boundary** | All async/tokio strictly in `src/mcp/`; structural test `tests/sync_boundary.rs` enforces; Phase 4 US1: harness sync runs outside MCP (CLI-only in binding phase) |
| **CLI entry** | `tome mcp` — new `Command::Mcp(McpArgs)` dispatched before tracing/ctrlc init (FR-221); Phase 4 US1: no new MCP entry points (binding is CLI-only) |
| **Phase 4 extensions** | Harness-specific MCP config integration via `src/harness/` module (F7); settings composition resolver in `src/settings/` (F8); project binding infrastructure in `src/workspace/binding.rs` + `src/index/workspace_projects` (US1); summariser skeleton in `src/summarise/` (F6); project context loading from `.tome/RULES.md` (US4) |

### Tool Details

#### `search_skills`

| Aspect | Details |
|--------|---------|
| **Purpose** | Semantic skill search: KNN embedding distance + optional reranking |
| **Input** | `SearchSkillsInput { query, limit, force_strict, ... }` per `contracts/mcp-tools.md` |
| **Output** | `SearchSkillsOutput { skills, ... }` — each result includes ID, name, catalog, score, snippet |
| **Handler** | `pub async fn handle(input, state)` in `src/mcp/tools/search_skills.rs` |
| **Reuse** | Delegates to `commands::query::pipeline(args, deps)` — silent compute path; Phase 4 US1: respects project binding if present to restrict to project's workspace catalogs |
| **Reranker** | Lazily loaded; shared across calls |

#### `get_skill`

| Aspect | Details |
|--------|---------|
| **Purpose** | Retrieve single skill full detail by ID |
| **Input** | `GetSkillInput { id: String }` — `<catalog>/<plugin>/<skill-name>` |
| **Output** | `GetSkillOutput { skill: Option<SkillDetail>, ... }` |
| **Handler** | `pub async fn handle(input, state)` in `src/mcp/tools/get_skill.rs` |
| **Query** | Read-only index lookup; Phase 3 Polish: symlink rejection hardening; Phase 4 US1: project binding respected |

---

## What Does NOT Belong Here

- Internal code architecture → ARCHITECTURE.md
- Testing infrastructure → TESTING.md
- Security policies → SECURITY.md
- Dependency versions → STACK.md

---

*This document maps external service dependencies and integration points in Tome at Phase 4 Foundational F1–F11 + US1 complete. Phase 4 adds harness module abstraction with five concrete implementations, multi-level settings composition, project binding infrastructure (workspace_projects table), atomic marker landing, and skeleton for harness MCP config + rules-file sync. Binary size projection remains ~28–34 MB, well under the 50 MB cap. Integration with five agentic harnesses (Claude Code, Codex, Cursor, Gemini, OpenCode) is framework-complete; real sync wiring pending US1.b.*

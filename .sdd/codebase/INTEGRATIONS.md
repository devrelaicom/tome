# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-26 (Phase 4 US4 complete; 690+ tests across 96+ suites; v0.4.0 release)

## Databases & Data Stores

### Local SQLite Index

| Service | Type | Purpose | Location |
|---------|------|---------|----------|
| SQLite 3 | Embedded relational DB | Local skill index — metadata, embeddings, reranker scores, workspace bindings, project bindings, summary cache state | Global: `${XDG_DATA_HOME}/tome/index.db` (WAL mode); Workspace: `${WORKSPACE}/.tome/index.db` (Phase 3 Foundational F1); schema in `src/index/schema.rs` |

### Connection Patterns

- **Statically linked**: `rusqlite` with `bundled` feature — no system SQLite dependency.
- **Concurrency model**: Single advisory lockfile (`index.lock` — global or workspace-scoped) serialises writes; WAL mode allows readers during writes; MCP server uses read-only open per FR-056; Phase 4 US4: summary regeneration writes `generated_at` + hash under advisory lock for atomicity.
- **ORM/Query builder**: Direct SQL via `rusqlite` — prepared statements, parameterised queries.
- **Migration approach**: Forward-only migrations under advisory lock in `src/index/migrations.rs`; Phase 4 F1 introduces schema v2 with `workspace_catalogs` (F11 live) + `workspace_projects` (US1 live) tables; Phase 4 US4: extends meta table with summariser model identity tracking (name, version, last-known digest).

### Cache Structure

- **Catalog cache**: Each remote catalog source content-addressed by `sha256(url)` in `${XDG_DATA_HOME}/tome/catalogs/<sha256>/` (global) or `${WORKSPACE}/.tome/catalogs/<sha256>/` (workspace) — Git working tree, refreshed on `tome catalog update`. Multiple scopes can reference the same URL; shared via reference-count tracking — deleted only when no scope references it (Phase 3 / US3); Phase 4 F11: enrolment moved to `workspace_catalogs` junction table (sole source of truth per FR-360).
- **Model cache**: Downloaded model ONNX artefacts (embedder, reranker) + GGUF artefacts (summariser) stored in `${XDG_DATA_HOME}/tome/models/<model-name>/` (global, shared across scopes) with per-model `manifest.json` (strict JSON, `#[serde(deny_unknown_fields)]`); Phase 4 US4: summariser model (Qwen2.5-0.5B-Instruct GGUF, ~400 MB) downloaded alongside embedder/reranker; `tome models list --verify` validates all three via SHA-256; doctor reports summariser state.
- **Workspace summary cache**: Per-workspace `[summaries]` table in `${WORKSPACE}/.tome/settings.toml` with `short_summary`, `long_summary`, `generated_at` (RFC 3339 datetime literal), and `content_hash` (SHA-256 of input plugin list for invalidation detection); Phase 4 US4: regenerated on triggers (plugin enable/disable/reindex/catalog update) or explicit `tome workspace regen-summary`; forward-progress semantics: binding remains committed even if summarisation fails (exit 24 with partial state).
- **Atomic writes**: `tempfile` crate (rename-based) prevents corruption on SIGINT; workspace `init` uses `tempfile::Builder::tempdir_in(workspace_root)` for POSIX-atomic staging-to-final rename (Phase 3 / US2); Phase 4 US4: workspace `regen-summary` uses `toml_edit` to read/modify/write `[summaries]` table atomically.

### Workspace Registry (Phase 3 / US2, load-bearing in Phase 3 / US3, extended Phase 4 US2–US4)

- **File**: `${XDG_DATA_HOME}/tome/workspaces.txt` — opt-in (never created unless explicitly requested)
- **Format**: Line-delimited absolute paths to workspace roots; dedupe by exact-path match and canonicalize
- **Size cap**: 1 MiB; entry cap 10k (Phase 3 Polish hardening); no NUL or `..` path traversal sequences
- **Semantics**: Informational in US2; load-bearing in US3 — tracks which workspaces have been initialized via `--inherit-global`. US3 `catalog remove` consults this file to enumerate all scopes for reference-counting. Phase 4 US4: `workspace list` discovers workspaces via this optional registry (absent registry = global only); summary cache state tracked per workspace via central DB.
- **Usage**: Client harnesses can read this file to discover initialized workspaces; Tome treats absence as "no workspace scopes" (global scope only); Phase 4 US1: unused by binding algorithm (central DB is source of truth for workspace_projects); Phase 4 US4: still discovery-only for workspace list (central DB is authority for which workspaces actually exist)

---

## Authentication & Authorization

Phase 1–4 has no explicit application-layer authentication. Phase 3 / US1 MCP server is stdio-based (embedding in harness provides transport-level security). Phase 4 extends scope to workspace/project/harness level without auth changes.

- **Git operations**: Inherit system SSH keys and HTTP credential helpers (if configured in `~/.gitconfig`).
- **Hugging Face model downloads**: No API key required; public HTTPS URLs freely accessible (MODEL_REGISTRY pinned to MIT-licensed BGE variants + Apache-2.0 Qwen2.5).
- **Plugin manifest ownership**: File system permissions validate catalog ownership (email field in `tome-catalog.toml` is metadata only).
- **Workspace ownership**: Implicitly owned by the user who runs `tome workspace init`; no explicit permission model.
- **Project binding ownership**: Implicitly owned by the user who runs `tome workspace use`; binding record stored in central DB + `.tome/` marker created in project root with restricted permissions (no explicit ACL); Phase 4 US4: binding identity verified during summary regeneration (skipped if binding mismatch detected).
- **Workspace removal**: `--force` required when workspace has bound projects (FR-409); cascade teardown via `teardown_integration_for_project` removes harness-specific MCP config + rules-file entries; Phase 4 US4: summary cache deleted along with workspace settings.
- **Workspace rename**: Requires workspace to have no bound projects without `--force` (FR-410 enforces semantic constraint); atomic marker relocation via staging (Phase 4 US2); Phase 4 US4: binding identity verified during summary regeneration, harness sync respects rename identity in workspace settings.
- **Workspace regen-summary**: User runs `tome workspace regen-summary [<name>]` from any context (CLI-only, not MCP-accessible); regeneration happens under advisory lock with forward-progress semantics (binding committed even on summariser failure).
- **Credential scrubbing**: All Git stderr and model download error chains pass through `scrub_credentials()` before logging (principle XIII; extended to HF URLs and MCP log fields; Phase 4 US4: project path scrubbing in harness rules-file block insertion).
- **MCP server identity** (Phase 3 / US1): Identified by `server_info { name: "tome", version: "0.x" }` in the MCP handshake; no per-call authentication.
- **Doctor read-only access** (Phase 3 / US4): Diagnostics are read-only; repairs (`--fix`) require interactive confirmation; Phase 4 US4: extended to summariser state + summary cache freshness detection.
- **Harness config access** (Phase 4 US1–US4): Direct filesystem access to harness-owned `.mcp.json` / `.mcp.toml` files; no permission model beyond OS-level file permissions; Phase 4 US4: unchanged (harness sync independent of summarisation).

---

## External APIs

### First-Party APIs

- `commands::query::pipeline(args, deps) -> Result<QueryOutcome, TomeError>` — silent compute path reused by MCP `search_skills` tool (Phase 3 / US1.b)
- `workspace::binding::bind_project(project_root, workspace_name, deps)` — project-to-workspace binding orchestrator (Phase 4 US1.a)
- `workspace::list::list(paths) -> Result<Vec<WorkspaceInfo>, TomeError>` — workspace discovery via opt-in registry (Phase 4 US2.a)
- `workspace::rename::rename(old_name, new_name, paths) -> Result<RenameOutcome, TomeError>` — atomic workspace marker relocation with harness marker presence check (Phase 4 US2.a)
- `workspace::regen_summary::regen(name, summariser, paths) -> Result<RegenSummaryOutcome, TomeError>` — summary regeneration via configured summariser (Phase 4 US2.c, fully wired Phase 4 US4.a with `LlamaSummariser`)
- `workspace::sync::sync_for_project_root(project_root, scope, deps) -> Result<SyncOutcome, TomeError>` — harness MCP config + rules-file syncer (Phase 4 US3 complete; unchanged in US4)
- `workspace::remove::remove(name, force, paths, home, scope)` — 5-step cascade per FR-405: harness teardown, marker removal, DB cleanup, workspace dir removal, catalog refcount check; Phase 4 US4: summary cache deleted during step 1 (workspace settings deletion).
- `summarise::LlamaSummariser::new(model_path) -> Result<Self, TomeError>` — initialiser for production summariser (Phase 4 US4.a; returns early error if model not found or corrupt)
- `summarise::regenerate_for_trigger(workspace_name, deps) -> Result<SummariserOutput, TomeError>` — automatic summary regeneration triggered by plugin/catalog mutations (Phase 4 US4.b wired in lifecycle); Phase 4 US4.d-1 consolidates summary cache length checks (SHORT_MAX_CHARS / LONG_MAX_CHARS) and triggers
- Phase 4 F1–F11 + US1–US4 continues to reuse library-level APIs without new external surfaces

### Third-Party APIs

#### Hugging Face Model Registry

| Provider | Purpose | SDK/Client | Configuration |
|----------|---------|------------|---------------|
| Hugging Face (`huggingface.co`) | ONNX + GGUF model downloads (embedder, reranker, summariser) | `reqwest::blocking` (direct HTTPS) | `src/embedding/registry.rs` — `MODEL_REGISTRY` (compile-time constants); `src/summarise/registry.rs` — `SUMMARISER_NAME`, `SUMMARISER_VERSION`, `SUMMARISER_SHA256` |

**Details**:
- **Embedder**: `bge-small-en-v1.5` INT8 (~66 MB) from quantised variant
- **Reranker**: `bge-reranker-base` INT8 (~280 MB) from `onnx-community/bge-reranker-base-ONNX` (source moved Phase 3 slice 1)
- **Summariser** (Phase 4 US4): `qwen2.5-0.5b-instruct` GGUF (~400 MB, Q4_K_M quantisation) from `Qwen/Qwen2.5-0.5B-Instruct-GGUF`; Phase 4 F6 adds placeholder with all-zero checksum guard (downloads refused until real digest landed); Phase 4 US4.a ships production `LlamaSummariser`; Phase 4 US4.d-1 confirms real SHA-256 pinned: `74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db` (491,400,032 bytes)
- **Integrity**: Pinned SHA-256 + size_bytes verified post-download; no checksum endpoint (hashes are real upstream digests verified at Phase 3 slice 1 start and Phase 4 US4.d-1 for summariser)
- **Network**: HTTPS only via `rustls-tls` (no system OpenSSL)
- **Failure modes**: Network error → `TomeError::Io` (exit 7); checksum mismatch → `TomeError::ModelChecksumMismatch` (exit 32); corrupted registry → `TomeError::ModelCorrupt` (exit 31); missing model → `TomeError::ModelMissing` (exit 30); embedder drift → `TomeError::EmbedderNameDrift` (exit 41); summariser model issues → `TomeError::SummariserFailure` (exit 24) with `SummariserFailureKind::{OutputEmpty, BackendInitFailed, InferenceFailure, ModelNotFound, ModelCorrupt}`; Phase 4 US4: adds exit 24 for summarisation pipeline failures; Phase 4 US1–US3: adds harness-specific failure codes (13–20 per FR-592)
- **Explicit management**: Phase 6 wires `tome models {download,list,remove}` to manage artefacts; `tome models list --verify` validates SHA-256 per-file via `embedding::download::sha256_file()` + `summarise::download::verify_summariser_model()` (Phase 4 US4)
- **Status visibility**: Phase 8 adds `tome status [--verify]` for read-only audit without triggering downloads; Phase 4 US4: extends to include summariser model identity + state
- **Doctor integration** (Phase 3 / US4, extended Phase 4 US4): `tome doctor` reports model health (all three: embedder, reranker, summariser) with optional repair via `--fix`; Phase 4 US4: added `check_summariser` diagnostic helper
- **Scope**: Models are global (shared across all workspaces); downloaded to `${XDG_DATA_HOME}/tome/models/` regardless of active scope
- **Cache invalidation** (Phase 4 US4): Summary cache content-hash compared to current input; if hash matches, cached summaries reused; no re-download needed unless model is corrupted or missing

---

## Message Queues & Event Systems

None. Phase 3 / US1 MCP server is stdio-based (single request/response); Phase 4 adds no async event infrastructure. Phase 3 Polish: explicit SIGTERM handler for graceful shutdown (Unix-only) with 5s timeout.

---

## Caching

| Service | Purpose | TTL / Eviction | Configuration |
|---------|---------|----------------|-----------------|
| Filesystem (XDG) | Catalog Git working trees | Explicit `tome catalog remove` (user-managed); persistent; shared across scopes via refcount (Phase 3 / US3) | Global: `${XDG_DATA_HOME}/tome/catalogs/`; Workspace: `${WORKSPACE}/.tome/catalogs/` (Phase 3 Foundational F1); same URL reused — clone deleted only when all scopes drop it; Phase 3 Polish: orphan clones reported by doctor |
| Filesystem (XDG) | Downloaded model artefacts (all three: embedder, reranker, summariser) | Explicit `tome models remove` (user-managed); persistent | `${XDG_DATA_HOME}/tome/models/` — one dir per model with manifest + ONNX/GGUF files; shared across all scopes (global); Phase 4 US4: summariser model stored alongside embedder/reranker; `tome status --verify` validates all three |
| Workspace Settings TOML | Cached workspace summaries | Explicit `tome workspace regen-summary` (user-managed); invalidation on plugin enable/disable/reindex/catalog update (automatic triggers); persistent until `workspace remove` | `${WORKSPACE}/.tome/settings.toml` — `[summaries]` table with short + long + generated_at timestamp + content_hash (SHA-256 of input list); Phase 4 US4: content-hash detects stale cache; automatic invalidation baked into lifecycle triggers |

No TTL-based eviction. Explicit user commands for cleanup (principle VI). Phase 3 Polish: doctor provides advisory cleanup candidates. Phase 4 US4: workspace remove cascades summary cache deletion via workspace settings deletion.

---

## Monitoring & Observability

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| Structured logging (via `tracing`) | Diagnostic tracing to stderr (CLI) and JSON-lines to file (MCP server) | CLI: `RUST_LOG` or `TOME_LOG` environment variables; independent of `--json` stdout. MCP: JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log` per `contracts/log-format.md`; 10 MiB rotation cap; stderr reserved for fatal startup errors only (FR-222); Phase 3 Polish: custom `ContractEventFormat` emits contract-pinned field names (`ts`, `level`, `target`, `msg`); log file 0600 mode (Unix-only); credential scrubbing on `workspace_path` and `error_message` fields; Phase 4 US4: summarisation progress logged at debug level (LLM inference steps, model load time) |
| Exit codes | Scriptable error handling | 30+ enumerated codes: Phase 2 baseline + Phase 3 additions + Phase 4 F1–F11 (13–20 per FR-592 for harness/settings/summariser) + Phase 4 US4 (24 for `SummariserFailure`); documented in `contracts/exit-codes.md` and incremental updates |
| Status checks | Per-subsystem health via `tome status` | Phase 8 — models (all three), index, drift state with lazy `--verify` flag; Phase 4 US4: extended to summariser model state (present/valid/corrupt), summary cache state (present/stale/fresh), workspace binding status, settings composition validation |
| Doctor diagnostics | Subsystem health assessment + harness discovery + repair | Phase 3 / US4 — `tome doctor [--fix]` reports model/index/workspace/drift/harness health; Phase 3 Polish: orphan clone detection, registry status; Phase 4 US4: extended to summariser state (present/corrupt/drift), summary cache state (present/stale), settings composition + workspace binding drift (orphaned markers, stale DB records), harness MCP config consistency checks |

---

## File Storage

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| XDG-compliant filesystem | Configuration, catalogs, models, index, logs, workspace directories | Global: `${XDG_CONFIG_HOME}/tome/settings.toml` (Phase 4 F8+), `${XDG_DATA_HOME}/tome/catalogs/<sha>/`, `${XDG_DATA_HOME}/tome/models/` (embedder/reranker/summariser), `${XDG_DATA_HOME}/tome/index.db`, `${XDG_STATE_HOME}/tome/mcp.log`, `${XDG_DATA_HOME}/tome/workspaces.txt` (opt-in); Workspace: `${WORKSPACE}/.tome/config.toml`, `${WORKSPACE}/.tome/settings.toml` (includes `[summaries]` table per US4), `${WORKSPACE}/.tome/catalogs/<sha>/`, `${WORKSPACE}/.tome/index.db`, `${WORKSPACE}/.tome/RULES.md` (skeleton Phase 4 US1, real content Phase 4 US4); Project: `${PROJECT}/.tome/config.toml` (Phase 4 US1, binding marker), `${PROJECT}/.tome/RULES.md` (Phase 4 US4, project context + rules for summarisation), per-harness `${PROJECT}/.{claude,codex,etc}/.rules.md` / `.mcp.json` / `.mcp.toml` (Phase 4 US3, read-modify-write atomically); Phase 4 US4: workspace settings now houses summary cache alongside general workspace config |

---

## Email & Notifications

None in Phase 1–4.

---

## Agentic Coding Harness Integration (Phase 3 / US4, extended Phase 4 F1–F11 + US1–US4)

Phase 3 / US4 adds harness discovery; Phase 4 Foundational extends to harness-specific MCP config integration and settings composition; Phase 4 US1–US3 adds project binding + rules-file + MCP config sync + full settings composition + harness sync algorithm; Phase 4 US4 adds workspace summary integration (independent of harness sync — summary regeneration is CLI-only, not MCP-triggered).

| Harness | Install Location | Discovery | Purpose | Phase 4 US4 Additions |
|---------|------------------|-----------|---------|----------------------|
| Claude Code | `~/.claude` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection → Phase 4 US3 reads/validates `.mcp.json` for sync | First-party harness | US4: unchanged; `search_skills` tool description optionally includes cached short summary (max 800 chars); summary regeneration is independent of harness config |
| Codex | `~/.codex` | Existence only → Phase 4 F1+ extends to `.mcp.toml` inspection → Phase 4 US3 reads/validates `.mcp.toml` for sync | Third-party harness | US4: unchanged |
| Cursor | `~/.cursor` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection → Phase 4 US3 reads/validates `.mcp.json` for sync | Third-party harness | US4: unchanged |
| Gemini CLI | `~/.gemini` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection → Phase 4 US3 reads/validates `.mcp.json` for sync | Third-party harness | US4: unchanged |
| OpenCode | `~/.opencode` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection → Phase 4 US3 reads/validates `.mcp.json` for sync | Third-party harness | US4: unchanged |

**Discovery semantics (research §R-7, FR-167, Phase 4 R-9/R-11, unchanged US4):**
- **Probe timing**: At startup, `doctor`, or harness commands; scans `$HOME` for each harness directory; Phase 4 US1–US4: also called during binding workflow to sync harness config, during workspace removal to cascade teardown, and during workspace summary regeneration to validate harness presence (but summary regeneration doesn't update harness config)
- **Scope**: Fixed compile-time list in `src/harness/mod.rs::SUPPORTED_HARNESSES` — no dynamic discovery
- **Content read**: Phase 3 — existence only; Phase 4 F1–F11 — extends to harness-specific MCP config inspection; Phase 4 US3: harness sync reads MCP config, validates, modifies, writes back atomically; Phase 4 US4: unchanged (summary regeneration doesn't modify harness config)
- **Report shape**: `HarnessPresence { name, path, present: bool }` per contract; Phase 4: extended with optional `mcp_config_present: bool`; Phase 4 US4: unchanged (summary state reported separately)
- **Update path**: Harness module trait dispatch (`HarnessModule` impl per harness); code change + contract update (not user-configurable); Phase 4 US4: unchanged (summary regeneration is harness-independent)

**Summary integration with harnesses (Phase 4 US4):**
- **MCP tool description**: `search_skills` tool description includes the cached short summary read once at server startup (FR-425); over-length emits `tracing::warn!` but never refuses to start; Phase 4 US4: summary read from workspace-scoped cache if workspace detected, else uses static fallback description
- **Project RULES.md**: Long summary written to `${PROJECT}/.tome/RULES.md` on `tome workspace regen-summary` or automatic triggers; harness-agnostic (not specific to any one harness); workspace-scoped (applies to all projects bound to that workspace)
- **Independent of harness sync**: Summary regeneration doesn't modify harness MCP config or rules-file markers; summary cache lives in workspace settings separate from harness-specific state

---

## Settings Composition (Phase 4 F1–F11 + US1–US4)

Phase 4 Foundational F8 introduces multi-level settings composition framework reused by both CLI and MCP server. Phase 4 US1 extends with project-level config. Phase 4 US2 uses composition resolver in workspace cascade teardown. Phase 4 US3 fully wires resolver into harness sync algorithm. Phase 4 US4 uses resolver to determine summariser eligibility (unused in this phase, but infrastructure ready).

| Level | Location | Purpose | Precedence | Phase |
|-------|----------|---------|-----------|-------|
| **Project** | `${PROJECT}/.tome/config.toml` (strict, Tome-owned) + `.tome/RULES.md` | Project-specific settings: harness overrides, tool preferences; project context + rules for summarisation (lenient frontmatter) | Highest | F1 (skeleton), US1 (binding record), US3 (read in harness sync), US4 (RULES.md read for summarisation context) |
| **Workspace** | `${WORKSPACE}/.tome/settings.toml` (strict, Tome-owned) | Workspace-local enablement, harness overrides, tool preferences, summary cache (`[summaries]` table) | Medium | F8, US3 (fully wired in resolver), US4 (summary cache stored here) |
| **Global** | `${XDG_CONFIG_HOME}/tome/settings.toml` (strict, Tome-owned) | User-wide defaults, catalog list, model preferences | Lowest | F8, US3 (fully wired in resolver) |

**Composition resolver** (`src/settings/resolver.rs`, unchanged US4):
- Loads all applicable layers (project optional; workspace optional; global required)
- Merges in precedence order (project > workspace > global) following FR-441 (stop at first declaring `harnesses` key)
- Returns unified `ComposedSettings` struct with effective harness list (or empty if opted-out)
- Validation per layer (Tome-owned → strict `deny_unknown_fields`)
- Phase 4 US3: `ScopeProvider` trait defines workspace membership checks; production `CentralDbScopeProvider` queries central DB + reads workspace settings.toml; Phase 4 US4: unchanged (summary regeneration doesn't depend on composition resolver, though infrastructure is available)

**Harness-specific MCP config** (Phase 4 F8+, fully wired US3, unchanged US4):
- Location: `~/.harness/.mcp.json` or `.mcp.toml` (e.g., `~/.claude/.mcp.json`, `~/.codex/.mcp.toml`)
- Format: JSON array of tool descriptors (most harnesses) or TOML table (Codex) per MCP spec + harness-specific convention
- Edit pattern: Phase 4 US3 harness sync reads, parses into struct, validates, modifies via `HarnessModule::sync`, writes back with comment/order preservation; atomic via `NamedTempFile::persist`; Phase 4 US4: unchanged (summary regeneration doesn't touch harness config)
- Integration: Doctor reports harness MCP config state + consistency; Phase 4 US4: summary cache integration independent (tools optionally include summary description, but don't re-trigger harness config updates)

---

## Project Binding Integration (Phase 4 US1, extended Phase 4 US2–US4)

Phase 4 / US1 introduces `tome workspace use` — one-way binding from a project directory to a workspace. Phase 4 / US2 extends `workspace remove` to cascade harness teardown and binding cleanup. Phase 4 / US3 wires harness sync to respect binding identity. Phase 4 / US4 validates binding during summary regeneration (skipped if mismatch detected).

| Aspect | Details |
|--------|---------|
| **Binding semantics** | User runs `tome workspace use <workspace-name>` from a project directory; Tome records the binding in the central DB (`workspace_projects` table, PK on project_path) and creates an atomic `${PROJECT}/.tome/` marker directory; Phase 4 US2: `workspace remove` cascades by reading workspace_projects + harness compose list, tearing down per-harness entries, removing markers; Phase 4 US3: harness sync respects binding identity and uses composition resolver to determine per-project harness list; Phase 4 US4: summary regeneration verifies binding matches current workspace context (skipped if mismatch detected with warning) |
| **Storage** | Central: `workspace_projects` table in `${XDG_DATA_HOME}/tome/index.db` (1:1 mapping project_path → workspace_id); Project-local: `${PROJECT}/.tome/config.toml` (contains workspace name for verification); Phase 4 US4: workspace-scoped settings now houses summary cache; binding identity validated before regeneration |
| **Atomicity** | `bind_project` acquires advisory lock, UPSERTs DB row, lands marker dir via `tempfile::Builder::tempdir_in + rename`, releases lock; Phase 4 US3: harness sync acquires lock, reads composition/settings, calls per-harness sync methods, releases lock; Phase 4 US4: summary regeneration acquires lock, validates binding, reads project context + workspace settings, invokes summariser, writes `[summaries]` table atomically |
| **Discovery** | Doctor scans for orphaned markers (DB row absent, filesystem present); orphaned markers are advisory — can be manually removed or recovered via re-bind; Phase 4 US4: workspace regen-summary skips project-level RULES.md reads if binding is missing or stale (logged at warn level) |
| **Scope inference** | When a project is bound, `Paths::resolve()` can return the project's workspace scope if the marker is present + DB record matches. CWD walk sequence: cwd → ancestors → found `.tome/` marker → verify binding in DB → return `Scope(workspace_name)`; Phase 4 US4: scope inference used by `workspace regen-summary` to determine which workspace's settings to update |
| **CLI entry** | `tome workspace use [<workspace-name>] [--workspace <override>]` — new `WorkspaceCommand::Use` (Phase 4 US1.a); interactive selection if no workspace-name given; Phase 4 US2–US4: no new use changes (harness sync + composition resolver + summary regeneration handle project-specific context transparently) |
| **Summary regeneration** | Phase 4 US4: `tome workspace regen-summary [<name>]` loads project `.tome/RULES.md` frontmatter + body (if binding present) as input context to summariser; summary written to workspace settings; binding identity validated before regeneration (skipped with warning if mismatch) |
| **Failure modes** | Non-existent workspace → error; project already bound to different workspace → confirm + rebind; CWD not a project dir (no .git / pyproject.toml / etc.) → error; binding record stale (workspace deleted, marker orphaned) → doctor repair or manual cleanup; Phase 4 US4: project RULES.md missing/unparseable → warning, continue with minimal context; summariser failure → exit 24 with partial state (binding retained); unsupported harness in settings → exit 14; harness clash → exit 19; Phase 4 US2: workspace remove refuses without `--force` when bound projects exist; cascade per-step failures are logged but don't abort |

---

## Workspace Scope Integration (Phase 3 / US2–US3, extended Phase 4 F1–F11 + US1–US4)

**Status:** Workspace info + init landed (Phase 3 / US2); scope-aware paths (Foundational F1); reference-counted catalog sharing (US3); project binding (US1); workspace lifecycle (US2); full settings composition + harness sync (US3); workspace summary caching (US4).

| Aspect | Details |
|--------|---------|
| **Scope types** | Global (default, uses XDG paths) or Workspace (per `.tome/` directory); resolved via `Paths::resolve()` which walks `cwd` up the tree looking for `.tome/` marker; Phase 4 US4: scope inference also used for workspace-scoped summary cache lookup |
| **Path model** | Per-scope `Paths` accessor methods: `Paths::config_file_for(&Scope)`, etc. (Phase 3 Foundational F1); Phase 4 US4: workspace-scoped paths (settings file, summary cache) resolved consistently |
| **Config location** | Global: `${XDG_CONFIG_HOME}/tome/settings.toml` (Phase 4 F8+); Workspace: `${WORKSPACE}/.tome/settings.toml` (Phase 4 F8+, includes `[summaries]` table per US4); Project: `${PROJECT}/.tome/config.toml` (Phase 4 US1, binding marker); Phase 4 US4: workspace settings.toml carries both general settings + `[summaries]` table |
| **Index location** | Global: `${XDG_DATA_HOME}/tome/index.db`; Workspace: `${WORKSPACE}/.tome/index.db` (same WAL + advisory lock model); Phase 4 US4: project-scoped queries use workspace-scoped index; schema meta table tracks summariser model identity |
| **Catalog cache location** | Global: `${XDG_DATA_HOME}/tome/catalogs/<sha>/`; Workspace: `${WORKSPACE}/.tome/catalogs/<sha>/`; Phase 4 US4: unchanged (summary regeneration doesn't depend on catalog state directly, only on enabled-plugin list) |
| **Summary cache location** | Phase 4 US4: `${WORKSPACE}/.tome/settings.toml` — `[summaries]` table with short/long + generated_at (RFC 3339) + content_hash (SHA-256 of input list); workspace-scoped (applies to all projects bound to workspace) |
| **Info command** | `tome workspace info` (Phase 3 / US2.a) — read-only scope report; Phase 4 US4: no new changes to info output |
| **Init command** | `tome workspace init [<path>] [--inherit-global] [--force]` (Phase 3 / US2.b) — atomic `.tome/` creation; Phase 4 US4: no new changes to init semantics (summary cache starts empty) |
| **List command** | `tome workspace list [--json]` (Phase 4 US2.a) — discover workspaces via opt-in registry; returns `Vec<WorkspaceListItem>` with name + root + binding count + summary cache state; Phase 4 US4: extended to show summary cache presence/staleness |
| **Rename command** | `tome workspace rename <old> <new> [--force]` (Phase 4 US2.a) — atomic marker relocation via staging; requires no bound projects without `--force`; updates project marker + workspace settings + DB metadata; Phase 4 US4: summary cache preserved (namespace-independent, attached to workspace name, renamed atomically) |
| **Regen-summary command** | `tome workspace regen-summary [<name>]` (Phase 4 US2.c, fully wired Phase 4 US4.a) — regenerate `[summaries]` table via configured summariser; loads project context (`.tome/RULES.md` frontmatter if binding present) as input; caches short/long summaries + generated_at timestamp + content_hash in workspace settings; automatic triggers wired in Phase 4 US4.b (enable/disable/reindex/catalog update) |
| **Remove command** | `tome workspace remove <name> [--force]` (Phase 4 US2.b) — 5-step cascade per FR-405: harness teardown (per-project composition-aware), marker removal, DB cleanup, workspace dir removal (includes deletion of workspace settings.toml which holds summary cache), catalog refcount check; Phase 4 US4: summary cache deleted as part of workspace settings cleanup |
| **Registry file** | `${XDG_DATA_HOME}/tome/workspaces.txt` — opt-in; Phase 3 / US3 makes it load-bearing for refcount enumeration; Phase 4 US4: `workspace list` discovers workspaces via this optional registry; unchanged (central DB remains authoritative for which workspaces exist) |
| **CLI wiring** | `Command::Workspace(WorkspaceArgs)` + `WorkspaceCommand::{Info, Init, Use, List, Rename, RegenSummary, Remove}` (Phase 4 US1 adds `Use`, Phase 4 US2 adds `List/Rename/RegenSummary/Remove`); scope resolution integrated into all commands via `Paths::resolve()`; Phase 4 US4: `workspace regen-summary` driven by summariser output; automatic invalidation tied to lifecycle triggers |

---

## Schema Migration Integration (Phase 3 / US5, extended Phase 4 F1–F11 + US1–US4)

**Status:** Forward-migration framework (Phase 3 Foundational F7); integration test coverage (Phase 3 / US5); v2 schema (Phase 4 F1+); US1–US3 populate binding tables; US4 extends meta with summariser tracking.

| Aspect | Details |
|--------|---------|
| **Framework** | `src/index/migrations.rs` — `Migration` struct with function-pointer apply hooks; `apply_pending(conn, current, target)` three-arg signature; `MIGRATIONS_OVERRIDE` test-injection point |
| **Schema versions** | v0 (Phase 2 bootstrap), v1 (Phase 3 baseline), v2 (Phase 4 / F1 introduces `workspace_catalogs` + `workspace_projects` tables + meta enhancements); v2→v2.1 (Phase 4 US4 extends meta with summariser model identity) — structural-only, no data migration |
| **Test coverage** | `tests/schema_migration_e2e.rs` — integration tests via synthetic-fixture injection; Phase 4 US4: v2→v2.1 migration passes (extends meta table with new optional columns for summariser identity) |
| **Test fixtures** | `tests/common/mod.rs::write_index_db_with_schema_version` helper fabricates old-version DBs |
| **Atomicity** | All migrations run under advisory lock; rollback on error; no partial state visible to readers; Phase 4 US4: workspace summary cache invalidation tracks model identity for drift detection (separate from migration framework) |
| **Version semantics** | Write-path checks schema version, emits `SchemaVersionTooNew` (exit 73) if too new; read-path retains legacy `SchemaTooNew` (exit 52) for backward compat |
| **Production migrations** | Compile-time `MIGRATIONS` array (Phase 4 F1: v1→v2 introduces structural tables; Phase 4 US4: v2→v2.1 extends meta with summariser identity) |
| **Doctor integration** | `tome doctor` can repair schema via `--fix`; Phase 4 US4: extended to validate summariser model identity against registry; summary cache freshness checks depend on recorded model digest |

---

## Index Schema Changes (Phase 4 / F1–F11 + US1–US4)

Phase 4 / F1 introduces schema v2 with structural-only changes. Phase 4 / US4 extends v2 to v2.1 adding summariser tracking.

### New/Extended Tables (v2 → v2.1)

| Table | Purpose | Load-bearing Phase | Phase 4 US4 Changes |
|-------|---------|-------------------|---------------------|
| `workspace_catalogs` | Junction table: workspace scopes × catalog URLs; replaces `Config.catalogs` as sole source of truth per FR-360 | F11 (moved enrolment to table) | US4: unchanged; used by composition resolver + summary context (which catalogs are enabled determines plugin context for summarisation) |
| `workspace_projects` | 1:1 binding: project_path → workspace_id; primary key on `project_path` alone (FR-598) | US1 (first real usage when binding a project) | US4: harness sync validates binding, summary regeneration skips if binding mismatch detected |
| `meta` (extended) | Schema metadata; Phase 3 carries `schema_version`, `summariser_name`, `summariser_version`; Phase 4 US4 adds optional `summariser_last_verified` (RFC 3339), `summariser_verified_digest` (hex SHA-256) | F1 (v2 baseline) | US4: new optional columns track summariser model identity + last-verified digest (for drift detection during doctor checks) |

### Primary Key Changes

- `workspace_projects.project_path`: Unique constraint (1:1 binding to one workspace)
- `workspace_catalogs`: Composite key on `(workspace_id, catalog_url)` for uniqueness across scopes

---

## Environment Variables

| Variable | Required | Purpose | Example | Updated Phase |
|----------|----------|---------|---------|---------------|
| `HOME` | Yes | Base directory for XDG path resolution and harness home probe | `/Users/aaronbassett` | — |
| `XDG_CONFIG_HOME` | No (defaults to `~/.config`) | Override config directory | `/opt/etc` | — |
| `XDG_DATA_HOME` | No (defaults to `~/.local/share`) | Override data directory (models, catalogs, index.db, workspaces.txt) | `/opt/var` | Phase 4 US4: summariser model stored here; summary cache stored in workspace settings.toml |
| `XDG_STATE_HOME` | No (defaults to `~/.local/state`) | Override state directory (MCP log) | `/opt/state` | Phase 3 Foundational F8 |
| `TOME_LOG` | No | Custom log filter (overrides `RUST_LOG`) | `debug`, `info`, `tome=trace` | Phase 4 US4: includes summarisation progress, model verification |
| `RUST_LOG` | No | Standard Rust log filter | `info`, `warn` | — |
| `NO_COLOR` | No | Disable coloured output (per CLICOLOR spec) | (presence enables) | Phase 4 US4: maintained for summary output |
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
- `llama.cpp` shared library (Tome vendors + statically links via `llama-cpp-2` — Phase 4 US4)

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
| **Error scrubbing** | Captured stderr passed through `scrub_credentials()` before logging — covers URLs, tokens, SSH keys, long hex strings (principle XIII); Phase 4 US4: extended to harness rules-file + summary regeneration error paths |

---

## Third-Party Manifest Parsing

| Format | Location | Strictness | Purpose |
|--------|----------|-----------|---------|
| `plugin.json` | Catalog plugin dirs | Lenient (unknown fields ignored) | Third-party plugin metadata (FR-013a boundary) |
| SKILL.md YAML frontmatter | Upstream plugin repos | Lenient (unknown fields ignored) | Third-party skill/agent/command/hook metadata |
| `tome-catalog.toml` | Catalog root | Strict (`deny_unknown_fields`) | Tome-owned manifest; rejects typos early |
| `.tome/config.toml` (workspace) | `${WORKSPACE}/.tome/` | Strict (`deny_unknown_fields`) | Workspace marker identity; created on init |
| `.tome/config.toml` (project) | `${PROJECT}/.tome/` | Strict (`deny_unknown_fields`) | Project binding identity; created on bind; read during summary regeneration to verify binding context |
| `settings.toml` (workspace) | `${WORKSPACE}/.tome/settings.toml` | Strict (`deny_unknown_fields`) | Workspace-level settings + summary cache (`[summaries]` table); Phase 4 US4: `[summaries]` table carries short/long + generated_at + content_hash; cache invalidation checks content_hash against current input |
| `settings.toml` (global) | `${XDG_CONFIG_HOME}/tome/` | Strict (`deny_unknown_fields`) | User-wide settings; composition resolver queries this for fallback harness list |
| `.mcp.json` / `.mcp.toml` (harness) | `~/.harness/` | Lenient (parse per MCP spec) | Harness-owned MCP server config; Phase 4 US4: unchanged (summary regeneration doesn't touch harness config) |
| `.tome/RULES.md` frontmatter + body | Project root (Phase 4 US4) | YAML frontmatter (lenient) + Markdown body | Project context + rules for summarisation; auto-created on first bind; frontmatter loaded during summary regeneration as input context; body is project-specific prose |

---

## MCP Server Integration (Phase 3 / US1, hardened Phase 3 Polish, extended Phase 4 F1–F11 + US1–US4)

**Status:** Server loop + tool registration (Phase 3 / US1); Phase 4 / F1–F11 adds harness-specific config integration + extended error semantics; Phase 4 US1–US3: project binding + workspace lifecycle + harness sync + settings composition complete; Phase 4 US4: summary cache integration with tool descriptions (optional, lenient).

| Aspect | Details |
|--------|---------|
| **Protocol** | `rmcp` (1.x) — Model Context Protocol stdio server per `contracts/mcp-server.md` |
| **Runtime** | Single-threaded `tokio` backing `src/mcp/` (Phase 3 Foundational F8); scoped via `tests/sync_boundary.rs` |
| **Process model** | Stdio: stdin = MCP messages, stdout = MCP responses; stderr for fatal startup errors only (FR-222); SIGTERM handler (Unix-only) with 5s graceful-shutdown timeout |
| **Tools advertised** | Two: `search_skills` (semantic KNN + optional reranking, description optionally includes cached summary) and `get_skill` (retrieve skill detail by ID); Phase 4 US4: summary integration optional (over-length doesn't block startup) |
| **Logging** | JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log`; 10 MiB rotation; Phase 3 Polish: custom `ContractEventFormat` for contract-pinned field names; log file 0600 (Unix-only); credential scrubbing on `workspace_path` and `error_message` fields; Phase 4 US4: includes summary cache hit/miss logs at debug level |
| **Pre-flight** | FR-110 startup pipeline (schema check → drift detect → SHA-256 verify on all three models → eager-load FastembedEmbedder → load workspace summary cache if binding present) scoped to `src/mcp/preflight.rs`; Phase 4 US4: extended to load summariser model identity from settings (doesn't eagerly load summariser unless needed); over-length summary description logged at warn, doesn't block startup |
| **Tool integration** | Embedder loaded once at startup; reranker lazily on first ranking call; Phase 4 US4: summary cache loaded at startup (read-only); project scope inferred from binding if present, summary description optionally included in tool details; Phase 4 US4: tool handlers respect project-scoped harness list + workspace summary cache |
| **Tool I/O schemas** | `#[derive(JsonSchema)]` from `schemars` crate per `contracts/mcp-tools.md` |
| **Index access** | Read-only; Phase 4 US4: also reads workspace summary cache from settings.toml (workspace-scoped, not index-scoped) |
| **Error handling** | Fatal startup errors (schema too new, drift, embedder load, summariser model missing) → stderr + log + exit 60 (`McpStartupFailed`) or 61 (`McpProtocolIo`); Phase 4 US4: summariser placeholder checksum → exit 31 (`ModelCorrupt`) at startup (prevents MCP server from launching); tool errors mapped to MCP error responses |
| **Sync boundary** | All async/tokio strictly in `src/mcp/`; structural test `tests/sync_boundary.rs` enforces; Phase 4 US4: summariser (sync throughout) stays outside async; summary cache read at startup (before async reactor starts) |
| **CLI entry** | `tome mcp` — new `Command::Mcp(McpArgs)` dispatched before tracing/ctrlc init (FR-221); Phase 4 US4: no new MCP entry points (summary regeneration is CLI-only, not MCP-triggered) |
| **Phase 4 US4 extensions** | Summary cache integration in tool descriptions; over-length summaries logged at warn; summary content optional (tool still usable without it); no new MCP tools; summary regeneration remains CLI-only (not exposed to MCP callers) |

### Tool Details

#### `search_skills`

| Aspect | Details |
|--------|---------|
| **Purpose** | Semantic skill search: KNN embedding distance + optional reranking; tool description optionally includes cached workspace summary (per FR-425) |
| **Input** | `SearchSkillsInput { query, limit, force_strict, ... }` per `contracts/mcp-tools.md` |
| **Output** | `SearchSkillsOutput { skills, ... }` — each result includes ID, name, catalog, score, snippet |
| **Handler** | `pub async fn handle(input, state)` in `src/mcp/tools/search_skills.rs` |
| **Summary integration** | Phase 4 US4: tool description includes short summary if cached in workspace scope; over-length (>800 chars) emits `warn!` at startup, description still published (FR-425 allows over-length); Phase 4 US4: summary description helps harness understand workspace context without needing to query separately |
| **Reuse** | Delegates to `commands::query::pipeline(args, deps)` — silent compute path; respects project binding if present to restrict to project's workspace catalogs; respects composition-resolved harness list |
| **Reranker** | Lazily loaded; shared across calls; shared between tool requests + harness sync operations (single per-process instance via `OnceCell`) |

#### `get_skill`

| Aspect | Details |
|--------|---------|
| **Purpose** | Retrieve single skill full detail by ID |
| **Input** | `GetSkillInput { id: String }` — `<catalog>/<plugin>/<skill-name>` |
| **Output** | `GetSkillOutput { skill: Option<SkillDetail>, ... }` |
| **Handler** | `pub async fn handle(input, state)` in `src/mcp/tools/get_skill.rs` |
| **Query** | Read-only index lookup; Phase 4 US4: unchanged (summary integration is tool-description only, not skill-detail related) |

---

## What Does NOT Belong Here

- Internal code architecture → ARCHITECTURE.md
- Testing infrastructure → TESTING.md
- Security policies → SECURITY.md
- Dependency versions → STACK.md

---

*This document maps external service dependencies and integration points in Tome at Phase 4 Foundational F1–F11 + US1–US4 complete (v0.4.0 release). Phase 4 adds harness module abstraction with five concrete implementations, multi-level settings composition (fully wired), project binding infrastructure (workspace_projects table), workspace lifecycle (list/rename/sync/regen-summary/remove with atomic marker relocation), harness sync algorithm end-to-end, and workspace summary caching with LLM inference (Qwen2.5-0.5B-Instruct via llama-cpp-2). Phase 4 US4 ships production `LlamaSummariser` with real SHA-256 pinned (2026-05-26), integrated into workspace summary regeneration with content-hash cache invalidation. Summary cache stored in workspace settings.toml with automatic triggers on plugin/catalog lifecycle mutations + explicit `regen-summary` command. Binary size projection remains ~28–34 MB, well under the 50 MB cap. Integration with five agentic harnesses fully end-to-end with atomic MCP config + rules-file sync; per-project harness overrides respected; workspace-scoped cascade teardown complete; all three inference runtimes (embedder/reranker/summariser) coordinated under single model registry + status/doctor reporting.*

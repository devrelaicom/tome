# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-05-25
> **Last Updated**: 2026-05-25 (Phase 4 Foundational F1–F11 + US1–US3 complete; 677+ tests across 92+ suites; v0.4.0 trajectory)

## Databases & Data Stores

### Local SQLite Index

| Service | Type | Purpose | Location |
|---------|------|---------|----------|
| SQLite 3 | Embedded relational DB | Local skill index — metadata, embeddings, reranker scores, workspace bindings, project bindings | Global: `${XDG_DATA_HOME}/tome/index.db` (WAL mode); Workspace: `${WORKSPACE}/.tome/index.db` (Phase 3 Foundational F1); schema in `src/index/schema.rs` |

### Connection Patterns

- **Statically linked**: `rusqlite` with `bundled` feature — no system SQLite dependency.
- **Concurrency model**: Single advisory lockfile (`index.lock` — global or workspace-scoped) serialises writes; WAL mode allows readers during writes; MCP server uses read-only open per FR-056; Phase 3 Polish: validators gate entry paths (malformed config / unopenable index → `WorkspaceMalformed` exit 70); Phase 4 US1–US3: binding + workspace operations + harness sync use advisory lock for atomic UPSERT + reads + marker landing/relocation.
- **ORM/Query builder**: Direct SQL via `rusqlite` — prepared statements, parameterised queries.
- **Migration approach**: Forward-only migrations under advisory lock in `src/index/migrations.rs`; Phase 4 F1 introduces schema v2 with `workspace_catalogs` (F11 live) + `workspace_projects` (US1 live) tables; drift detection in `src/index/meta.rs`.

### Cache Structure

- **Catalog cache**: Each remote catalog source content-addressed by `sha256(url)` in `${XDG_DATA_HOME}/tome/catalogs/<sha256>/` (global) or `${WORKSPACE}/.tome/catalogs/<sha256>/` (workspace) — Git working tree, refreshed on `tome catalog update`. Multiple scopes can reference the same URL; shared via reference-count tracking — deleted only when no scope references it (Phase 3 / US3); Phase 4 F11: enrolment moved to `workspace_catalogs` junction table (sole source of truth per FR-360).
- **Model cache**: Downloaded model ONNX artefacts stored in `${XDG_DATA_HOME}/tome/models/<model-name>/` (global, shared across scopes) with per-model `manifest.json` (strict JSON, `#[serde(deny_unknown_fields)]`); Phase 6 adds explicit `tome models {download,list,remove}` commands; Phase 8 adds read-only audit via `tome status [--verify]`; Phase 4 F1: summariser model (Qwen2.5-0.5B-Instruct GGUF) added to registry alongside embedder/reranker.
- **Atomic writes**: `tempfile` crate (rename-based) prevents corruption on SIGINT; workspace `init` uses `tempfile::Builder::tempdir_in(workspace_root)` for POSIX-atomic staging-to-final rename (Phase 3 / US2); Phase 4 US1: project binding uses same atomic-dir pattern for `.tome/` marker; Phase 4 US3: harness rules-file + MCP config writes atomic via `NamedTempFile::persist`.

### Workspace Registry (Phase 3 / US2, load-bearing in Phase 3 / US3, extended Phase 4 US2–US3)

- **File**: `${XDG_DATA_HOME}/tome/workspaces.txt` — opt-in (never created unless explicitly requested)
- **Format**: Line-delimited absolute paths to workspace roots; dedupe by exact-path match and canonicalize
- **Size cap**: 1 MiB; entry cap 10k (Phase 3 Polish hardening); no NUL or `..` path traversal sequences
- **Semantics**: Informational in US2; load-bearing in US3 — tracks which workspaces have been initialized via `--inherit-global`. US3 `catalog remove` consults this file to enumerate all scopes for reference-counting. Phase 4 US2–US3: `workspace list` discovers workspaces via this optional registry (absent registry = global only).
- **Usage**: Client harnesses can read this file to discover initialized workspaces; Tome treats absence as "no workspace scopes" (global scope only); Phase 4 US1: unused by binding algorithm (central DB is source of truth for workspace_projects); Phase 4 US3: still discovery-only for workspace list (central DB is authority for which workspaces actually exist)

---

## Authentication & Authorization

Phase 1–4 has no explicit application-layer authentication. Phase 3 / US1 MCP server is stdio-based (embedding in harness provides transport-level security). Phase 4 / F1–F11 + US1–US3 maintains the same posture.

- **Git operations**: Inherit system SSH keys and HTTP credential helpers (if configured in `~/.gitconfig`).
- **Hugging Face model downloads**: No API key required; public HTTPS URLs freely accessible (MODEL_REGISTRY pinned to MIT-licensed BGE variants + Apache-2.0 Qwen2.5).
- **Plugin manifest ownership**: File system permissions validate catalog ownership (email field in `tome-catalog.toml` is metadata only).
- **Workspace ownership**: Implicitly owned by the user who runs `tome workspace init`; no explicit permission model.
- **Workspace removal**: `--force` required when workspace has bound projects (FR-409); cascade teardown via `teardown_integration_for_project` removes harness-specific MCP config + rules-file entries (Phase 4 US2).
- **Project binding ownership**: Implicitly owned by the user who runs `tome workspace use`; binding record stored in central DB + `.tome/` marker created in project root with restricted permissions (no explicit ACL); Phase 4 US3: binding identity verified during harness sync (skipped if binding mismatch detected).
- **Workspace rename**: Requires workspace to have no bound projects without `--force` (FR-410 enforces semantic constraint); atomic marker relocation via staging (Phase 4 US2); Phase 4 US3: harness sync respects rename identity in workspace settings |
- **Credential scrubbing**: All Git stderr and model download error chains pass through `scrub_credentials()` before logging (principle XIII; extended to HF URLs and MCP log fields; Phase 4 US1–US3: harness sync + workspace cascade + settings resolution paths included in scrubbing).
- **MCP server identity** (Phase 3 / US1): Identified by `server_info { name: "tome", version: "0.x" }` in the MCP handshake; no per-call authentication.
- **Doctor read-only access** (Phase 3 / US4): Diagnostics are read-only; repairs (`--fix`) require interactive confirmation; Phase 4 US3: extended to harness config drift detection.
- **Harness config access** (Phase 4 US1–US3): Direct filesystem access to harness-owned `.mcp.json` / `.mcp.toml` files; no permission model beyond OS-level file permissions; Phase 4 US3: workspace remove cascades harness config cleanup with per-harness error tolerance; harness sync validates harness existence before attempting config updates.

---

## External APIs

### First-Party APIs

- `commands::query::pipeline(args, deps) -> Result<QueryOutcome, TomeError>` — silent compute path reused by MCP `search_skills` tool (Phase 3 / US1.b)
- `workspace::binding::bind_project(project_root, workspace_name, deps)` — project-to-workspace binding orchestrator (Phase 4 US1.a)
- `workspace::list::list(paths) -> Result<Vec<WorkspaceInfo>, TomeError>` — workspace discovery via opt-in registry (Phase 4 US2.a)
- `workspace::rename::rename(old_name, new_name, paths) -> Result<RenameOutcome, TomeError>` — atomic workspace marker relocation with harness marker presence check (Phase 4 US2.a)
- `workspace::regen_summary::regen(name, summariser, paths) -> Result<RegenSummaryOutcome, TomeError>` — summary regeneration via configured summariser (Phase 4 US2.c; calls `Summariser::summarise`)
- `workspace::sync::sync_for_project_root(project_root, scope, deps) -> Result<SyncOutcome, TomeError>` — harness MCP config + rules-file syncer (Phase 4 US1.b → US3: full wiring complete; coordinates per-harness writes via `HarnessModule` trait; detects harness clash (HarnessClash exit 19) when multi-harness targets same path; forward-progress semantics: binding remains committed even on sync failure)
- `workspace::remove::remove(name, force, paths, home, scope)` — 5-step cascade per FR-405: harness teardown, marker removal, DB cleanup, workspace dir removal, catalog refcount check (Phase 4 US2.b); Phase 4 US3: teardown uses composition-aware harness list from workspace settings (respects project-level overrides)
- Phase 4 F1–F11 + US1–US3 continues to reuse library-level APIs without new external surfaces

### Third-Party APIs

#### Hugging Face Model Registry

| Provider | Purpose | SDK/Client | Configuration |
|----------|---------|------------|---------------|
| Hugging Face (`huggingface.co`) | ONNX + GGUF model downloads (embedder, reranker, summariser) | `reqwest::blocking` (direct HTTPS) | `src/embedding/registry.rs` — `MODEL_REGISTRY` (compile-time constants) |

**Details**:
- **Embedder**: `bge-small-en-v1.5` INT8 (~66 MB) from quantised variant
- **Reranker**: `bge-reranker-base` INT8 (~280 MB) from `onnx-community/bge-reranker-base-ONNX` (source moved Phase 3 slice 1)
- **Summariser** (Phase 4 F1+): `qwen2.5-0.5b-instruct` GGUF (~400 MB placeholder, real digest in US4) from `Qwen/Qwen2.5-0.5B-Instruct-GGUF`; Phase 4 F6 adds placeholder with all-zero checksum guard (downloads refused until real digest landed in US4); Phase 4 US1–US3: summariser model infrastructure complete but not actively used in harness sync (pending US4.a wiring)
- **Integrity**: Pinned SHA-256 + size_bytes verified post-download; no checksum endpoint (hashes are real upstream digests verified at Phase 3 slice 1 start)
- **Network**: HTTPS only via `rustls-tls` (no system OpenSSL)
- **Failure modes**: Network error → `TomeError::Io` (exit 7); checksum mismatch → `TomeError::ModelChecksumMismatch` (exit 32); corrupted registry → `TomeError::ModelCorrupt` (exit 31); missing model → `TomeError::ModelMissing` (exit 30); embedder drift → `TomeError::EmbedderNameDrift` (exit 41); summariser placeholder → `TomeError::ModelCorrupt` (exit 31); Phase 4 US1: adds harness-specific failure codes (13–20 per FR-592) for harness module errors; Phase 4 US3: adds exit 19 for `HarnessClash` (multi-harness same-path)
- **Explicit management**: Phase 6 wires `tome models {download,list,remove}` to manage artefacts; `tome models list --verify` validates SHA-256 per-file via `embedding::download::sha256_file()`
- **Status visibility**: Phase 8 adds `tome status [--verify]` for read-only audit without triggering downloads; Phase 4 US3: harness MCP config state included in status report
- **Doctor integration** (Phase 3 / US4): `tome doctor` reports model health with optional repair via `--fix`; Phase 3 Polish: specific exit codes for name mismatch vs missing; Phase 4 US3: extended to harness config drift detection (invalid entries, stale markers)
- **Scope**: Models are global (shared across all workspaces); downloaded to `${XDG_DATA_HOME}/tome/models/` regardless of active scope

---

## Message Queues & Event Systems

None. Phase 3 / US1 MCP server is stdio-based (single request/response); Phase 4 F1–F11 + US1–US3 adds no async event infrastructure. Phase 3 Polish: explicit SIGTERM handler for graceful shutdown (Unix-only) with 5s timeout.

---

## Caching

| Service | Purpose | TTL / Eviction | Configuration |
|---------|---------|----------------|-----------------|
| Filesystem (XDG) | Catalog Git working trees | Explicit `tome catalog remove` (user-managed); persistent; shared across scopes via refcount (Phase 3 / US3) | Global: `${XDG_DATA_HOME}/tome/catalogs/`; Workspace: `${WORKSPACE}/.tome/catalogs/` (Phase 3 Foundational F1); same URL reused — clone deleted only when all scopes drop it; Phase 3 Polish: orphan clones reported by doctor |
| Filesystem (XDG) | Downloaded model artefacts | Explicit `tome models remove` (user-managed); persistent | `${XDG_DATA_HOME}/tome/models/` — one dir per model with manifest + ONNX/GGUF files; shared across all scopes (global); Phase 3 / US4 doctor can remove corrupt models |
| Filesystem (Workspace) | Cached summaries | Explicit `tome workspace regen-summary` (user-managed); persistent; invalidation on plugin enable/disable/reindex/catalog update (pending US4) | `${WORKSPACE}/.tome/settings.toml` — `[summaries]` table with short + long + generated_at timestamp; Phase 4 US2–US3: regenerated on explicit `regen-summary` command, future US4 wires automatic invalidation triggers |

No TTL-based eviction. Explicit user commands for cleanup (principle VI). Phase 3 Polish: doctor provides advisory cleanup candidates. Phase 4 US2–US3: workspace remove cascades catalog cache cleanup via refcount check.

---

## Monitoring & Observability

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| Structured logging (via `tracing`) | Diagnostic tracing to stderr (CLI) and JSON-lines to file (MCP server) | CLI: `RUST_LOG` or `TOME_LOG` environment variables; independent of `--json` stdout. MCP: JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log` per `contracts/log-format.md`; 10 MiB rotation cap; stderr reserved for fatal startup errors only (FR-222); Phase 3 Polish: custom `ContractEventFormat` emits contract-pinned field names (`ts`, `level`, `target`, `msg`); log file 0600 mode (Unix-only); credential scrubbing on `workspace_path` and `error_message` fields; Phase 4 US1–US3: binding + workspace + harness sync operations included in scrubbing |
| Exit codes | Scriptable error handling | 30+ enumerated codes: Phase 2 baseline + Phase 3 additions + Phase 4 F1–F11 (13–20 per FR-592 for harness/settings/summariser); Phase 4 US1–US3: adds 12/14/19 for harness sync errors + clash detection; documented in `contracts/exit-codes.md` and incremental updates |
| Status checks | Per-subsystem health via `tome status` | Phase 8 — models, index, drift state with lazy `--verify` flag; Phase 4 US1–US3: extended to summariser state, harness MCP config state (present/absent/parseable), workspace binding status, workspace summary cache state, settings composition validation |
| Doctor diagnostics | Subsystem health assessment + harness discovery + repair | Phase 3 / US4 — `tome doctor [--fix]` reports model/index/workspace/drift/harness health; Phase 3 Polish: orphan clone detection, registry status; Phase 4 US1–US3: extended to summariser state, harness config state, settings composition, workspace binding drift (orphaned markers, stale DB records), workspace summary cache freshness, harness MCP config consistency checks |

---

## File Storage

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| XDG-compliant filesystem | Configuration, catalogs, models, index, logs, workspace directories | Global: `${XDG_CONFIG_HOME}/tome/settings.toml` (Phase 4 F8+), `${XDG_DATA_HOME}/tome/catalogs/<sha>/`, `${XDG_DATA_HOME}/tome/models/`, `${XDG_DATA_HOME}/tome/index.db`, `${XDG_STATE_HOME}/tome/mcp.log`, `${XDG_DATA_HOME}/tome/workspaces.txt` (opt-in); Workspace: `${WORKSPACE}/.tome/config.toml`, `${WORKSPACE}/.tome/settings.toml` (Phase 4 F8+), `${WORKSPACE}/.tome/catalogs/<sha>/`, `${WORKSPACE}/.tome/index.db`, `${WORKSPACE}/.tome/RULES.md` (skeleton Phase 4 US1, real content Phase 4 US4); Project: `${PROJECT}/.tome/config.toml` (Phase 4 US1), `${PROJECT}/.tome/RULES.md` (Phase 4 US4), per-harness `${PROJECT}/.{claude,codex,etc}/.rules.md` / `.mcp.json` / `.mcp.toml` (Phase 4 US3, read-modify-write atomically); Phase 4 US2–US3: workspace directories atomic-relocatable via staging |

---

## Email & Notifications

None in Phase 1–4.

---

## Agentic Coding Harness Integration (Phase 3 / US4, extended Phase 4 F1–F11 + US1–US3)

Phase 3 / US4 adds harness discovery; Phase 4 Foundational extends to harness-specific MCP config integration and settings composition; Phase 4 US1 adds project binding + rules-file + MCP config sync; Phase 4 US2 extends workspace removal to cascade harness teardown; Phase 4 US3 wires harness sync algorithm end-to-end with per-harness MCP config + rules-file atomicity.

| Harness | Install Location | Discovery | Purpose | Phase 4 Additions |
|---------|------------------|-----------|---------|-------------------|
| Claude Code | `~/.claude` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection → Phase 4 US3 reads/validates `.mcp.json` for sync | First-party harness | F7: `src/harness/claude_code.rs` impl; F8: MCP config read-modify-write; US1: sync algorithm skeleton; US3: full sync wiring — reads `.mcp.json`, validates, inserts `"tome"` entry, writes back atomically |
| Codex | `~/.codex` | Existence only → Phase 4 F1+ extends to `.mcp.toml` inspection → Phase 4 US3 reads/validates `.mcp.toml` for sync | Third-party harness | F7: `src/harness/codex.rs` impl; F8: TOML-specific read-modify-write via `toml_edit`; US1: sync algorithm skeleton; US3: full sync wiring — reads `.mcp.toml` via `toml_edit`, validates, inserts `[mcp_servers.tome]` entry, writes back with comment/order preservation |
| Cursor | `~/.cursor` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection → Phase 4 US3 reads/validates `.mcp.json` for sync | Third-party harness | F7: `src/harness/cursor.rs` impl; F8: JSON + standalone rules-file support; US1: sync algorithm skeleton; US3: full sync wiring — reads `.mcp.json`, inserts entry, writes back; standalone `.rules.md` created atomically in `~/.cursor/rules/` |
| Gemini CLI | `~/.gemini` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection → Phase 4 US3 reads/validates `.mcp.json` for sync | Third-party harness | F7: `src/harness/gemini.rs` impl; F8: MCP config + block rules-file; US1: sync algorithm skeleton; US3: full sync wiring — reads `.mcp.json`, inserts entry, writes back; block rules-file entry in `.gemini/rules.md` |
| OpenCode | `~/.opencode` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection → Phase 4 US3 reads/validates `.mcp.json` for sync | Third-party harness | F7: `src/harness/opencode.rs` impl; F8: MCP config + block rules-file; US1: sync algorithm skeleton; US3: full sync wiring — reads `.mcp.json`, inserts entry, writes back; block rules-file entry in `~/.opencode/rules.md` |

**Discovery semantics (research §R-7, FR-167, Phase 4 R-9/R-11):**
- **Probe timing**: At startup, `doctor`, or harness commands; scans `$HOME` for each harness directory; Phase 4 US1–US3: also called during binding workflow to sync harness config, and during workspace removal to cascade teardown
- **Scope**: Fixed compile-time list in `src/harness/mod.rs::SUPPORTED_HARNESSES` — no dynamic discovery
- **Content read**: Phase 3 — existence only; Phase 4 F1–F11 — extends to harness-specific MCP config inspection (comment-preserving read via `toml_edit` for `Codex`, `serde_json` for others); Phase 4 US3: harness sync reads MCP config, validates, modifies, writes back atomically; clash detection when two harnesses target same path (HarnessClash exit 19)
- **Report shape**: `HarnessPresence { name, path, present: bool }` per contract; Phase 4: extended with optional `mcp_config_present: bool`; Phase 4 US3: binding outcome includes per-harness sync result with status + error detail, remove outcome includes per-harness teardown result
- **Update path**: Harness module trait dispatch (`HarnessModule` impl per harness); code change + contract update (not user-configurable); Phase 4 US3: sync algorithm is data-driven (reads settings.toml `harnesses` list, dispatches per `HarnessModule::sync` impl)

**Harness module architecture (Phase 4 F7+, fully wired US3):**
- **Trait**: `HarnessModule` — defines home dir, MCP config format (JSON / TOML), rules-file strategy (Block / Standalone), parent key, read/write/delete operations; Phase 4 US3: trait includes `sync(project_root, workspace_settings, deps)` method
- **Registry**: `SUPPORTED_HARNESSES: &[&dyn HarnessModule]` in `mod.rs`; lookup by name via `harness::lookup(&str) -> Option<&'dyn HarnessModule>`
- **Implementations**: Five concrete impls (`claude_code`, `codex`, `cursor`, `gemini`, `opencode`); each pins format + path decisions per contract + upstream harness docs; Phase 4 US3: each impl's `sync` method handles project marker creation, MCP config entry insertion, rules-file block insertion
- **Test injection**: `HARNESS_MODULES_OVERRIDE` thread-local for test-injecting `StubHarness` (Phase 4 US1–US3 test discipline)
- **Sync algorithm** (Phase 4 US3): `src/harness/sync.rs::sync_for_project_root(project_root, scope, deps) -> SyncOutcome` orchestrates read/modify/write per harness; acquires advisory lock, iterates effective harness list from settings resolver, calls `HarnessModule::sync` per harness, detects path clashes (HarnessClash exit 19), releases lock; failure on one harness doesn't abort others (forward-progress); outcome envelope lists per-harness status + errors
- **Remove algorithm** (Phase 4 US2–US3): `src/workspace/remove.rs::teardown_integration_for_project` calls `harness::rules_file::remove_*` + `harness::mcp_config::remove_entry` per harness; Phase 4 US3: uses composition-aware harness list from workspace settings (respects project-level overrides)
- **Rules file strategy**: `BlockInExistingFile` (Claude Code, Codex, Gemini, OpenCode) or `StandaloneFile` (Cursor); implemented in `src/harness/rules_file.rs`; Phase 4 US3: markers preserved during sync, comment-preserving writes
- **MCP config strategy**: JSON (Claude Code, Cursor, Gemini, OpenCode) or TOML (Codex); implemented in `src/harness/mcp_config.rs`; Phase 4 US3: atomic writes via `NamedTempFile::persist`, comment/order preservation via `toml_edit` + `serde_json/preserve_order`

---

## Settings Composition (Phase 4 F1–F11 + US1–US3)

Phase 4 Foundational F8 introduces multi-level settings composition framework reused by both CLI and MCP server. Phase 4 US1 extends with project-level config. Phase 4 US2 uses composition resolver in workspace cascade teardown. Phase 4 US3 fully wires resolver into harness sync algorithm with `CentralDbScopeProvider` checking workspace membership.

| Level | Location | Purpose | Precedence | Phase |
|-------|----------|---------|-----------|-------|
| **Project** | `${PROJECT}/.tome/config.toml` (strict, Tome-owned) | Project-specific settings: harness overrides, tool preferences, project binding identity | Highest | F1 (skeleton), US1 (binding record), US3 (read in harness sync) |
| **Project (alt)** | `.tome/RULES.md` (lenient frontmatter + Markdown body) | Project-specific context + rules for summarisation; parsed on-demand by summariser | — | F1 (skeleton), US4 (real parsing) |
| **Workspace** | `${WORKSPACE}/.tome/settings.toml` (strict, Tome-owned) | Workspace-local enablement, harness overrides, tool preferences | Medium | F8, US3 (fully wired in resolver) |
| **Global** | `${XDG_CONFIG_HOME}/tome/settings.toml` (strict, Tome-owned) | User-wide defaults, catalog list, model preferences | Lowest | F8, US3 (fully wired in resolver) |

**Composition resolver** (`src/settings/resolver.rs`):
- Loads all applicable layers (project optional; workspace optional; global required)
- Merges in precedence order (project > workspace > global) following FR-441 (stop at first declaring `harnesses` key)
- Returns unified `ComposedSettings` struct with effective harness list (or empty if opted-out)
- Validation per layer (Tome-owned → strict `deny_unknown_fields`)
- Phase 4 US3: `ScopeProvider` trait defines workspace membership checks; production `CentralDbScopeProvider` queries central DB + reads workspace settings.toml; `StubScope::new()` for test/non-production paths
- Phase 4 US3: composition resolver fully wired into harness sync (`resolve_effective_list` with production `CentralDbScopeProvider`)
- Phase 4 US3: composition logic validates harness references (unsupported harness detection → exit 14 `UnsupportedHarness`)

**Harness-specific MCP config** (Phase 4 F8+, fully wired US3):
- Location: `~/.harness/.mcp.json` or `.mcp.toml` (e.g., `~/.claude/.mcp.json`, `~/.codex/.mcp.toml`)
- Format: JSON array of tool descriptors (most harnesses) or TOML table (Codex) per MCP spec + harness-specific convention
- Edit pattern: Phase 4 US3 harness sync reads, parses into struct, validates, modifies via `HarnessModule::sync`, writes back with comment/order preservation (`toml_edit` + `serde_json`); atomic via `NamedTempFile::persist`; clash detection when multi-harness targets same path
- Integration: Doctor reports harness MCP config state + consistency; settings composition resolver can query harness config; Phase 4 US3: harness sync driven by composition resolver, uses per-harness sync methods; workspace cascade uses same resolver
- **Project-level harness overrides** (Phase 4 US3): `${PROJECT}/.tome/config.toml` can declare harness-specific settings that override workspace/global for that project's context; composition resolver respects overrides when resolving effective list; harness sync enforces overrides at read/modify/write time

---

## Project Binding Integration (Phase 4 US1, extended Phase 4 US2–US3)

Phase 4 / US1 introduces `tome workspace use` — one-way binding from a project directory to a workspace, enabling project-scoped skill indexing and summarisation. Phase 4 / US2 extends `workspace remove` to cascade harness teardown and binding cleanup. Phase 4 / US3 wires harness sync to respect binding identity and use composition resolver for per-project harness list.

| Aspect | Details |
|--------|---------|
| **Binding semantics** | User runs `tome workspace use <workspace-name>` from a project directory; Tome records the binding in the central DB (`workspace_projects` table, PK on project_path) and creates an atomic `${PROJECT}/.tome/` marker directory; Phase 4 US2: `workspace remove` cascades by reading workspace_projects + harness compose list, tearing down per-harness entries, removing markers; Phase 4 US3: harness sync respects binding identity and uses composition resolver to determine per-project harness list |
| **Storage** | Central: `workspace_projects` table in `${XDG_DATA_HOME}/tome/index.db` (1:1 mapping project_path → workspace_id); Project-local: `${PROJECT}/.tome/config.toml` (contains workspace name for verification); Phase 4 US3: workspace-scoped settings cache binding identity, composition resolver validates binding identity matches before sync |
| **Atomicity** | `bind_project` acquires advisory lock, UPSERTs DB row, lands marker dir via `tempfile::Builder::tempdir_in + rename`, releases lock; Phase 4 US3: harness sync acquires lock, reads composition/settings, calls per-harness sync methods, releases lock. Phase 4 US2: cascade remove uses same lock window for all 5 steps (harness teardown, marker removal, DB cleanup, workspace dir removal, catalog refcount check); per-step failures are non-fatal (logged at warn, cascade continues) |
| **Discovery** | Doctor scans for orphaned markers (DB row absent, filesystem present); orphaned markers are advisory — can be manually removed or recovered via re-bind; Phase 4 US3: workspace remove lists bound projects upfront, requires `--force` if any are present; harness sync checks binding exists + matches before proceeding (binding identity mismatch → skip sync for that project, log error) |
| **Scope inference** | When a project is bound, `Paths::resolve()` can return the project's workspace scope if the marker is present + DB record matches. CWD walk sequence: cwd → ancestors → found `.tome/` marker → verify binding in DB → return `Scope(workspace_name)`; Phase 4 US3: scope inference used in harness sync to resolve workspace settings |
| **CLI entry** | `tome workspace use [<workspace-name>] [--workspace <override>]` — new `WorkspaceCommand::Use` (Phase 4 US1.a); interactive selection if no workspace-name given; Phase 4 US2–US3: no new use changes (harness sync + composition resolver handle project-specific overrides transparently) |
| **Harness sync** | Phase 4 US1.b skeleton → Phase 4 US3: `commands::harness::sync_for_project_root(project_root)` called post-binding to sync harness MCP config + rules-file; fully wired — acquires advisory lock, resolves workspace scope from binding, reads composition (project > workspace > global), calls per-harness `HarnessModule::sync`, detects clash, returns SyncOutcome; forward-progress: binding remains committed even on sync failure |
| **Failure modes** | Non-existent workspace → error; project already bound to different workspace → confirm + rebind; CWD not a project dir (no .git / pyproject.toml / etc.) → error; binding record stale (workspace deleted, marker orphaned) → doctor repair or manual cleanup; Phase 4 US3: binding identity mismatch during harness sync → log warning, skip that project's sync (forward-progress); unsupported harness in settings → exit 14; harness clash (multi-harness same-path) → exit 19; Phase 4 US2: workspace remove refuses without `--force` when bound projects exist; cascade per-step failures are logged but don't abort |

---

## Workspace Scope Integration (Phase 3 / US2–US3, extended Phase 4 F1–F11 + US1–US3)

**Status:** Workspace info + init landed (Phase 3 / US2); scope-aware paths (Foundational F1); reference-counted catalog sharing (US3); project binding (US1); workspace lifecycle (US2); full settings composition + harness sync (US3). Phase 4 / F1–F11 + US1–US3: extends scope model with WorkspaceName + project binding + settings composition + list/rename/remove + harness sync.

| Aspect | Details |
|--------|---------|
| **Scope types** | Global (default, uses XDG paths) or Workspace (per `.tome/` directory); resolved via `Paths::resolve()` which walks `cwd` up the tree looking for `.tome/` marker; Phase 4 F1–F11: `Scope` becomes `WorkspaceName` newtype + `Scope(WorkspaceName)` tuple struct (F10); Phase 4 US1–US3: extended with project marker detection + binding verification; Phase 4 US3: scope inference used by harness sync to determine workspace settings |
| **Path model** | Per-scope `Paths` accessor methods: `Paths::config_file_for(&Scope)`, etc. (Phase 3 Foundational F1); Phase 4 F11: scope model simplified (deleted enum variants, all scopes now use WorkspaceName); Phase 4 US3: project paths resolved via binding lookup + scope inference when binding present |
| **Config location** | Global: `${XDG_CONFIG_HOME}/tome/settings.toml` (Phase 4 F8+); Workspace: `${WORKSPACE}/.tome/settings.toml` (Phase 4 F8+); Project: `${PROJECT}/.tome/config.toml` (Phase 4 US1, binding marker); Phase 4 US3: harness sync reads composition from all three layers in precedence order |
| **Index location** | Global: `${XDG_DATA_HOME}/tome/index.db`; Workspace: `${WORKSPACE}/.tome/index.db` (same WAL + advisory lock model); Phase 4 US3: project-scoped queries use workspace-scoped index (the workspace that owns the binding); harness sync may consult central DB for workspace membership validation |
| **Catalog cache location** | Global: `${XDG_DATA_HOME}/tome/catalogs/<sha>/`; Workspace: `${WORKSPACE}/.tome/catalogs/<sha>/`; Phase 4 F11: enrolment moved to `workspace_catalogs` junction table (sole source of truth); Phase 4 US3: project inherits workspace's catalogs (no project-local catalog list); harness sync uses workspace catalogs to determine skill search scope |
| **Reference counting (Phase 3 / US3)** | `catalog::store::reference_count(url, paths) -> Vec<Scope>` enumerates scopes that reference a URL; Phase 4 F11: extended to junction-table query via `src/index/workspace_catalogs.rs`; Phase 4 US3: cascade cleanup in workspace remove uses refcount to decide on-disk cache deletion |
| **Info command** | `tome workspace info` (Phase 3 / US2.a) — read-only scope report; Phase 4 F1–F11 + US1–US3: no new changes to info output |
| **Init command** | `tome workspace init [<path>] [--inherit-global] [--force]` (Phase 3 / US2.b) — atomic `.tome/` creation; Phase 4 F1–F11 + US1–US3: no new changes to init semantics |
| **List command** | `tome workspace list [--json]` (Phase 4 US2.a) — discover workspaces via opt-in registry; returns `Vec<WorkspaceListItem>` with name + root + binding count; Phase 4 US3: unchanged (still discovery-only) |
| **Rename command** | `tome workspace rename <old> <new> [--force]` (Phase 4 US2.a) — atomic marker relocation via staging; requires no bound projects without `--force` (FR-410); updates project marker + workspace settings + DB metadata; Phase 4 US3: harness sync uses composition resolver which validates workspace identity |
| **Regen-summary command** | `tome workspace regen-summary [<name>]` (Phase 4 US2.c) — regenerate `[summaries]` table via configured summariser; caches short/long summaries + generated_at timestamp in workspace settings; Phase 4 US3: unchanged (harness sync doesn't depend on summaries) |
| **Remove command** | `tome workspace remove <name> [--force]` (Phase 4 US2.b) — 5-step cascade per FR-405: harness teardown (per-project composition-aware), marker removal, DB cleanup, workspace dir removal, catalog refcount check; requires `--force` when bound projects exist; Phase 4 US3: harness teardown uses composition resolver to determine per-project harness set |
| **Registry file** | `${XDG_DATA_HOME}/tome/workspaces.txt` — opt-in; Phase 3 / US3 makes it load-bearing for refcount enumeration; Phase 4 US3: `workspace list` discovers workspaces via this optional registry (absent registry = global only); harness sync doesn't use registry (central DB is authoritative) |
| **CLI wiring** | `Command::Workspace(WorkspaceArgs)` + `WorkspaceCommand::{Info, Init, Use, List, Rename, RegenSummary, Remove}` (Phase 4 US1 adds `Use`, Phase 4 US2 adds `List/Rename/RegenSummary/Remove`); scope resolution integrated into all commands via `Paths::resolve()`; Phase 4 US3: `harness::sync_for_project_root` called post-binding to sync harness config |

---

## Schema Migration Integration (Phase 3 / US5, extended Phase 4 F1–F11 + US1–US3)

**Status:** Forward-migration framework (Phase 3 Foundational F7); integration test coverage (Phase 3 / US5); v2 schema (Phase 4 F1+). Phase 4 / F1–F11 + US1–US3: extends schema with project binding table, workspace summary cache metadata.

| Aspect | Details |
|--------|---------|
| **Framework** | `src/index/migrations.rs` — `Migration` struct with function-pointer apply hooks; `apply_pending(conn, current, target)` three-arg signature; `MIGRATIONS_OVERRIDE` test-injection point |
| **Schema versions** | v0 (Phase 2 bootstrap), v1 (Phase 3 baseline), v2 (Phase 4 / F1 introduces `workspace_catalogs` + `workspace_projects` tables, structural-only, no data migration) |
| **Test coverage** | `tests/schema_migration_e2e.rs` — integration tests via synthetic-fixture injection; Phase 4 F1–F11 + US1–US3: v1→v2 migration passes (tables are structural-only, safe to create empty) |
| **Test fixtures** | `tests/common/mod.rs::write_index_db_with_schema_version` helper fabricates old-version DBs |
| **Atomicity** | All migrations run under advisory lock; rollback on error; no partial state visible to readers; Phase 4 US3: workspace harness sync also runs under same advisory lock |
| **Version semantics** | Write-path checks schema version, emits `SchemaVersionTooNew` (exit 73) if too new; read-path retains legacy `SchemaTooNew` (exit 52) for backward compat |
| **Production migrations** | Compile-time `MIGRATIONS` array (Phase 4 F1: v1→v2 structural-only migration registered, adds `workspace_catalogs` + `workspace_projects` tables) |
| **Doctor integration** | `tome doctor` can repair schema via `--fix`; Phase 4 US3: extended to validate workspace_catalogs junction table + workspace_projects binding consistency + harness config state |

---

## Index Schema Changes (Phase 4 / F1–F11 + US1–US3)

Phase 4 / F1 introduces schema v2 with structural-only changes (no data migration needed, new tables are optional until load-bearing phases). Phase 4 / US1–US3 populates schema tables.

### New Tables (v2)

| Table | Purpose | Load-bearing Phase | Phase 4 Additions in US1–US3 |
|-------|---------|-------------------|---------------------------|
| `workspace_catalogs` | Junction table: workspace scopes × catalog URLs; replaces `Config.catalogs` as sole source of truth per FR-360 | F11 (moved enrolment to table) | US3 (unchanged; used by composition resolver + sync) |
| `workspace_projects` | 1:1 binding: project_path → workspace_id; primary key on `project_path` alone (FR-598) | US1 (first real usage when binding a project) | US3 (harness sync validates binding, respects per-project harness overrides) |
| `workspace_summaries` (future US4) | Per-workspace summary cache metadata (pending implementation) | US4 (wired in full summarisation lifecycle) | US3: skeleton only (regen-summary writes `[summaries]` to workspace settings.toml, not index.db) |

### Primary Key Changes

- `workspace_projects.project_path`: Unique constraint (1:1 binding to one workspace)
- `workspace_catalogs`: Composite key on `(workspace_id, catalog_url)` for uniqueness across scopes

---

## Environment Variables

| Variable | Required | Purpose | Example | Updated Phase |
|----------|----------|---------|---------|---------------|
| `HOME` | Yes | Base directory for XDG path resolution and harness home probe | `/Users/aaronbassett` | — |
| `XDG_CONFIG_HOME` | No (defaults to `~/.config`) | Override config directory | `/opt/etc` | — |
| `XDG_DATA_HOME` | No (defaults to `~/.local/share`) | Override data directory (models, catalogs, index.db, workspaces.txt) | `/opt/var` | Phase 3 / US2 (workspaces.txt); Phase 4 / F1–F11 (settings composition); Phase 4 / US3 (harness sync config resolution) |
| `XDG_STATE_HOME` | No (defaults to `~/.local/state`) | Override state directory (MCP log) | `/opt/state` | Phase 3 Foundational F8 |
| `TOME_LOG` | No | Custom log filter (overrides `RUST_LOG`) | `debug`, `info`, `tome=trace` | Phase 4 US3: harness sync diagnostics included |
| `RUST_LOG` | No | Standard Rust log filter | `info`, `warn` | — |
| `NO_COLOR` | No | Disable coloured output (per CLICOLOR spec) | (presence enables) | Phase 3 Polish (consistent coverage); Phase 4 / US3 (maintained) |
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
| **Error scrubbing** | Captured stderr passed through `scrub_credentials()` before logging — covers URLs, tokens, SSH keys, long hex strings (principle XIII); Phase 3 Polish: extended to MCP log field scrubbing; Phase 4 US1–US3: extended to harness sync + workspace cascade error paths |

---

## Third-Party Manifest Parsing

| Format | Location | Strictness | Purpose |
|--------|----------|-----------|---------|
| `plugin.json` | Catalog plugin dirs | Lenient (unknown fields ignored) | Third-party plugin metadata (FR-013a boundary) |
| SKILL.md YAML frontmatter | Upstream plugin repos | Lenient (unknown fields ignored) | Third-party skill/agent/command/hook metadata |
| `tome-catalog.toml` | Catalog root | Strict (`deny_unknown_fields`) | Tome-owned manifest; rejects typos early |
| `.tome/config.toml` (workspace) | `${WORKSPACE}/.tome/` | Strict (`deny_unknown_fields`) | Workspace marker identity; created on init |
| `.tome/config.toml` (project) | `${PROJECT}/.tome/` | Strict (`deny_unknown_fields`) | Project binding identity; created on bind; Phase 4 US3: read during harness sync to verify binding context |
| `settings.toml` (workspace) | `${WORKSPACE}/.tome/settings.toml` | Strict (`deny_unknown_fields`) | Workspace-level settings; Phase 4 F8+; Phase 4 US3: fully wired in composition resolver + harness sync; validate harness references + detect unsupported harnesses (exit 14) |
| `settings.toml` (global) | `${XDG_CONFIG_HOME}/tome/` | Strict (`deny_unknown_fields`) | User-wide settings; Phase 4 F8+; Phase 4 US3: highest precedence in composition walk |
| `.mcp.json` / `.mcp.toml` (harness) | `~/.harness/` | Lenient (parse per MCP spec) | Harness-owned MCP server config; Phase 4 US3: read-modify-write atomically via `HarnessModule::sync` |
| `.tome/RULES.md` frontmatter + body | Project root (Phase 4 US4) | YAML frontmatter (lenient) + Markdown body | Project context + rules for summarisation; auto-created on first bind; Phase 4 US3: skeleton landing with `~` markers for project/workspace names |

---

## MCP Server Integration (Phase 3 / US1, hardened Phase 3 Polish, extended Phase 4 F1–F11 + US1–US3)

**Status:** Server loop + tool registration (Phase 3 / US1); Phase 4 / F1–F11 adds harness-specific config integration + extended error semantics; Phase 4 US1–US3: project binding + workspace lifecycle + harness sync + settings composition complete.

| Aspect | Details |
|--------|---------|
| **Protocol** | `rmcp` (1.x) — Model Context Protocol stdio server per `contracts/mcp-server.md` |
| **Runtime** | Single-threaded `tokio` backing `src/mcp/` (Phase 3 Foundational F8); scoped via `tests/sync_boundary.rs` |
| **Process model** | Stdio: stdin = MCP messages, stdout = MCP responses; stderr for fatal startup errors only (FR-222); SIGTERM handler (Unix-only) with 5s graceful-shutdown timeout |
| **Tools advertised** | Two: `search_skills` (semantic KNN + optional reranking) and `get_skill` (retrieve skill detail by ID); Phase 4 US4: pending third tool for project context / summaries |
| **Logging** | JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log`; 10 MiB rotation; Phase 3 Polish: custom `ContractEventFormat` for contract-pinned field names; log file 0600 (Unix-only); credential scrubbing on `workspace_path` and `error_message` fields; Phase 4 US1–US3: binding + workspace + harness operations included in scrubbing |
| **Pre-flight** | FR-110 startup pipeline (schema check → drift detect → SHA-256 verify → eager-load FastembedEmbedder) scoped to `src/mcp/preflight.rs`; Phase 4 US1–US3: extended to harness MCP config validation, project binding check, summariser placeholder check (exit 31 if all-zero checksum), settings composition validation (exit 14 if unsupported harness) |
| **Tool integration** | Embedder loaded once at startup; reranker lazily on first ranking call; Phase 4 US1–US3: summariser lazily on first project-context request (not yet wired in tools, but infrastructure ready); project scope inferred from binding if present, else global; Phase 4 US3: tool handlers respect project-scoped harness list |
| **Tool I/O schemas** | `#[derive(JsonSchema)]` from `schemars` crate per `contracts/mcp-tools.md` |
| **Index access** | Read-only; Phase 3 Polish: symlink rejection hardening in skill walk; Phase 4 US1–US3: project-scoped skill search uses workspace-scoped index if binding present; Phase 4 US3: tool handlers respect composition-resolved harness list for skill filtering |
| **Error handling** | Fatal startup errors (schema too new, drift, embedder load) → stderr + log + exit 60 (`McpStartupFailed`) or 61 (`McpProtocolIo`); Phase 4 F1–F11 + US1–US3: adds 10 new exit codes (13–20, 32) for harness + settings + summariser + model failures per FR-592; tool errors mapped to MCP error responses |
| **Sync boundary** | All async/tokio strictly in `src/mcp/`; structural test `tests/sync_boundary.rs` enforces; Phase 4 US1–US3: harness sync + workspace lifecycle run outside MCP (CLI-only); MCP tools don't trigger sync (Phase 4 US4 deferred) |
| **CLI entry** | `tome mcp` — new `Command::Mcp(McpArgs)` dispatched before tracing/ctrlc init (FR-221); Phase 4 US1–US3: no new MCP entry points (binding + workspace lifecycle + harness sync are CLI-only) |
| **Phase 4 extensions** | Harness-specific MCP config integration via `src/harness/` module (F7–US3); settings composition resolver in `src/settings/` (F8–US3); project binding infrastructure in `src/workspace/binding.rs` + `src/index/workspace_projects` (US1–US3); workspace lifecycle (list/rename/regen-summary/remove) in `src/workspace/` (US2–US3); harness sync algorithm end-to-end wired in `src/harness/sync.rs` (US3); summariser skeleton in `src/summarise/` (F6); project context loading from `.tome/RULES.md` (US4) |

### Tool Details

#### `search_skills`

| Aspect | Details |
|--------|---------|
| **Purpose** | Semantic skill search: KNN embedding distance + optional reranking |
| **Input** | `SearchSkillsInput { query, limit, force_strict, ... }` per `contracts/mcp-tools.md` |
| **Output** | `SearchSkillsOutput { skills, ... }` — each result includes ID, name, catalog, score, snippet |
| **Handler** | `pub async fn handle(input, state)` in `src/mcp/tools/search_skills.rs` |
| **Reuse** | Delegates to `commands::query::pipeline(args, deps)` — silent compute path; Phase 4 US1–US3: respects project binding if present to restrict to project's workspace catalogs; Phase 4 US3: respects composition-resolved harness list (if applicable) |
| **Reranker** | Lazily loaded; shared across calls; Phase 4 US3: shared between tool requests + harness sync operations (single per-process instance via `OnceCell`) |

#### `get_skill`

| Aspect | Details |
|--------|---------|
| **Purpose** | Retrieve single skill full detail by ID |
| **Input** | `GetSkillInput { id: String }` — `<catalog>/<plugin>/<skill-name>` |
| **Output** | `GetSkillOutput { skill: Option<SkillDetail>, ... }` |
| **Handler** | `pub async fn handle(input, state)` in `src/mcp/tools/get_skill.rs` |
| **Query** | Read-only index lookup; Phase 3 Polish: symlink rejection hardening; Phase 4 US1–US3: project binding respected; Phase 4 US3: composition-aware scope used if binding present |

---

## What Does NOT Belong Here

- Internal code architecture → ARCHITECTURE.md
- Testing infrastructure → TESTING.md
- Security policies → SECURITY.md
- Dependency versions → STACK.md

---

*This document maps external service dependencies and integration points in Tome at Phase 4 Foundational F1–F11 + US1–US3 complete. Phase 4 adds harness module abstraction with five concrete implementations, multi-level settings composition (fully wired US3 with `CentralDbScopeProvider`), project binding infrastructure (workspace_projects table), workspace lifecycle (list/rename/sync/regen-summary/remove with atomic marker relocation), and harness sync algorithm end-to-end. Phase 4 US3 closes settings resolver integration with production-grade workspace membership checking via central DB + workspace settings.toml reading. Binary size projection remains ~28–34 MB, well under the 50 MB cap. Integration with five agentic harnesses (Claude Code, Codex, Cursor, Gemini, OpenCode) fully end-to-end; all harness MCP config + rules-file sync operations atomic with clash detection (HarnessClash exit 19); per-project harness overrides respected; workspace-scoped cascade teardown wired end-to-end; harness module framework is data-driven and extensible.*

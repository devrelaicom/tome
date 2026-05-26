# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-26 (Phase 4 US5.a complete; 894+ tests across 122+ suites; v0.4.0 release)

## Databases & Data Stores

### Local SQLite Index

| Service | Type | Purpose | Location |
|---------|------|---------|----------|
| SQLite 3 | Embedded relational DB | Local skill index — metadata, embeddings, reranker scores, workspace bindings, project bindings, summary cache state | Global: `${XDG_DATA_HOME}/tome/index.db` (WAL mode); Workspace: `${WORKSPACE}/.tome/index.db` (Phase 3 Foundational F1); schema in `src/index/schema.rs` |

### Connection Patterns

- **Statically linked**: `rusqlite` with `bundled` feature — no system SQLite dependency.
- **Concurrency model**: Single advisory lockfile (`index.lock` — global or workspace-scoped) serialises writes; WAL mode allows readers during writes; MCP server uses read-only open per FR-056; Phase 4 US4: summary regeneration writes `generated_at` + hash under advisory lock for atomicity; US5.a: doctor queries workspace_projects for binding state without taking lock (read-only).
- **ORM/Query builder**: Direct SQL via `rusqlite` — prepared statements, parameterised queries.
- **Migration approach**: Forward-only migrations under advisory lock in `src/index/migrations.rs`; Phase 4 F1 introduces schema v2 with `workspace_catalogs` (F11 live) + `workspace_projects` (US1 live) tables; Phase 4 US4: extends meta table with summariser model identity tracking (name, version, last-known digest); US5.a: binding queries enrich doctor report with ProjectBindingState (marker path, mtime for drift detection).

### Cache Structure

- **Catalog cache**: Each remote catalog source content-addressed by `sha256(url)` in `${XDG_DATA_HOME}/tome/catalogs/<sha256>/` (global) or `${WORKSPACE}/.tome/catalogs/<sha256>/` (workspace) — Git working tree, refreshed on `tome catalog update`. Multiple scopes can reference the same URL; shared via reference-count tracking — deleted only when no scope references it (Phase 3 / US3); Phase 4 F11: enrolment moved to `workspace_catalogs` junction table (sole source of truth per FR-360); US5.a: doctor detects orphaned catalog clones and suggests repair via `--fix`.
- **Model cache**: Downloaded model ONNX artefacts (embedder, reranker) + GGUF artefacts (summariser) stored in `${XDG_DATA_HOME}/tome/models/<model-name>/` (global, shared across scopes) with per-model `manifest.json` (strict JSON, `#[serde(deny_unknown_fields)]`); Phase 4 US4: summariser model (Qwen2.5-0.5B-Instruct GGUF, ~400 MB) downloaded alongside embedder/reranker; `tome models list --verify` validates all three via SHA-256; doctor reports summariser state; US5.a: doctor subsystem classifies summariser as Ok/Missing/Corrupt/Drift; drift detected via meta table identity mismatch.
- **Workspace summary cache**: Per-workspace `[summaries]` table in `${WORKSPACE}/.tome/settings.toml` with `short_summary`, `long_summary`, `generated_at` (RFC 3339 datetime literal), and `content_hash` (SHA-256 of input plugin list for invalidation detection); Phase 4 US4: regenerated on triggers (plugin enable/disable/reindex/catalog update) or explicit `tome workspace regen-summary`; forward-progress semantics: binding remains committed even if summarisation fails (exit 24 with partial state); US5.a: doctor detects cache staleness via content-hash mismatch → Degraded classification.
- **Atomic writes**: `tempfile` crate (rename-based) prevents corruption on SIGINT; workspace `init` uses `tempfile::Builder::tempdir_in(workspace_root)` for POSIX-atomic staging-to-final rename (Phase 3 / US2); Phase 4 US4: workspace `regen-summary` uses `toml_edit` to read/modify/write `[summaries]` table atomically; US5.a: doctor `--fix` repairs atomically (settings writes under advisory lock, staging-rename pattern for `.tome.tmp.*` cleanup).

### Workspace Registry (Phase 3 / US2, load-bearing in Phase 3 / US3, extended Phase 4 US2–US5)

- **File**: `${XDG_DATA_HOME}/tome/workspaces.txt` — opt-in (never created unless explicitly requested)
- **Format**: Line-delimited absolute paths to workspace roots; dedupe by exact-path match and canonicalize
- **Size cap**: 1 MiB; entry cap 10k (Phase 3 Polish hardening); no NUL or `..` path traversal sequences
- **Semantics**: Informational in US2; load-bearing in US3 — tracks which workspaces have been initialized via `--inherit-global`. US3 `catalog remove` consults this file to enumerate all scopes for reference-counting. Phase 4 US4: `workspace list` discovers workspaces via this optional registry (absent registry = global only); summary cache state tracked per workspace via central DB; US5.a: doctor enumerates workspaces via registry for comprehensive per-workspace binding checks.
- **Usage**: Client harnesses can read this file to discover initialized workspaces; Tome treats absence as "no workspace scopes" (global scope only); Phase 4 US1: unused by binding algorithm (central DB is source of truth for workspace_projects); Phase 4 US4: still discovery-only for workspace list (central DB is authority for which workspaces actually exist); US5.a: doctor walks registry to enumerate all workspaces (optional file may be absent — fallback is global only).

---

## Authentication & Authorization

Phase 1–4 has no explicit application-layer authentication. Phase 3 / US1 MCP server is stdio-based (embedding in harness provides transport-level security). Phase 4 extends scope to workspace/project/harness level without auth changes. Phase 4 US5.a adds binding drift detection (no auth change, informational only).

- **Git operations**: Inherit system SSH keys and HTTP credential helpers (if configured in `~/.gitconfig`).
- **Hugging Face model downloads**: No API key required; public HTTPS URLs freely accessible (MODEL_REGISTRY pinned to MIT-licensed BGE variants + Apache-2.0 Qwen2.5).
- **Plugin manifest ownership**: File system permissions validate catalog ownership (email field in `tome-catalog.toml` is metadata only).
- **Workspace ownership**: Implicitly owned by the user who runs `tome workspace init`; no explicit permission model.
- **Project binding ownership**: Implicitly owned by the user who runs `tome workspace use`; binding record stored in central DB + `.tome/` marker created in project root with restricted permissions (no explicit ACL); Phase 4 US4: binding identity verified during summary regeneration (skipped if binding mismatch detected); US5.a: doctor detects orphaned bindings (marker present but DB record missing or vice versa) and suggests recovery or cleanup via `--fix`.
- **Workspace removal**: `--force` required when workspace has bound projects (FR-409); cascade teardown via `teardown_integration_for_project` removes harness-specific MCP config + rules-file entries; Phase 4 US4: summary cache deleted along with workspace settings; US5.a: doctor reports workspace with bound projects as advisory (not an error, just info).
- **Workspace rename**: Requires workspace to have no bound projects without `--force` (FR-410 enforces semantic constraint); atomic marker relocation via staging (Phase 4 US2); Phase 4 US4: binding identity verified during summary regeneration, harness sync respects rename identity in workspace settings; US5.a: doctor detects rename-stale markers (binding record mtime newer than marker mtime → drift).
- **Workspace regen-summary**: User runs `tome workspace regen-summary [<name>]` from any context (CLI-only, not MCP-accessible); regeneration happens under advisory lock with forward-progress semantics (binding committed even on summariser failure); US5.a: doctor detects summary cache staleness via content-hash comparison.
- **Credential scrubbing**: All Git stderr and model download error chains pass through `scrub_credentials()` before logging (principle XIII; extended to HF URLs and MCP log fields; Phase 4 US4: project path scrubbing in harness rules-file block insertion); US5.a: doctor error logs scrub project paths + workspace paths.
- **MCP server identity** (Phase 3 / US1): Identified by `server_info { name: "tome", version: "0.x" }` in the MCP handshake; no per-call authentication; US5.a: MCP startup validates query length cap (4096 chars) — over-length rejected with `code: query_too_long`.
- **Doctor read-only access** (Phase 3 / US4, extended Phase 4 US4 and US5.a): Diagnostics are read-only; repairs (`--fix`) require interactive confirmation; Phase 4 US4: extended to summariser state + summary cache freshness detection; US5.a: extended to binding state, binding-rules-copy drift, per-harness rules + MCP config state; confirmation per subsystem.
- **Harness config access** (Phase 4 US1–US5, unchanged US5.a): Direct filesystem access to harness-owned `.mcp.json` / `.mcp.toml` files; no permission model beyond OS-level file permissions; US5.a: doctor detects UserOwned MCP config (modification detected by harness-module-specific parsing, Tome doesn't claim ownership); `--fix --force` can override (lands in US5.b).

---

## External APIs

### First-Party APIs

- `commands::query::pipeline(args, deps) -> Result<QueryOutcome, TomeError>` — silent compute path reused by MCP `search_skills` tool (Phase 3 / US1.b); Phase 4 US5.a: input length validated (4096 chars max) before dispatch.
- `workspace::binding::bind_project(project_root, workspace_name, deps)` — project-to-workspace binding orchestrator (Phase 4 US1.a); US5.a: doctor detects stale binding (marker mtime vs DB record).
- `workspace::list::list(paths) -> Result<Vec<WorkspaceInfo>, TomeError>` — workspace discovery via opt-in registry (Phase 4 US2.a); US5.a: includes binding count per workspace in report (distinct from doctor—just summary).
- `workspace::rename::rename(old_name, new_name, paths) -> Result<RenameOutcome, TomeError>` — atomic workspace marker relocation with harness marker presence check (Phase 4 US2.a); US5.a: doctor verifies rename didn't leave stale marker links.
- `workspace::regen_summary::regen(name, summariser, paths) -> Result<RegenSummaryOutcome, TomeError>` — summary regeneration via configured summariser (Phase 4 US2.c, fully wired Phase 4 US4.a with `LlamaSummariser`); US5.a: doctor detects cache staleness.
- `workspace::sync::sync_for_project_root(project_root, scope, deps) -> Result<SyncOutcome, TomeError>` — harness MCP config + rules-file syncer (Phase 4 US3 complete; unchanged in US4 and US5.a).
- `workspace::remove::remove(name, force, paths, home, scope)` — 5-step cascade per FR-405: harness teardown, marker removal, DB cleanup, workspace dir removal, catalog refcount check; Phase 4 US4: summary cache deleted during step 1 (workspace settings deletion); US5.a: cascades binding cleanup (workspace_projects rows for all bound projects) via same lock.
- `summarise::LlamaSummariser::new(model_path) -> Result<Self, TomeError>` — initialiser for production summariser (Phase 4 US4.a; returns early error if model not found or corrupt); US5.a: doctor can call this in verify mode to detect model issues.
- `summarise::regenerate_for_trigger(workspace_name, deps) -> Result<SummariserOutput, TomeError>` — automatic summary regeneration triggered by plugin/catalog mutations (Phase 4 US4.b wired in lifecycle); Phase 4 US4.d-1 consolidates summary cache length checks (SHORT_MAX_CHARS / LONG_MAX_CHARS) and triggers; US5.a: triggers fire on binding changes (project bound/unbound → invalidate summary cache).
- `doctor::assemble_report(scope, paths, home, verify) -> Result<DoctorReport, TomeError>` — silent compute path for diagnosis (Phase 3 / US4.a, extended Phase 4 US5.a); returns report with five Phase 4 subsystems (project_binding, summariser, effective_harness_list, harness_rules, harness_mcp); no advisory lock taken — reads snapshot; Phase 4 US5: new library helpers `doctor::binding::check_binding()` + `doctor::harness_integration::check_harness_integration()`.
- `doctor::fixes::apply(&mut report, paths, scope) -> Result<usize, TomeError>` — repair dispatcher (Phase 3 / US4.b, extended Phase 4 US4 + US5.a); US5.a: repairs include binding-rules-copy sync, per-harness rules + MCP re-sync (coalesced per project), orphan `.tome.tmp.*` cleanup (1-hour mtime gate).

### Third-Party APIs

#### Hugging Face Model Registry

| Provider | Purpose | SDK/Client | Configuration |
|----------|---------|------------|---------------|
| Hugging Face (`huggingface.co`) | ONNX + GGUF model downloads (embedder, reranker, summariser) | `reqwest::blocking` (direct HTTPS) | `src/embedding/registry.rs` — `MODEL_REGISTRY` (compile-time constants); `src/summarise/registry.rs` — `SUMMARISER_NAME`, `SUMMARISER_VERSION`, `SUMMARISER_SHA256` |

**Details**:
- **Embedder**: `bge-small-en-v1.5` INT8 (~66 MB) from quantised variant
- **Reranker**: `bge-reranker-base` INT8 (~280 MB) from `onnx-community/bge-reranker-base-ONNX` (source moved Phase 3 slice 1)
- **Summariser** (Phase 4 US4): `qwen2.5-0.5b-instruct` GGUF (~400 MB, Q4_K_M quantisation) from `Qwen/Qwen2.5-0.5B-Instruct-GGUF`; Phase 4 F6 adds placeholder with all-zero checksum guard (downloads refused until real digest landed); Phase 4 US4.a ships production `LlamaSummariser`; Phase 4 US4.d-1 confirms real SHA-256 pinned: `74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db` (491,400,032 bytes); US5.a: doctor can verify presence + checksum independently.
- **Integrity**: Pinned SHA-256 + size_bytes verified post-download; no checksum endpoint (hashes are real upstream digests verified at Phase 3 slice 1 start and Phase 4 US4.d-1 for summariser); US5.a: doctor can re-verify all three artefacts with `--verify` flag (compares against registry + recorded meta).
- **Network**: HTTPS only via `rustls-tls` (no system OpenSSL); US5.a: doctor never triggers downloads (read-only diagnostics).
- **Failure modes**: Network error → `TomeError::Io` (exit 7); checksum mismatch → `TomeError::ModelChecksumMismatch` (exit 32); corrupted registry → `TomeError::ModelCorrupt` (exit 31); missing model → `TomeError::ModelMissing` (exit 30); embedder drift → `TomeError::EmbedderNameDrift` (exit 41); summariser model issues → `TomeError::SummariserFailure` (exit 24) with `SummariserFailureKind::{OutputEmpty, BackendInitFailed, InferenceFailure, ModelNotFound, ModelCorrupt}`; Phase 4 US4: adds exit 24 for summarisation pipeline failures; Phase 4 US1–US3: adds harness-specific failure codes (13–20 per FR-592); US5.a: doctor repair exit codes depend on fix type (75 if unrecoverable issues remain).
- **Explicit management**: Phase 6 wires `tome models {download,list,remove}` to manage artefacts; `tome models list --verify` validates SHA-256 per-file via `embedding::download::sha256_file()` + `summarise::download::verify_summariser_model()` (Phase 4 US4); US5.a: doctor can validate via `--verify` without triggering downloads.
- **Status visibility**: Phase 8 adds `tome status [--verify]` for read-only audit without triggering downloads; Phase 4 US4: extends to include summariser model identity + state; US5.a: doctor extends status reporting to include all three models per Subsystem enum.
- **Doctor integration** (Phase 3 / US4, extended Phase 4 US4 and US5.a): `tome doctor` reports model health (all three: embedder, reranker, summariser) with optional repair via `--fix`; Phase 4 US4: added `check_summariser` diagnostic helper; US5.a: model diagnostics promoted to Subsystem enum variants (Embedder, Reranker, Summariser); repair via `fixes::apply` (no-op in US5.a pending US5.b handlers).
- **Scope**: Models are global (shared across all workspaces); downloaded to `${XDG_DATA_HOME}/tome/models/` regardless of active scope; US5.a: doctor reports global model state, not per-scope.
- **Cache invalidation** (Phase 4 US4): Summary cache content-hash compared to current input; if hash matches, cached summaries reused; no re-download needed unless model is corrupted or missing; US5.a: doctor detects summariser identity drift in meta table (recorded name/version mismatch against registry).

---

## Message Queues & Event Systems

None. Phase 3 / US1 MCP server is stdio-based (single request/response); Phase 4 adds no async event infrastructure; US5.a adds no event system. Phase 3 Polish: explicit SIGTERM handler for graceful shutdown (Unix-only) with 5s timeout.

---

## Caching

| Service | Purpose | TTL / Eviction | Configuration |
|---------|---------|----------------|-----------------|
| Filesystem (XDG) | Catalog Git working trees | Explicit `tome catalog remove` (user-managed); persistent; shared across scopes via refcount (Phase 3 / US3) | Global: `${XDG_DATA_HOME}/tome/catalogs/`; Workspace: `${WORKSPACE}/.tome/catalogs/` (Phase 3 Foundational F1); same URL reused — clone deleted only when all scopes drop it; Phase 3 Polish: orphan clones reported by doctor; US5.a: doctor can suggest repair via `--fix` refcount validation. |
| Filesystem (XDG) | Downloaded model artefacts (all three: embedder, reranker, summariser) | Explicit `tome models remove` (user-managed); persistent | `${XDG_DATA_HOME}/tome/models/` — one dir per model with manifest + ONNX/GGUF files; shared across all scopes (global); Phase 4 US4: summariser model stored alongside embedder/reranker; `tome status --verify` validates all three; US5.a: doctor validates via `--verify`, classifies state (Ok/Missing/Corrupt/Drift). |
| Workspace Settings TOML | Cached workspace summaries | Explicit `tome workspace regen-summary` (user-managed); invalidation on plugin enable/disable/reindex/catalog update (automatic triggers); persistent until `workspace remove` | `${WORKSPACE}/.tome/settings.toml` — `[summaries]` table with short + long + generated_at timestamp + content_hash (SHA-256 of input list); Phase 4 US4: content-hash detects stale cache; automatic invalidation baked into lifecycle triggers; US5.a: doctor detects staleness, suggests regeneration via `--fix`. |
| Filesystem | Orphaned staging dirs | Explicit cleanup via `tome doctor --fix`; 1-hour mtime gate (stale staging > 1h old assumed abandoned) | `${workspace_root}/.tome.tmp.*` staging dirs from failed atomic writes; US5.a: doctor `--fix` cleans up orphaned staging > 1h old via `filetime`-assisted backdating in tests. |

No TTL-based eviction. Explicit user commands for cleanup (principle VI). Phase 3 Polish: doctor provides advisory cleanup candidates; US5.a extends cleanup to per-harness integration state via `--fix`.

---

## Monitoring & Observability

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| Structured logging (via `tracing`) | Diagnostic tracing to stderr (CLI) and JSON-lines to file (MCP server) | CLI: `RUST_LOG` or `TOME_LOG` environment variables; independent of `--json` stdout. MCP: JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log` per `contracts/log-format.md`; 10 MiB rotation cap; stderr reserved for fatal startup errors only (FR-222); Phase 3 Polish: custom `ContractEventFormat` emits contract-pinned field names (`ts`, `level`, `target`, `msg`); log file 0600 mode (Unix-only); credential scrubbing on `workspace_path` and `error_message` fields; Phase 4 US4: summarisation progress logged at debug level (LLM inference steps, model load time); US5.a: doctor subsystem diagnostics logged at debug/warn (binding drift, harness integration state). |
| Exit codes | Scriptable error handling | 30+ enumerated codes: Phase 2 baseline + Phase 3 additions + Phase 4 F1–F11 (13–20 per FR-592 for harness/settings/summariser) + Phase 4 US4 (24 for `SummariserFailure`) + US5.a (75 for `DoctorFixNotSafe` when `--fix` ran but unrecoverable issues remain); documented in `contracts/exit-codes.md` and incremental updates. |
| Status checks | Per-subsystem health via `tome status` | Phase 8 — models (all three), index, drift state with lazy `--verify` flag; Phase 4 US4: extended to summariser model state (present/valid/corrupt), summary cache state (present/stale/fresh), workspace binding status, settings composition validation; US5.a: unchanged (status stays focused on quick read-only checks; doctor handles broader diagnostics). |
| Doctor diagnostics | Subsystem health assessment + harness discovery + repair | Phase 3 / US4 — `tome doctor [--fix]` reports model/index/workspace/drift/harness health; Phase 3 Polish: orphan clone detection, registry status; Phase 4 US4: extended to summariser state (present/corrupt/drift), summary cache state (present/stale), settings composition + workspace binding drift (orphaned markers, stale DB records), harness MCP config consistency checks; US5.a: added five new subsystems per Subsystem enum (project_binding, binding_rules_copy, summariser, harness_rules, harness_mcp); classification per FR-561 (Unhealthy on binding broken, Degraded on binding-rules mismatch + harness config issues). |

---

## File Storage

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| XDG-compliant filesystem | Configuration, catalogs, models, index, logs, workspace directories | Global: `${XDG_CONFIG_HOME}/tome/settings.toml` (Phase 4 F8+), `${XDG_DATA_HOME}/tome/catalogs/<sha>/`, `${XDG_DATA_HOME}/tome/models/` (embedder/reranker/summariser), `${XDG_DATA_HOME}/tome/index.db`, `${XDG_STATE_HOME}/tome/mcp.log`, `${XDG_DATA_HOME}/tome/workspaces.txt` (opt-in); Workspace: `${WORKSPACE}/.tome/config.toml`, `${WORKSPACE}/.tome/settings.toml` (includes `[summaries]` table per US4), `${WORKSPACE}/.tome/catalogs/<sha>/`, `${WORKSPACE}/.tome/index.db`, `${WORKSPACE}/.tome/RULES.md` (skeleton Phase 4 US1, real content Phase 4 US4); Project: `${PROJECT}/.tome/config.toml` (Phase 4 US1, binding marker), `${PROJECT}/.tome/RULES.md` (Phase 4 US4, project context + rules for summarisation), per-harness `${PROJECT}/.{claude,codex,etc}/.rules.md` / `.mcp.json` / `.mcp.toml` (Phase 4 US3, read-modify-write atomically); Phase 4 US4: workspace settings now houses summary cache alongside general workspace config; US5.a: doctor detects binding-rules-copy drift (project `.tome/RULES.md` vs workspace `.tome/RULES.md` byte mismatch). |

---

## Email & Notifications

None in Phase 1–5.

---

## Agentic Coding Harness Integration (Phase 3 / US4, extended Phase 4 F1–F11 + US1–US5)

Phase 3 / US4 adds harness discovery; Phase 4 Foundational extends to harness-specific MCP config integration and settings composition; Phase 4 US1–US3 adds project binding + rules-file + MCP config sync + full settings composition + harness sync algorithm; Phase 4 US4 adds workspace summary integration (independent of harness sync — summary regeneration is CLI-only, not MCP-triggered); Phase 4 US5.a adds harness integration diagnostics.

| Harness | Install Location | Discovery | Purpose | Phase 4 US5.a Additions |
|---------|------------------|-----------|---------|------------------------|
| Claude Code | `~/.claude` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection → Phase 4 US3 reads/validates `.mcp.json` for sync | First-party harness | US5.a: doctor detects rules/MCP config drift + UserOwned state; FR-563 mtime walk validates no mutation during `--fix` |
| Codex | `~/.codex` | Existence only → Phase 4 F1+ extends to `.mcp.toml` inspection → Phase 4 US3 reads/validates `.mcp.toml` for sync | Third-party harness | US5.a: doctor detects rules/MCP config drift + UserOwned state |
| Cursor | `~/.cursor` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection → Phase 4 US3 reads/validates `.mcp.json` for sync | Third-party harness | US5.a: doctor detects rules file (standalone via RulesFileStrategy) presence + content match |
| Gemini CLI | `~/.gemini` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection → Phase 4 US3 reads/validates `.mcp.json` for sync | Third-party harness | US5.a: doctor detects rules/MCP config drift + UserOwned state |
| OpenCode | `~/.opencode` | Existence only → Phase 4 F1+ extends to `.mcp.json` inspection → Phase 4 US3 reads/validates `.mcp.json` for sync | Third-party harness | US5.a: doctor detects rules/MCP config drift + UserOwned state |

**Discovery semantics (research §R-7, FR-167, Phase 4 R-9/R-11, extended US5.a):**
- **Probe timing**: At startup, `doctor`, or harness commands; scans `$HOME` for each harness directory; Phase 4 US1–US4: also called during binding workflow to sync harness config, during workspace removal to cascade teardown, and during workspace summary regeneration to validate harness presence (but summary regeneration doesn't update harness config); US5.a: doctor probes harnesses to detect per-harness integration state (rules-file + MCP config).
- **Scope**: Fixed compile-time list in `src/harness/mod.rs::SUPPORTED_HARNESSES` — no dynamic discovery.
- **Content read**: Phase 3 — existence only; Phase 4 F1–F11 — extends to harness-specific MCP config inspection; Phase 4 US3: harness sync reads MCP config, validates, modifies, writes back atomically; Phase 4 US4: unchanged (summary regeneration doesn't modify harness config); US5.a: doctor reads rules-file + MCP config per harness, classifies as Ok/Drift/Broken/UserOwned.
- **Report shape**: `HarnessPresence { name, path, present: bool }` per contract; Phase 4: extended with optional `mcp_config_present: bool`; Phase 4 US5.a: extended to `HarnessSubsystemReport` with rules-file state + MCP-config state (per HarnessModule trait method composition snapshot).
- **Update path**: Harness module trait dispatch (`HarnessModule` impl per harness); code change + contract update (not user-configurable); Phase 4 US5.a: doctor `--fix` repairs via per-harness sync (coalesced once per project per C-M3 fix); US5.b lands dispatch handlers + per-harness teardown on workspace removal.

**Summary integration with harnesses (Phase 4 US4, unchanged US5.a):**
- **MCP tool description**: `search_skills` tool description includes the cached short summary read once at server startup (FR-425); over-length emits `tracing::warn!` but never refuses to start; Phase 4 US4: summary read from workspace-scoped cache if workspace detected, else uses static fallback description; US5.a: unchanged (summary integration independent of doctor diagnostics).
- **Project RULES.md**: Long summary written to `${PROJECT}/.tome/RULES.md` on `tome workspace regen-summary` or automatic triggers; harness-agnostic (not specific to any one harness); workspace-scoped (applies to all projects bound to that workspace); US5.a: doctor detects binding-rules-copy drift by comparing project `.tome/RULES.md` to workspace `.tome/RULES.md` byte-to-byte.
- **Independent of harness sync**: Summary regeneration doesn't modify harness MCP config or rules-file markers; summary cache lives in workspace settings separate from harness-specific state; US5.a: doctor subsystems for summary cache + harness config are separate (independent repair paths).

---

## Settings Composition (Phase 4 F1–F11 + US1–US5)

Phase 4 Foundational F8 introduces multi-level settings composition framework reused by both CLI and MCP server. Phase 4 US1 extends with project-level config. Phase 4 US2 uses composition resolver in workspace cascade teardown. Phase 4 US3 fully wires resolver into harness sync algorithm. Phase 4 US4 uses resolver to determine summariser eligibility (unused in this phase, but infrastructure ready); Phase 4 US5.a: doctor reports composition snapshot (effective harness list + composition validation).

| Level | Location | Purpose | Precedence | Phase |
|-------|----------|---------|-----------|-------|
| **Project** | `${PROJECT}/.tome/config.toml` (strict, Tome-owned) + `.tome/RULES.md` | Project-specific settings: harness overrides, tool preferences; project context + rules for summarisation (lenient frontmatter) | Highest | F1 (skeleton), US1 (binding record), US3 (read in harness sync), US4 (RULES.md read for summarisation context), US5.a (binding state reported by doctor) |
| **Workspace** | `${WORKSPACE}/.tome/settings.toml` (strict, Tome-owned) | Workspace-local enablement, harness overrides, tool preferences, summary cache (`[summaries]` table) | Medium | F8, US3 (fully wired in resolver), US4 (summary cache stored here), US5.a (doctor detects cache staleness) |
| **Global** | `${XDG_CONFIG_HOME}/tome/settings.toml` (strict, Tome-owned) | User-wide defaults, catalog list, model preferences | Lowest | F8, US3 (fully wired in resolver), US5.a (doctor reports as fallback composition level) |

**Composition resolver** (`src/settings/resolver.rs`, extended US5.a):
- Loads all applicable layers (project optional; workspace optional; global required)
- Merges in precedence order (project > workspace > global) following FR-441 (stop at first declaring `harnesses` key)
- Returns unified `ComposedSettings` struct with effective harness list (or empty if opted-out); US5.a: EffectiveHarnessList serialisable for JSON doctor output
- Validation per layer (Tome-owned → strict `deny_unknown_fields`)
- Phase 4 US3: `ScopeProvider` trait defines workspace membership checks; production `CentralDbScopeProvider` queries central DB + reads workspace settings.toml
- Phase 4 US5.a: doctor calls resolver and includes composition snapshot in report (effective list per scope); used to determine which harnesses should be checked per HarnessSubsystemReport

**Harness-specific MCP config** (Phase 4 F8+, fully wired US3, extended diagnostics US5.a):
- Location: `~/.harness/.mcp.json` or `.mcp.toml` (e.g., `~/.claude/.mcp.json`, `~/.codex/.mcp.toml`)
- Format: JSON array of tool descriptors (most harnesses) or TOML table (Codex) per MCP spec + harness-specific convention
- Edit pattern: Phase 4 US3 harness sync reads, parses into struct, validates, modifies via `HarnessModule::sync`, writes back with comment/order preservation; atomic via `NamedTempFile::persist`; Phase 4 US4: unchanged (summary regeneration doesn't touch harness config); US5.a: doctor reads config state per harness via `HarnessModule::mcp_config_read` + `is_tome_owned`, classifies as Ok/Drift/Broken/UserOwned
- Integration: Doctor reports harness MCP config state + consistency per subsystem (HarnessMcp); Phase 4 US5.a: doctor suggests repair via `--fix` (coalesced per project per C-M3 rule) — implementation lands in US5.b
- MCP server: tool descriptions optional include summary (FR-425 allows over-length with warning); US5.a: unchanged (MCP server composition logic unchanged by doctor subsystem reporting)

---

## Project Binding Integration (Phase 4 US1, extended Phase 4 US2–US5)

Phase 4 / US1 introduces `tome workspace use` — one-way binding from a project directory to a workspace. Phase 4 / US2 extends `workspace remove` to cascade harness teardown and binding cleanup. Phase 4 / US3 wires harness sync to respect binding identity. Phase 4 / US4 validates binding during summary regeneration (skipped if mismatch detected); Phase 4 / US5.a: doctor detects binding drift + rules-copy state.

| Aspect | Details |
|--------|---------|
| **Binding semantics** | User runs `tome workspace use <workspace-name>` from a project directory; Tome records the binding in the central DB (`workspace_projects` table, PK on project_path) and creates an atomic `${PROJECT}/.tome/` marker directory; Phase 4 US2: `workspace remove` cascades by reading workspace_projects + harness compose list, tearing down per-harness entries, removing markers; Phase 4 US3: harness sync respects binding identity and uses composition resolver to determine per-project harness list; Phase 4 US4: summary regeneration verifies binding matches current workspace context (skipped if mismatch detected with warning); US5.a: doctor detects binding drift (marker mtime vs DB record) + rules-copy state (project `.tome/RULES.md` content-equal to workspace `.tome/RULES.md`) |
| **Storage** | Central: `workspace_projects` table in `${XDG_DATA_HOME}/tome/index.db` (1:1 mapping project_path → workspace_id); Project-local: `${PROJECT}/.tome/config.toml` (contains workspace name for verification); Phase 4 US4: workspace-scoped settings now houses summary cache; binding identity validated before regeneration; US5.a: doctor compares project marker mtime to DB record for drift detection (US5.b drift repair updates marker mtime) |
| **Atomicity** | `bind_project` acquires advisory lock, UPSERTs DB row, lands marker dir via `tempfile::Builder::tempdir_in + rename`, releases lock; Phase 4 US3: harness sync acquires lock, reads composition/settings, calls per-harness sync methods, releases lock; Phase 4 US4: summary regeneration acquires lock, validates binding, reads project context + workspace settings, invokes summariser, writes `[summaries]` table atomically; US5.a: doctor queries binding state without lock (read-only) |
| **Discovery** | Doctor scans for orphaned markers (DB row absent, filesystem present) — advisory; scans for stale bindings (marker mtime outdated) — advisory; orphaned markers are recoverable via re-bind or cleanable via manual deletion; Phase 4 US4: workspace regen-summary skips project-level RULES.md reads if binding is missing or stale (logged at warn level); US5.a: ProjectBindingState in doctor report includes well-formedness + drift classification |
| **Scope inference** | When a project is bound, `Paths::resolve()` can return the project's workspace scope if the marker is present + DB record matches. CWD walk sequence: cwd → ancestors → found `.tome/` marker → verify binding in DB → return `Scope(workspace_name)`; Phase 4 US4: scope inference used by `workspace regen-summary` to determine which workspace's settings to update; US5.a: scope inference validates binding freshness (mtime check) — used by doctor to detect stale bindings |
| **CLI entry** | `tome workspace use [<workspace-name>] [--workspace <override>]` — new `WorkspaceCommand::Use` (Phase 4 US1.a); interactive selection if no workspace-name given; Phase 4 US2–US4: no new use changes (harness sync + composition resolver + summary regeneration handle project-specific context transparently); US5.a: unchanged (doctor is read-only, no CLI entry for binding management) |
| **Summary regeneration** | Phase 4 US4: `tome workspace regen-summary [<name>]` loads project `.tome/RULES.md` frontmatter + body (if binding present) as input context to summariser; summary written to workspace settings; binding identity validated before regeneration (skipped with warning if mismatch); US5.a: doctor detects stale binding (mtime drift) and rules-copy state — suggests `--fix` to sync rules-file from workspace (implementation lands US5.b) |
| **Failure modes** | Non-existent workspace → error; project already bound to different workspace → confirm + rebind; CWD not a project dir (no .git / pyproject.toml / etc.) → error; binding record stale (workspace deleted, marker orphaned) → doctor repair or manual cleanup; Phase 4 US4: project RULES.md missing/unparseable → warning, continue with minimal context; summariser failure → exit 24 with partial state (binding retained); unsupported harness in settings → exit 14; harness clash → exit 19; Phase 4 US2: workspace remove refuses without `--force` when bound projects exist; cascade per-step failures are logged but don't abort; US5.a: doctor classifies binding state (Healthy/Degraded/Unhealthy), suggests specific fixes per issue type (rules-copy sync, marker re-creation, orphan cleanup) |

---

## Workspace Scope Integration (Phase 3 / US2–US3, extended Phase 4 F1–F11 + US1–US5)

**Status:** Workspace info + init landed (Phase 3 / US2); scope-aware paths (Foundational F1); reference-counted catalog sharing (US3); project binding (US1); workspace lifecycle (US2); full settings composition + harness sync (US3); workspace summary caching (US4); diagnostic subsystem reporting (US5.a).

| Aspect | Details |
|--------|---------|
| **Scope types** | Global (default, uses XDG paths) or Workspace (per `.tome/` directory); resolved via `Paths::resolve()` which walks `cwd` up the tree looking for `.tome/` marker; Phase 4 US4: scope inference also used for workspace-scoped summary cache lookup; US5.a: scope inference validates binding freshness (mtime check) for diagnostic accuracy |
| **Path model** | Per-scope `Paths` accessor methods: `Paths::config_file_for(&Scope)`, etc. (Phase 3 Foundational F1); Phase 4 US4: workspace-scoped paths (settings file, summary cache) resolved consistently; US5.a: doctor uses scope-aware paths to enumerate per-scope subsystems |
| **Config location** | Global: `${XDG_CONFIG_HOME}/tome/settings.toml` (Phase 4 F8+); Workspace: `${WORKSPACE}/.tome/settings.toml` (Phase 4 F8+, includes `[summaries]` table per US4); Project: `${PROJECT}/.tome/config.toml` (Phase 4 US1, binding marker); Phase 4 US4: workspace settings.toml carries both general settings + `[summaries]` table; US5.a: doctor reports all three levels (project optional) |
| **Index location** | Global: `${XDG_DATA_HOME}/tome/index.db`; Workspace: `${WORKSPACE}/.tome/index.db` (same WAL + advisory lock model); Phase 4 US4: project-scoped queries use workspace-scoped index; schema meta table tracks summariser model identity; US5.a: doctor queries binding state from workspace-scoped index (if inside workspace) or global index (if global) |
| **Catalog cache location** | Global: `${XDG_DATA_HOME}/tome/catalogs/<sha>/`; Workspace: `${WORKSPACE}/.tome/catalogs/<sha>/`; Phase 4 US4: unchanged (summary regeneration doesn't depend on catalog state directly, only on enabled-plugin list); US5.a: doctor detects orphaned catalog clones and includes in suggested fixes |
| **Summary cache location** | Phase 4 US4: `${WORKSPACE}/.tome/settings.toml` — `[summaries]` table with short/long + generated_at (RFC 3339) + content_hash (SHA-256 of input list); workspace-scoped (applies to all projects bound to workspace); US5.a: doctor detects cache staleness via content-hash comparison + reports as Subsystem variant |
| **Info command** | `tome workspace info` (Phase 3 / US2.a) — read-only scope report; Phase 4 US4: no new changes to info output; US5.a: unchanged (info stays lightweight, doctor handles full diagnostics) |
| **Init command** | `tome workspace init [<path>] [--inherit-global] [--force]` (Phase 3 / US2.b) — atomic `.tome/` creation; Phase 4 US4: no new changes to init semantics (summary cache starts empty); US5.a: unchanged (init doesn't trigger doctor) |
| **List command** | `tome workspace list [--json]` (Phase 4 US2.a) — discover workspaces via opt-in registry; returns `Vec<WorkspaceListItem>` with name + root + binding count + summary cache state; Phase 4 US4: extended to show summary cache presence/staleness; US5.a: unchanged (list focuses on discovery, doctor focuses on diagnostics) |
| **Rename command** | `tome workspace rename <old> <new> [--force]` (Phase 4 US2.a) — atomic marker relocation via staging; requires no bound projects without `--force`; updates project marker + workspace settings + DB metadata; Phase 4 US4: summary cache preserved (namespace-independent, attached to workspace name, renamed atomically); US5.a: unchanged (rename stays lightweight, doctor detects post-rename drift via mtime) |
| **Regen-summary command** | `tome workspace regen-summary [<name>]` (Phase 4 US2.c, fully wired Phase 4 US4.a) — regenerate `[summaries]` table via configured summariser; loads project context (`.tome/RULES.md` frontmatter if binding present) as input; caches short/long summaries + generated_at timestamp + content_hash in workspace settings; automatic triggers wired in Phase 4 US4.b (enable/disable/reindex/catalog update); US5.a: doctor detects stale cache and suggests regeneration via `--fix` |
| **Remove command** | `tome workspace remove <name> [--force]` (Phase 4 US2.b) — 5-step cascade per FR-405: harness teardown (per-project composition-aware), marker removal, DB cleanup, workspace dir removal (includes deletion of workspace settings.toml which holds summary cache), catalog refcount check; Phase 4 US4: summary cache deleted as part of workspace settings cleanup; US5.a: doctor reports bound projects as advisory (not error, just info to help user decide `--force`) |
| **Registry file** | `${XDG_DATA_HOME}/tome/workspaces.txt` — opt-in; Phase 3 / US3 makes it load-bearing for refcount enumeration; Phase 4 US4: `workspace list` discovers workspaces via this optional registry; US5.a: doctor enumerates workspaces via registry (fallback to global if absent) for comprehensive subsystem checks |
| **CLI wiring** | `Command::Workspace(WorkspaceArgs)` + `WorkspaceCommand::{Info, Init, Use, List, Rename, RegenSummary, Remove}` (Phase 4 US1 adds `Use`, Phase 4 US2 adds `List/Rename/RegenSummary/Remove`); scope resolution integrated into all commands via `Paths::resolve()`; Phase 4 US4: `workspace regen-summary` driven by summariser output; automatic invalidation tied to lifecycle triggers; US5.a: no new CLI changes (doctor is separate command, called standalone or as `--fix` from doctor itself) |

---

## Schema Migration Integration (Phase 3 / US5, extended Phase 4 F1–F11 + US1–US5)

**Status:** Forward-migration framework (Phase 3 Foundational F7); integration test coverage (Phase 3 / US5); v2 schema (Phase 4 F1+); US1–US3 populate binding tables; US4 extends meta with summariser tracking; US5.a: no new migrations (structural schema stable).

| Aspect | Details |
|--------|---------|
| **Framework** | `src/index/migrations.rs` — `Migration` struct with function-pointer apply hooks; `apply_pending(conn, current, target)` three-arg signature; `MIGRATIONS_OVERRIDE` test-injection point |
| **Schema versions** | v0 (Phase 2 bootstrap), v1 (Phase 3 baseline), v2 (Phase 4 / F1 introduces `workspace_catalogs` + `workspace_projects` tables + meta enhancements); v2→v2.1 (Phase 4 US4 extends meta with summariser model identity) — structural-only, no data migration; US5.a: no new version (schema stable, diagnostics read meta as-is) |
| **Test coverage** | `tests/schema_migration_e2e.rs` — integration tests via synthetic-fixture injection; Phase 4 US4: v2→v2.1 migration passes (extends meta table with new optional columns for summariser identity); US5.a: unchanged (schema stays v2, diagnostic reads work against v2) |
| **Test fixtures** | `tests/common/mod.rs::write_index_db_with_schema_version` helper fabricates old-version DBs; US5.a: no new fixture generators |
| **Atomicity** | All migrations run under advisory lock; rollback on error; no partial state visible to readers; Phase 4 US4: workspace summary cache invalidation tracks model identity for drift detection (separate from migration framework); US5.a: doctor queries meta table safely (read-only, no locks) |
| **Version semantics** | Write-path checks schema version, emits `SchemaVersionTooNew` (exit 73) if too new; read-path retains legacy `SchemaTooNew` (exit 52) for backward compat; US5.a: doctor handles schema-too-new gracefully (surfaces as informational in report, doesn't crash) |
| **Production migrations** | Compile-time `MIGRATIONS` array (Phase 4 F1: v1→v2 introduces structural tables; Phase 4 US4: v2→v2.1 extends meta with summariser identity); US5.a: no new migrations (schema v2 is final for Phase 4) |
| **Doctor integration** | `tome doctor` can repair schema via `--fix`; Phase 4 US4: extended to validate summariser model identity against registry; summary cache freshness checks depend on recorded model digest; US5.a: doctor reads schema_version from meta, reports as advisory (not auto-fixed unless explicitly planned) |

---

## Index Schema Changes (Phase 4 / F1–F11 + US1–US5)

Phase 4 / F1 introduces schema v2 with structural-only changes. Phase 4 / US4 extends v2 to v2.1 adding summariser tracking. Phase 4 / US5.a: diagnostics stable on v2 schema.

### New/Extended Tables (v2 → v2.1)

| Table | Purpose | Load-bearing Phase | Phase 4 US5.a Changes |
|-------|---------|-------------------|----------------------|
| `workspace_catalogs` | Junction table: workspace scopes × catalog URLs; replaces `Config.catalogs` as sole source of truth per FR-360 | F11 (moved enrolment to table) | US5.a: unchanged; doctor checks for orphaned rows (catalog_url but no corresponding catalog dir) |
| `workspace_projects` | 1:1 binding: project_path → workspace_id; primary key on `project_path` alone (FR-598) | US1 (first real usage when binding a project) | US5.a: doctor queries binding state (project_path match), detects orphaned rows (DB record but no marker) or stale markers (marker exists but DB record missing) |
| `meta` (extended) | Schema metadata; Phase 3 carries `schema_version`, `summariser_name`, `summariser_version`; Phase 4 US4 adds optional `summariser_last_verified` (RFC 3339), `summariser_verified_digest` (hex SHA-256) | F1 (v2 baseline) | US5.a: doctor reads all meta fields, compares summariser identity to registry, detects drift (name/version/digest mismatch); **no new columns** (schema stable) |

### Primary Key Changes

- `workspace_projects.project_path`: Unique constraint (1:1 binding to one workspace)
- `workspace_catalogs`: Composite key on `(workspace_id, catalog_url)` for uniqueness across scopes

---

## Environment Variables

| Variable | Required | Purpose | Example | Updated Phase |
|----------|----------|---------|---------|---------------|
| `HOME` | Yes | Base directory for XDG path resolution and harness home probe | `/Users/aaronbassett` | — |
| `XDG_CONFIG_HOME` | No (defaults to `~/.config`) | Override config directory | `/opt/etc` | — |
| `XDG_DATA_HOME` | No (defaults to `~/.local/share`) | Override data directory (models, catalogs, index.db, workspaces.txt) | `/opt/var` | Phase 4 US4: summariser model stored here; summary cache stored in workspace settings.toml; US5.a: unchanged |
| `XDG_STATE_HOME` | No (defaults to `~/.local/state`) | Override state directory (MCP log) | `/opt/state` | Phase 3 Foundational F8; US5.a: unchanged |
| `TOME_LOG` | No | Custom log filter (overrides `RUST_LOG`) | `debug`, `info`, `tome=trace` | Phase 4 US4: includes summarisation progress, model verification; US5.a: includes doctor diagnostic logs at debug |
| `RUST_LOG` | No | Standard Rust log filter | `info`, `warn` | — |
| `NO_COLOR` | No | Disable coloured output (per CLICOLOR spec) | (presence enables) | Phase 4 US4: maintained for summary output; US5.a: maintained for doctor subsystem glyphs |
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
| **Error scrubbing** | Captured stderr passed through `scrub_credentials()` before logging — covers URLs, tokens, SSH keys, long hex strings (principle XIII); Phase 4 US4: extended to harness rules-file + summary regeneration error paths; US5.a: extended to doctor error logs (project paths, workspace paths) |

---

## Third-Party Manifest Parsing

| Format | Location | Strictness | Purpose |
|--------|----------|-----------|---------|
| `plugin.json` | Catalog plugin dirs | Lenient (unknown fields ignored) | Third-party plugin metadata (FR-013a boundary) |
| SKILL.md YAML frontmatter | Upstream plugin repos | Lenient (unknown fields ignored) | Third-party skill/agent/command/hook metadata |
| `tome-catalog.toml` | Catalog root | Strict (`deny_unknown_fields`) | Tome-owned manifest; rejects typos early |
| `.tome/config.toml` (workspace) | `${WORKSPACE}/.tome/` | Strict (`deny_unknown_fields`) | Workspace marker identity; created on init; US5.a: unchanged |
| `.tome/config.toml` (project) | `${PROJECT}/.tome/` | Strict (`deny_unknown_fields`) | Project binding identity; created on bind; read during summary regeneration to verify binding context; US5.a: doctor verifies binding matches DB record |
| `settings.toml` (workspace) | `${WORKSPACE}/.tome/settings.toml` | Strict (`deny_unknown_fields`) | Workspace-level settings + summary cache (`[summaries]` table); Phase 4 US4: `[summaries]` table carries short/long + generated_at + content_hash; cache invalidation checks content_hash against current input; US5.a: doctor detects staleness |
| `settings.toml` (global) | `${XDG_CONFIG_HOME}/tome/` | Strict (`deny_unknown_fields`) | User-wide settings; composition resolver queries this for fallback harness list; US5.a: doctor includes in composition snapshot |
| `.mcp.json` / `.mcp.toml` (harness) | `~/.harness/` | Lenient (parse per MCP spec) | Harness-owned MCP server config; Phase 4 US4: unchanged (summary regeneration doesn't touch harness config); US5.a: doctor reads and classifies state (Ok/Drift/Broken/UserOwned) |
| `.tome/RULES.md` frontmatter + body | Project root (Phase 4 US4) | YAML frontmatter (lenient) + Markdown body | Project context + rules for summarisation; auto-created on first bind; frontmatter loaded during summary regeneration as input context; body is project-specific prose; US5.a: doctor detects binding-rules-copy drift (byte comparison vs workspace version) |

---

## MCP Server Integration (Phase 3 / US1, hardened Phase 3 Polish, extended Phase 4 F1–F11 + US1–US5)

**Status:** Server loop + tool registration (Phase 3 / US1); Phase 4 / F1–F11 adds harness-specific config integration + extended error semantics; Phase 4 US1–US3: project binding + workspace lifecycle + harness sync + settings composition complete; Phase 4 US4: summary cache integration with tool descriptions (optional, lenient); Phase 4 US5.a: input length cap enforcement, diagnostics unchanged (MCP is read-only).

| Aspect | Details |
|--------|---------|
| **Protocol** | `rmcp` (1.x) — Model Context Protocol stdio server per `contracts/mcp-server.md` |
| **Runtime** | Single-threaded `tokio` backing `src/mcp/` (Phase 3 Foundational F8); scoped via `tests/sync_boundary.rs`; US5.a: unchanged |
| **Process model** | Stdio: stdin = MCP messages, stdout = MCP responses; stderr for fatal startup errors only (FR-222); SIGTERM handler (Unix-only) with 5s graceful-shutdown timeout; US5.a: unchanged |
| **Tools advertised** | Two: `search_skills` (semantic KNN + optional reranking, description optionally includes cached summary) and `get_skill` (retrieve skill detail by ID); Phase 4 US4: summary integration optional (over-length doesn't block startup); US5.a: `search_skills.query` enforces 4096-char max length (code: `query_too_long` on violation) |
| **Logging** | JSON-lines to `${XDG_STATE_HOME}/tome/mcp.log`; 10 MiB rotation; Phase 3 Polish: custom `ContractEventFormat` for contract-pinned field names; log file 0600 (Unix-only); credential scrubbing on `workspace_path` and `error_message` fields; Phase 4 US4: includes summary cache hit/miss logs at debug level; US5.a: includes query length validation logs at debug, no new error logs |
| **Pre-flight** | FR-110 startup pipeline (schema check → drift detect → SHA-256 verify on all three models → eager-load FastembedEmbedder → load workspace summary cache if binding present) scoped to `src/mcp/preflight.rs`; Phase 4 US4: extended to load summariser model identity from settings (doesn't eagerly load summariser unless needed); over-length summary description logged at warn, doesn't block startup; US5.a: unchanged (input length cap is handler-level, not startup) |
| **Tool integration** | Embedder loaded once at startup; reranker lazily on first ranking call; Phase 4 US4: summary cache loaded at startup (read-only); project scope inferred from binding if present, summary description optionally included in tool details; Phase 4 US4: tool handlers respect project-scoped harness list + workspace summary cache; US5.a: query length validated at handler entry, over-length returns MCP error |
| **Tool I/O schemas** | `#[derive(JsonSchema)]` from `schemars` crate per `contracts/mcp-tools.md`; US5.a: `MAX_QUERY_CHARS` constant exported so schema introspection tools can reference the boundary |
| **Index access** | Read-only; Phase 4 US4: also reads workspace summary cache from settings.toml (workspace-scoped, not index-scoped); US5.a: unchanged (MCP stays read-only, no doctor integration) |
| **Error handling** | Fatal startup errors (schema too new, drift, embedder load, summariser model missing) → stderr + log + exit 60 (`McpStartupFailed`) or 61 (`McpProtocolIo`); Phase 4 US4: summariser placeholder checksum → exit 31 (`ModelCorrupt`) at startup (prevents MCP server from launching); tool errors mapped to MCP error responses; US5.a: query-too-long tool error returns MCP error (no startup impact) |
| **Sync boundary** | All async/tokio strictly in `src/mcp/`; structural test `tests/sync_boundary.rs` enforces; Phase 4 US4: summariser (sync throughout) stays outside async; summary cache read at startup (before async reactor starts); US5.a: unchanged |
| **CLI entry** | `tome mcp` — new `Command::Mcp(McpArgs)` dispatched before tracing/ctrlc init (FR-221); Phase 4 US4: no new MCP entry points (summary regeneration is CLI-only, not MCP-triggered); US5.a: unchanged (doctor is separate CLI command, no MCP integration) |
| **Phase 4 US5.a changes** | Input length cap (4096 chars on search_skills.query) enforced at handler entry; no MCP API changes; query-too-long error surfaces in structured error envelope; constant exported for schema tools; no other MCP changes (summary integration remains optional, doctor diagnostics remain CLI-only) |

### Tool Details

#### `search_skills`

| Aspect | Details |
|--------|---------|
| **Purpose** | Semantic skill search: KNN embedding distance + optional reranking; tool description optionally includes cached workspace summary (per FR-425) |
| **Input** | `SearchSkillsInput { query, limit, force_strict, ... }` per `contracts/mcp-tools.md`; US5.a: query field strictly validated ≤ 4096 chars; over-length returns code: `query_too_long` |
| **Output** | `SearchSkillsOutput { skills, ... }` — each result includes ID, name, catalog, score, snippet; unchanged US5.a |
| **Handler** | `pub async fn handle(input, state)` in `src/mcp/tools/search_skills.rs`; US5.a: added length validation at handler entry |
| **Summary integration** | Phase 4 US4: tool description includes short summary if cached in workspace scope; over-length (>800 chars) emits `warn!` at startup, description still published (FR-425 allows over-length); Phase 4 US4: summary description helps harness understand workspace context without needing to query separately; US5.a: unchanged (query length cap is separate from summary integration) |
| **Reuse** | Delegates to `commands::query::pipeline(args, deps)` — silent compute path; respects project binding if present to restrict to project's workspace catalogs; respects composition-resolved harness list; US5.a: unchanged |
| **Reranker** | Lazily loaded; shared across calls; shared between tool requests + harness sync operations (single per-process instance via `OnceCell`); US5.a: unchanged |

#### `get_skill`

| Aspect | Details |
|--------|---------|
| **Purpose** | Retrieve single skill full detail by ID |
| **Input** | `GetSkillInput { id: String }` — `<catalog>/<plugin>/<skill-name>`; unchanged US5.a |
| **Output** | `GetSkillOutput { skill: Option<SkillDetail>, ... }`; unchanged US5.a |
| **Handler** | `pub async fn handle(input, state)` in `src/mcp/tools/get_skill.rs`; unchanged US5.a |
| **Query** | Read-only index lookup; Phase 4 US4: unchanged (summary integration is tool-description only, not skill-detail related); US5.a: unchanged |

---

## What Does NOT Belong Here

- Internal code architecture → ARCHITECTURE.md
- Testing infrastructure → TESTING.md
- Security policies → SECURITY.md
- Dependency versions → STACK.md

---

*This document maps external service dependencies and integration points in Tome at Phase 4 Foundational F1–F11 + US1–US5.a complete (v0.4.0 release). Phase 4 adds harness module abstraction with five concrete implementations, multi-level settings composition (fully wired), project binding infrastructure (workspace_projects table), workspace lifecycle (list/rename/sync/regen-summary/remove with atomic marker relocation), harness sync algorithm end-to-end, workspace summary caching with LLM inference (Qwen2.5-0.5B-Instruct via llama-cpp-2), and comprehensive diagnostic subsystem categorization. Phase 4 US5.a ships production `LlamaSummariser` with real SHA-256 pinned (2026-05-26), integrated into workspace summary regeneration with content-hash cache invalidation. Summary cache stored in workspace settings.toml with automatic triggers on plugin/catalog lifecycle mutations + explicit `regen-summary` command. US5.a extends `tome doctor` with typed `Subsystem` enum (byte-identical wire format to Phase 3), five new diagnostic subsystems (project_binding, binding_rules_copy, summariser, harness_rules, harness_mcp), and per-subsystem repair dispatch framework (implementation lands US5.b). MCP `search_skills` tool enforces 4096-char input length cap with structured error on violation. Binary size projection remains ~28–34 MB, well under the 50 MB cap. Test count: 490 → 894 across 64 → 122 suites. Integration with five agentic harnesses fully end-to-end with atomic MCP config + rules-file sync; per-project harness overrides respected; workspace-scoped cascade teardown complete; all three inference runtimes (embedder/reranker/summariser) coordinated under single model registry + status/doctor reporting; doctor detects binding drift + rules-copy state + per-harness integration issues; comprehensive workspace/project/harness diagnostics in single command.*

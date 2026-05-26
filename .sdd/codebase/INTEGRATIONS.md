# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-27 (Phase 5 / US1 shipped; MCP prompts capability + schema v3; collision tracking for prompt naming)

## Databases & Data Stores

### Local SQLite Index

| Service | Type | Purpose | Location |
|---------|------|---------|----------|
| SQLite 3 | Embedded relational DB | Local skill index — metadata, embeddings, reranker scores, workspace bindings, project bindings, enabled entries (skills + commands), diagnostic metadata | Global: `<home>/.tome/index.db` (WAL mode); schema v3 in `src/index/schema.rs` (Phase 5 F2) |

### Connection Patterns

- **Statically linked**: `rusqlite` with `bundled` feature — no system SQLite dependency.
- **Concurrency model**: Single advisory lockfile (`index.lock` — global or workspace-scoped) serialises writes; WAL mode allows readers during writes; MCP server uses read-only open per FR-056; Phase 5: `prompts/list` + `prompts/get` run read-only without taking lock.
- **ORM/Query builder**: Direct SQL via `rusqlite` — prepared statements, parameterised queries.
- **Migration approach**: Forward-only migrations under advisory lock in `src/index/migrations.rs`; Phase 5 F2 introduces schema v3 with unified `entries` table (replaces per-kind tables) + `kind` discriminator column; backfill defaults per contracts/schema-migration-p5.md.

### Cache Structure

- **Catalog cache**: Each remote catalog source content-addressed by `sha256(url)` in `<home>/.tome/catalogs/<sha256>/` — Git working tree, refreshed on `tome catalog update`. Multiple scopes can reference the same URL; shared via reference-count tracking — deleted only when no scope references it; Phase 5: unchanged (catalog sync independent of prompts).
- **Model cache**: Downloaded model ONNX + GGUF artefacts stored in `<home>/.tome/models/`; all three models (embedder, reranker, summariser) are global, shared across scopes; Phase 5: unchanged.
- **Workspace summary cache**: Per-workspace `[summaries]` table in `<home>/.tome/workspaces/<name>/settings.toml`; Phase 5: unchanged (summary cache independent of prompts).
- **Plugin/Workspace data directories** (Phase 5 / US2–US3): Lazy-created persistent storage under `<home>/.tome/data/plugins/<catalog>_<plugin>/` and `<home>/.tome/data/workspaces/<workspace-name>/` on first `{{TOME_PLUGIN_DATA}}` / `{{TOME_WORKSPACE_DATA}}` variable reference during prompt execution; created atomically via `std::fs::create_dir_all` per `src/substitution/data_dir.rs`.
- **Prompt collision tracking** (Phase 5 / US1): In-memory collision detection via `src/mcp/prompt_collision.rs` — maps `<catalog>_<plugin>_<entry_name>` → `EntryIdentity` and detects collisions when building prompt router at MCP startup; collisions trigger suffix-counter resolution per contracts/mcp-prompts.md §Prompt naming algorithm.
- **Atomic writes**: `tempfile` crate (rename-based) prevents corruption on SIGINT; Phase 5: data-dir creation stays non-atomic (recoverable via re-run; no critical state inside data-dirs per design).

### Workspace Registry (Phase 3 / US2, extended Phase 4, Phase 5 unchanged)

- **File**: `<home>/.tome/workspaces.txt` — opt-in (never created unless explicitly requested)
- **Format**: Line-delimited absolute paths to workspace roots; dedupe by exact-path match and canonicalize
- **Size cap**: 1 MiB; entry cap 10k; no NUL or `..` path traversal sequences
- **Semantics**: Informational in discovery; load-bearing in reference-counting; Phase 5: unchanged (workspace registry independent of prompts).

---

## Authentication & Authorization

Phase 1–5 has no explicit application-layer authentication. Phase 3 / US1 MCP server is stdio-based (embedding in harness provides transport-level security). Phase 5 / US1 extends MCP with `prompts` capability — same stdio transport, no auth changes.

- **Git operations**: Inherit system SSH keys and HTTP credential helpers (if configured in `~/.gitconfig`).
- **Hugging Face model downloads**: No API key required; public HTTPS URLs freely accessible.
- **Plugin manifest ownership**: File system permissions validate catalog ownership (email field in `tome-catalog.toml` is metadata only).
- **Workspace ownership**: Implicitly owned by the user who runs `tome workspace init`; no explicit permission model.
- **Project binding ownership**: Implicitly owned by the user who runs `tome workspace use`; Phase 5: unchanged (binding independent of prompts).
- **Credential scrubbing**: All Git stderr and model download error chains pass through `scrub_credentials()` before logging; Phase 5: extended to substitution error messages (workspace/plugin data-dir paths scrubbed from error logs).
- **MCP server identity** (Phase 3 / US1, extended Phase 5 / US1): Identified by `server_info { name: "tome", version: "0.x" }` in the MCP handshake; Phase 5: extended with `PromptsCapability { listChanged: false }` indicating static prompt list (no runtime changes via MCP).
- **Prompt access** (Phase 5 / US1): All enabled-and-user-invocable entries from resolved workspace exposed as prompts via MCP; Claude Code harness (or other client) can invoke via `prompts/get`; substitution context built per-call with caller-supplied argument values per contracts/mcp-prompts.md.

---

## External APIs

### First-Party APIs

- `commands::query::pipeline(args, deps) -> Result<QueryOutcome, TomeError>` — silent compute path reused by MCP `search_skills` tool (Phase 3 / US1.b); Phase 5: unchanged.
- `mcp::prompts::PromptRouter` — MCP `prompts/list` + `prompts/get` handlers (Phase 5 / US1); router built dynamically from enabled-and-user-invocable entries; `list_all` returns `Vec<Prompt>` with name, description (truncated per `DESCRIPTION_MAX_CHARS` = 300), arguments; `get` loads entry body, renders via substitution pipeline, returns as MCP PromptMessage array per contracts/mcp-prompts.md.
- `plugin::identity::EntryKind` enum — Skill vs Command discriminator (Phase 5 F2); used in schema v3, prompt router filtering, collision tracking, error messages.
- `mcp::prompt_name::derive_name(catalog, plugin, entry_name, kind) -> String` — deterministic prompt naming per `<plugin>__<entry_name>` + collision-suffix algorithm (Phase 5 / US1).
- `mcp::prompt_collision::resolve_collisions(Vec<EntryIdentity>) -> CollisionRecord` — detects and resolves prompt name collisions at startup (Phase 5 / US1) per contracts/mcp-prompts.md §Collision handling.
- `substitution::render(body, context) -> Result<String, SubstitutionError>` — four-stage variable substitution pipeline (Phase 5 / US1–US3 wired progressively).
- `substitution::SubstitutionContext` / `SubstitutionContextBuilder` — per-prompt context with workspace, plugin, entry identity, argument values (Phase 5 F3 skeleton, US1 builder wiring, US2–US3 argument value population).

### Third-Party APIs

#### Hugging Face Model Registry

| Provider | Purpose | SDK/Client | Configuration |
|----------|---------|------------|---------------|
| Hugging Face (`huggingface.co`) | ONNX + GGUF model downloads (embedder, reranker, summariser) | `reqwest::blocking` (direct HTTPS) | `src/embedding/registry.rs` — `MODEL_REGISTRY` (compile-time constants); `src/summarise/registry.rs` — summariser identity |

**Details**:
- **Embedder**: `bge-small-en-v1.5` INT8 (~66 MB)
- **Reranker**: `bge-reranker-base` INT8 (~280 MB) from `onnx-community/bge-reranker-base-ONNX`
- **Summariser** (Phase 4 US4): `qwen2.5-0.5b-instruct` GGUF (~400 MB); SHA-256 pinned per US4.d-1: `74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db`; Phase 5: unchanged
- **Integrity**: Pinned SHA-256 + size_bytes verified post-download; no checksum endpoint (hashes are real upstream digests).
- **Network**: HTTPS only via `rustls-tls` (no system OpenSSL).
- **Failure modes**: Network error → `TomeError::Io` (exit 7); checksum mismatch → `TomeError::ModelChecksumMismatch` (exit 32); corrupted registry → `TomeError::ModelCorrupt` (exit 31); missing model → `TomeError::ModelMissing` (exit 30); Phase 5: adds `WorkspaceDataDirWriteFailed` (26) and `PluginDataDirWriteFailed` (9) for data-dir failures, `PromptArgumentMismatch` (28), `EntryNotFound` (27), `SubstitutionFailed` (29), `InvalidArgumentFrontmatter` (25) per contracts/exit-codes-p5.md.
- **Status visibility**: Phase 8 adds `tome status [--verify]` for read-only audit; Phase 4 US4: extended to include summariser model identity; Phase 5: unchanged (models remain orthogonal to prompts).
- **Doctor integration**: `tome doctor` reports model health with optional repair via `--fix`; Phase 5: unchanged (doctor independent of prompts capability).
- **Scope**: Models are global (shared across all workspaces); Phase 5: unchanged.
- **Cache invalidation**: Separate from prompt-related caching (Phase 4 US4 model cache, Phase 5 prompt-specific data-dirs are independent).

---

## Message Queues & Event Systems

None in Phase 1–5. Phase 5 adds no async event infrastructure.

---

## Caching

| Service | Purpose | TTL / Eviction | Configuration |
|---------|---------|----------------|-----------------|
| Filesystem (home) | Catalog Git working trees | Explicit `tome catalog remove` (user-managed); persistent; shared across scopes via refcount | `<home>/.tome/catalogs/` — same URL reused — clone deleted only when all scopes drop it |
| Filesystem (home) | Downloaded model artefacts (all three: embedder, reranker, summariser) | Explicit `tome models remove` (user-managed); persistent | `<home>/.tome/models/` — one dir per model with manifest + ONNX/GGUF files |
| Workspace Settings TOML | Cached workspace summaries | Explicit `tome workspace regen-summary`; invalidation on plugin enable/disable/reindex/catalog update (automatic triggers); persistent until `workspace remove` | `<home>/.tome/workspaces/<name>/settings.toml` — `[summaries]` table with short + long + generated_at + content_hash |
| Filesystem | Persistent plugin/workspace data | User-managed (explicit cleanup); persistent across prompt executions; Phase 5 lazy-creation | `<home>/.tome/data/plugins/<catalog>_<plugin>/` and `<home>/.tome/data/workspaces/<workspace_name>/` — created on first `{{TOME_PLUGIN_DATA}}` / `{{TOME_WORKSPACE_DATA}}` reference |
| Filesystem | Orphaned staging dirs | Explicit cleanup via `tome doctor --fix`; 1-hour mtime gate (stale staging > 1h old assumed abandoned) | `<workspace_root>/.tome.tmp.*` staging dirs from failed atomic writes |

No TTL-based eviction. Explicit user commands for cleanup (principle VI). Phase 5: plugin/workspace data-dirs have no automatic eviction (user-managed, similar to summary cache).

---

## Monitoring & Observability

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| Structured logging (via `tracing`) | Diagnostic tracing to stderr (CLI) and JSON-lines to file (MCP server) | CLI: `RUST_LOG` or `TOME_LOG` environment variables; independent of `--json` stdout. MCP: JSON-lines to `<home>/.tome/mcp.log` per `contracts/log-format.md`; 10 MiB rotation cap; Phase 5: includes prompt collision warnings, data-dir creation failures, substitution errors at appropriate levels |
| Exit codes | Scriptable error handling | 30+ enumerated codes; Phase 5 F1 adds 25–29 for data-dir creation (26, 9), argument mismatches (28), missing entries (27), substitution failures (29), invalid frontmatter (25) |
| Status checks | Per-subsystem health via `tome status` | Phase 8 — models (all three), index, drift state; Phase 5: unchanged (status independent of prompts capability) |
| Doctor diagnostics | Subsystem health assessment + harness discovery + repair | Phase 3 / US4 onward; Phase 5: unchanged (doctor independent of prompts) |

---

## File Storage

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| XDG-compliant filesystem | Configuration, catalogs, models, index, logs, workspace directories, plugin/workspace data | Global: `<home>/.tome/settings.toml`, `<home>/.tome/catalogs/<sha>/`, `<home>/.tome/models/`, `<home>/.tome/index.db`, `<home>/.tome/mcp.log`, `<home>/.tome/workspaces.txt` (opt-in), `<home>/.tome/data/plugins/` and `<home>/.tome/data/workspaces/` (Phase 5 new); Workspace: `<home>/.tome/workspaces/<name>/{settings.toml, RULES.md, index.db, catalogs/<sha>/}`; Project: `${PROJECT}/.tome/{config.toml, RULES.md}`; Phase 5: data-dir layout per substitution context (lazy-created, user-managed) |

---

## Email & Notifications

None in Phase 1–5.

---

## Agentic Coding Harness Integration (Phase 5 / US1 extends with prompts)

Phase 5 / US1 introduces MCP `prompts` capability exposing enabled-and-user-invocable entries (skills + commands) as slash-prompts with variable substitution.

| Harness | Prompts Support | Changes |
|---------|-----------------|---------|
| Claude Code | Via MCP stdio transport | Phase 5 / US1: prompts/list + prompts/get handlers wired; prompt router built from `<workspace>` enabled + `user_invocable: true` entries (defaults to `false` for non-slash use cases) |
| Codex, Cursor, Gemini CLI, OpenCode | Via MCP stdio transport (if integrated) | Phase 5: same prompt exposure as Claude Code (harness-agnostic, all route through same MCP server) |

**Prompt integration details (Phase 5 / US1)**:
- **Prompt naming**: Deterministic `<plugin>__<entry_name>` per `src/mcp/prompt_name.rs`; collision-suffix counter on hash collision (`<plugin>__<entry_name>__N`) per contracts/mcp-prompts.md §Prompt naming algorithm.
- **Prompt listing**: `prompts/list` returns all enabled + user-invocable entries; description field truncated to 300 chars per `DESCRIPTION_MAX_CHARS` (FR-066); `listChanged: false` (static at startup per rmcp contract).
- **Prompt execution**: `prompts/get` accepts prompt name + optional argument values; loads entry body, builds `SubstitutionContext` with workspace + plugin + entry identity + argument values (phase 5 / US3), renders via four-stage substitution pipeline, returns as PromptMessage array per contracts/mcp-prompts.md.
- **Variable substitution** (Phase 5 / US1–US3 progressive wiring): `{{TOME_*}}` built-ins (workspace name, plugin id, entry kind, data directories) → `{{$VAR}}` env passthrough → `$ARGUMENTS` / `$N` / `$NAME` Claude Code argument syntax; per contracts/substitution-engine.md.
- **Scope inference**: Prompt router built using resolved workspace's enabled entries; scope determined at MCP startup via cwd walk (or `--workspace` CLI override) per `src/workspace/resolution.rs`.
- **Data directory scaffolding** (Phase 5 / US2): `{{TOME_PLUGIN_DATA}}` and `{{TOME_WORKSPACE_DATA}}` variables trigger lazy directory creation in `<home>/.tome/data/` on first reference per `src/substitution/data_dir.rs`; created atomically via `std::fs::create_dir_all`.
- **CLI-only execution**: Prompt bodies execute via MCP prompt invocation; substitution runs once over the body per execution. Unlike skills/commands which can be triggered from CLI directly, prompts are MCP-only (US1 ships prompts capability only; CLI slash-commands land in Phase 5 / US4 as first-class CLI entries discriminated by `EntryKind`).

---

## Settings Composition (Phase 4 extended, Phase 5 unchanged)

Composition resolver determines which prompts are available (enabled entries only) + which substitution context to use (workspace-scoped).

| Level | Location | Purpose | Precedence | Phase |
|-------|----------|---------|-----------|-------|
| **Project** | `${PROJECT}/.tome/config.toml` (strict) + `.tome/RULES.md` | Project-specific settings + context; Phase 5: context may include project-scoped data for summaries, but prompts stay workspace-scoped (no project discrimination) | Highest | F1+ |
| **Workspace** | `<home>/.tome/workspaces/<name>/settings.toml` (strict) | Workspace-local enablement, harness overrides, tool preferences, summary cache, entry filters; Phase 5: entry filters (which skills/commands are user-invocable) + `arguments` field validation | Medium | F8+ |
| **Global** | `<home>/.tome/settings.toml` (strict) | User-wide defaults, catalog list, model preferences; Phase 5: unchanged | Lowest | F8+ |

**Phase 5 additions to composition**:
- **Entry filtering**: Workspace settings can declare `user_invocable: false` to opt out of prompt exposure (for CLI-only skills that don't fit slash-command pattern).
- **Argument schema**: Both skills and commands can declare `arguments` frontmatter field (list of names + optional descriptions); Phase 5 validates at parse-time per `src/plugin/frontmatter.rs`.

---

## Schema Version 3 (Phase 5 / F2)

**Structural change**: Unified `entries` table with `kind` discriminator column (replaces `skills` + `commands` tables).

| Aspect | Details |
|--------|---------|
| **Migration path** | v2 → v3: forward-only migration under advisory lock; backfill `kind = 'skill'` for all existing rows; `commands` table remains empty (future Phase 5 US4 CLI commands populate it); reads from either table work (backward-compat query semantics) per contracts/schema-migration-p5.md |
| **Discriminator** | `EntryKind` enum — Skill vs Command — stored as lowercased string literal ("skill" / "command") per database convention |
| **Collision tracking** | In-memory only (built at router startup); no persistence in schema (collision records are computed from enabled + user-invocable entries) |
| **Prompt routing** | Phase 5 / US1: reads from unified `entries` table; filters by `kind = 'skill'` and `enabled = 1` and scanned frontmatter `user_invocable` field; Phase 5 / US4: CLI commands land in same table with `kind = 'command'` discrimination |

---

## Substitution Engine (Phase 5 / F3 skeleton, US1–US3 wiring)

**Four-stage pipeline** (`src/substitution/mod.rs` main entry point: `render(body, context)`):

| Stage | Input | Processing | Output | Phase |
|-------|-------|-----------|--------|-------|
| **Built-ins** | `{{TOME_WORKSPACE_NAME}}`, `{{TOME_WORKSPACE_ID}}`, `{{TOME_PLUGIN_CATALOG}}`, `{{TOME_PLUGIN_ID}}`, `{{TOME_ENTRY_NAME}}`, `{{TOME_ENTRY_KIND}}`, `{{TOME_PLUGIN_DATA}}`, `{{TOME_WORKSPACE_DATA}}` | Via `src/substitution/builtins.rs` — substitution context lookups + lazy data-dir creation on first reference | Rendered string | F3 stub, US1–US3 progressive wiring |
| **Environment** | `{{$VAR}}` (any `$` prefix inside `{{...}}`) | Via `src/substitution/env.rs` — pass through `std::env::var` (fails if var unset) | Rendered string | F3 stub, US2 wiring |
| **Arguments** | `$ARGUMENTS`, `$N`, `$NAME` | Via `src/substitution/arguments.rs` — positional or named argument lookup from `ArgumentValues` enum | Rendered string | F3 stub, US3 wiring |
| **ARGUMENTS tail** | `$ARGUMENTS` only if not handled by prior stages | Reserved for future use per contracts; Phase 5 US1–US3 don't implement | Rendered string | Future |

**Regex compilation** (Phase 5 / US2–US3): Compiled regex patterns cached in `src/substitution/regex_sets.rs` via `std::sync::OnceLock` — one slot per stage (builtins / env / arguments patterns); populated on first render call; reused across all prompts.

**Data directory creation** (`src/substitution/data_dir.rs`):
- `{{TOME_PLUGIN_DATA}}` → `<home>/.tome/data/plugins/<catalog>_<plugin>/`
- `{{TOME_WORKSPACE_DATA}}` → `<home>/.tome/data/workspaces/<workspace_name>/`
- Created atomically via `std::fs::create_dir_all` on first reference; failure → `SubstitutionError::PluginDataDirCreationFailed` or `WorkspaceDataDirCreationFailed` (exit 26 / 9 respectively).

**Context building** (`src/substitution/context.rs` — `SubstitutionContextBuilder`):
- Per-prompt; workspace + plugin identity + entry name/kind + argument values provided by caller.
- `ArgumentValues` enum — `Positional(Vec<String>)` or `Named(HashMap<String, String>)` per frontmatter declaration.

---

## Project Binding Integration (Phase 4 US1, Phase 5 unchanged)

Phase 5 / US1 prompts are workspace-scoped (not project-scoped). Binding still used for:
- Workspace scope inference (`Paths::resolve()` cwd walk detects project marker).
- Project context for summary regeneration (Phase 4 US4 RULES.md body + frontmatter).
- Project-level harness MCP config (Phase 4 US1–US3).

Phase 5: Prompts don't access project context directly (only workspace + plugin + entry identity).

---

## Prompt Name Derivation (Phase 5 / US1)

Per `src/mcp/prompt_name.rs` + `src/mcp/prompt_collision.rs`:

| Input | Processing | Output |
|-------|-----------|--------|
| `(catalog, plugin, entry_name, kind)` | Format as `<plugin>__<entry_name>` per contracts/mcp-prompts.md §Prompt naming algorithm | e.g., `claude-code__ask__skill` |
| Collision detected (hash match on `<plugin>__<entry_name>`) | Append counter suffix (`__1`, `__2`, ...) | e.g., `claude-code__ask__skill__1` on collision |

Collision resolution runs at router startup; in-memory collision records track identity via `src/mcp/prompt_collision.rs::EntryIdentity { catalog, plugin, name, kind }`.

---

## What Does NOT Belong Here

- Internal code architecture → ARCHITECTURE.md
- Testing infrastructure → TESTING.md
- Security policies → SECURITY.md
- Dependency versions → STACK.md

---

*This document maps external service dependencies and integration points in Tome at Phase 5 / US1 (prompts capability shipped). Phase 5 / US1 introduces MCP `prompts` capability exposing enabled + user-invocable entries as slash-prompts; schema v3 migration with unified `entries` table + `kind` discriminator; substitution skeleton + prompt naming + collision tracking; 5 new exit codes (25–29 for data-dir/argument/entry failures). Zero new top-level dependencies (regex promoted from transitive to direct, no net change). Data-dir scaffolding (lazy-created plugin + workspace persistent storage) lives in `<home>/.tome/data/` per substitution context. Prompt router built dynamically at startup from workspace-enabled + user-invocable entries; `listChanged: false` indicates static list (changes only on plugin enable/disable/reindex). Substitution pipeline (4 stages) runs once per prompt execution. Phase 5 / US2–US3 wire argument substitution + environment variable expansion; Phase 5 / US4 ships CLI slash-commands as first-class entries alongside skills.*

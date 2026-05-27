# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-27 (Phase 5 / US4 shipped; new MCP tool `get_skill_info` + search_skills extensions; when_to_use indexing verified; `kind` field + description truncation + searchable filter in MCP results)

## Databases & Data Stores

### Local SQLite Index

| Service | Type | Purpose | Location |
|---------|------|---------|----------|
| SQLite 3 | Embedded relational DB | Local skill index — metadata, embeddings, reranker scores, workspace bindings, project bindings, enabled entries (skills + commands), diagnostic metadata, when_to_use text field (Phase 5 US4), searchable filter (Phase 5 US4) | Global: `<home>/.tome/index.db` (WAL mode); schema v3 in `src/index/schema.rs` (Phase 5 F2) |

### Connection Patterns

- **Statically linked**: `rusqlite` with `bundled` feature — no system SQLite dependency.
- **Concurrency model**: Single advisory lockfile (`index.lock` — global or workspace-scoped) serialises writes; WAL mode allows readers during writes; MCP server uses read-only open per FR-056; Phase 5 US2–US4: `prompts/list`, `prompts/get`, `search_skills`, `get_skill_info` run read-only without taking lock.
- **ORM/Query builder**: Direct SQL via `rusqlite` — prepared statements, parameterised queries.
- **Migration approach**: Forward-only migrations under advisory lock in `src/index/migrations.rs`; Phase 5 F2 introduces schema v3 with unified `entries` table (replaces per-kind tables) + `kind` discriminator column + `when_to_use` text field (indexed for KNN) + `searchable` boolean column; backfill defaults per contracts/schema-migration-p5.md; Phase 5 US4: no migration changes (schema stable).

### Cache Structure

- **Catalog cache**: Each remote catalog source content-addressed by `sha256(url)` in `<home>/.tome/catalogs/<sha256>/` — Git working tree, refreshed on `tome catalog update`. Multiple scopes can reference the same URL; shared via reference-count tracking — deleted only when no scope references it; Phase 5 US4: unchanged (catalog sync independent of discovery tools).
- **Model cache**: Downloaded model ONNX + GGUF artefacts stored in `<home>/.tome/models/`; all three models (embedder, reranker, summariser) are global, shared across scopes; Phase 5: unchanged.
- **Workspace summary cache**: Per-workspace `[summaries]` table in `<home>/.tome/workspaces/<name>/settings.toml`; Phase 5 US4: unchanged (summary cache independent of discovery tools).
- **Plugin/Workspace data directories** (Phase 5 / US2–US4): Lazy-created persistent storage under `<home>/.tome/data/plugins/<catalog>_<plugin>/` and `<home>/.tome/data/workspaces/<workspace-name>/` on first `{{TOME_PLUGIN_DATA}}` / `{{TOME_WORKSPACE_DATA}}` variable reference during prompt execution via `src/substitution/data_dir.rs`; created non-atomically via `std::fs::create_dir_all` per design (recoverable via re-run); failure → `PluginDataDirWriteFailed` (9) or `WorkspaceDataDirWriteFailed` (26).
- **Prompt collision tracking** (Phase 5 / US1): In-memory collision detection via `src/mcp/prompt_collision.rs` — maps `<catalog>_<plugin>_<entry_name>` → `EntryIdentity` and detects collisions when building prompt router at MCP startup; collisions trigger suffix-counter resolution per contracts/mcp-prompts.md §Prompt naming algorithm.
- **Atomic writes**: `tempfile` crate (rename-based) prevents corruption on SIGINT; Phase 5 US2–US4: data-dir creation stays non-atomic (recoverable via re-run; no critical state inside data-dirs per design).

### Workspace Registry (Phase 3 / US2, extended Phase 4, Phase 5 US2 with relocation)

- **File**: `<home>/.tome/workspaces.txt` — opt-in (never created unless explicitly requested)
- **Format**: Line-delimited absolute paths to workspace roots; dedupe by exact-path match and canonicalize
- **Size cap**: 1 MiB; entry cap 10k; no NUL or `..` path traversal sequences
- **Semantics**: Informational in discovery; load-bearing in reference-counting
- **Phase 5 US2 relocation**: `tome workspace rename <old> <new>` updates all bound project markers via `toml_edit` surgical edits (rewrites `[bound_workspace] workspace = "<old>"` → `workspace = "<new>"` per project marker) without touching the registry itself; relocation happens atomically per project under `index.lock`; Phase 5 US4: unchanged

---

## Authentication & Authorization

Phase 1–5 has no explicit application-layer authentication. Phase 3 / US1 MCP server is stdio-based (embedding in harness provides transport-level security). Phase 5 / US1 extends MCP with `prompts` capability — same stdio transport, no auth changes. Phase 5 / US2–US3 add no auth changes (substitution is caller-supplied, no external validation). Phase 5 / US4 adds discovery tools (search_skills, get_skill_info) — same MCP transport, no auth changes.

- **Git operations**: Inherit system SSH keys and HTTP credential helpers (if configured in `~/.gitconfig`).
- **Hugging Face model downloads**: No API key required; public HTTPS URLs freely accessible.
- **Plugin manifest ownership**: File system permissions validate catalog ownership (email field in `tome-catalog.toml` is metadata only).
- **Workspace ownership**: Implicitly owned by the user who runs `tome workspace init`; Phase 5 US2–US4: rename is workspace-scoped (re-binds all projects atomically); no permission model change.
- **Project binding ownership**: Implicitly owned by the user who runs `tome workspace use`; Phase 5 US2–US4: relocation happens via marker surgical edit (no binding-level auth change).
- **Credential scrubbing**: All Git stderr and model download error chains pass through `scrub_credentials()` before logging; Phase 5 US2–US4: extended to substitution error messages (workspace/plugin data-dir paths scrubbed from error logs; env var names in `{{$VARNAME}}` failures are logged but the values are not).
- **MCP server identity** (Phase 3 / US1, extended Phase 5 / US1): Identified by `server_info { name: "tome", version: "0.x" }` in the MCP handshake; Phase 5: extended with `PromptsCapability { listChanged: false }` indicating static prompt list (no runtime changes via MCP); Phase 5 US4: discovery tools accessible via same MCP transport (no auth boundary change).
- **Prompt access** (Phase 5 / US1–US4): All enabled-and-user-invocable entries from resolved workspace exposed as prompts via MCP; Claude Code harness (or other client) can invoke via `prompts/get`; substitution context built per-call with caller-supplied argument values + environment variable visibility per contracts/mcp-prompts.md; Phase 5 US3: argument values validated against declared schema (coerced per caller-supplied format; unknown fields rejected with PromptArgumentMismatch exit 28); Phase 5 US4: unchanged (discovery tools are read-only, no substitution).
- **Discovery access** (Phase 5 / US4): `search_skills` and `get_skill_info` tools are read-only queries over the local index; no caller authentication boundary (inherited from MCP transport).

---

## External APIs

### First-Party APIs

- `commands::query::pipeline(args, deps) -> Result<QueryOutcome, TomeError>` — silent compute path reused by MCP `search_skills` tool (Phase 3 / US1.b); Phase 5 US4: unchanged (search_skills dispatches to this pipeline).
- `mcp::prompts::PromptRouter` — MCP `prompts/list` + `prompts/get` handlers (Phase 5 / US1); router built dynamically from enabled-and-user-invocable entries; `list_all` returns `Vec<Prompt>` with name, description (truncated per `DESCRIPTION_MAX_CHARS` = 300), arguments; `get` loads entry body, renders via substitution pipeline, returns as MCP PromptMessage array per contracts/mcp-prompts.md; Phase 5 US3: `get` accepts caller-supplied `arguments` field (Object or String), validates + coerces per declared schema; Phase 5 US4: unchanged (discovery tools independent of prompts).
- `mcp::tools::search_skills::handle(state, input) -> Result<Output, McpError>` (Phase 5 / US4 extension): Executes KNN search + reranking; accepts `query` (search text), `top_k` (1..=100, default 10), `catalog` / `plugin` filters (optional), new `description_max_chars` parameter (default 150, sanity cap 100K per US4.d M-1); returns `Vec<SkillResult>` with catalog, plugin, name, **kind** (Skill/Command discriminator per EntryKind enum), **description** (truncated per `description_max_chars`), plugin_version, path, distance (cosine score); implementation enforces `searchable = 1` filter per FR-088 in `src/index/query.rs::knn`.
- `mcp::tools::get_skill_info::handle(state, input) -> Result<SkillInfo, McpError>` (Phase 5 / US4 new): Middle-tier discovery tool; inputs: catalog, plugin, name, kind (Skill default); returns full description (no truncation), optional `when_to_use` guidance text, plugin_version, `user_invocable` flag, optional `resources` enumeration (files + directories in parent); per `src/mcp/tools/get_skill_info.rs` implementing contracts/mcp-tools-p5.md § `get_skill_info`.
- `mcp::tools::get_skill` — full body fetch (Phase 3 / US1.c, unchanged in Phase 5 US4).
- `plugin::identity::EntryKind` enum — Skill vs Command discriminator (Phase 5 F2); used in schema v3, prompt router filtering, collision tracking, error messages; Phase 5 US4: exported to search_skills results + get_skill_info discriminator field.
- `mcp::prompt_name::derive_name(catalog, plugin, entry_name, kind) -> String` — deterministic prompt naming per `<plugin>__<entry_name>` + collision-suffix algorithm (Phase 5 / US1); Phase 5 US4: unchanged (discovery tools independent of naming).
- `mcp::prompt_collision::resolve_collisions(Vec<EntryIdentity>) -> CollisionRecord` — detects and resolves prompt name collisions at startup (Phase 5 / US1) per contracts/mcp-prompts.md §Collision handling; Phase 5 US4: unchanged (discovery tools read-only).
- `substitution::render(body, context) -> Result<String, SubstitutionError>` — four-stage variable substitution pipeline (Phase 5 / US1–US3); Phase 5 / US2: Stages 1–2 (built-ins + env) wired via unified COMBINED_RE single-sweep design; Phase 5 / US3: Stages 3–4 wired via COMBINED_RE extension (6 named capture groups); entire 4-stage pipeline scanned once per render call (enforces NFR-007 no-rescan invariant); Phase 5 US4: unchanged (discovery tools do not invoke substitution).
- `substitution::SubstitutionContext` / `SubstitutionContextBuilder` — per-prompt context with workspace, plugin, entry identity, argument values (Phase 5 F3 skeleton, US1 builder wiring, US2 env + data-dir wiring, US3 argument value coercion + validation); Phase 5 US4: unchanged.
- `substitution::regex_sets::combined_regex()` — lazy-compiled unified regex for all 4 stages ({{TOME_*}} built-ins + {{$VAR}} env + $ARGUMENTS/$N/$NAME patterns + bare $ARGUMENTS) via `src/substitution/regex_sets.rs`; compiled once per process on first `render()` call; enforces no-rescan invariant (NFR-007) by emitting resolved values directly to output without re-scanning; Phase 5 US4: unchanged.
- `index::query::knn(conn, workspace_name, query_vec, filters, top_k) -> Result<Vec<Candidate>, TomeError>` (Phase 5 US4 extension): Returns top-k enabled entries closest to query vector in cosine space; `Candidate` struct includes **kind** field (EntryKind); query enforces `searchable = 1` filter per FR-088; Phase 5 US4 only: filters by `kind = 'skill'` in search_skills context (per contracts/mcp-tools-p5.md §Searchable entries definition; commands excluded from KNN to reduce ranking noise).

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
- **Failure modes**: Network error → `TomeError::Io` (exit 7); checksum mismatch → `TomeError::ModelChecksumMismatch` (exit 32); corrupted registry → `TomeError::ModelCorrupt` (exit 31); missing model → `TomeError::ModelMissing` (exit 30); Phase 5 adds data-dir failures (26, 9), argument mismatches (28), missing entries (27), substitution failures (29), invalid frontmatter (25) per contracts/exit-codes-p5.md; Phase 5 US4: uses `EntryNotFound` (27) when discovery query returns no results.
- **Status visibility**: Phase 8 adds `tome status [--verify]` for read-only audit; Phase 4 US4: extended to include summariser model identity; Phase 5 US4: unchanged (models remain orthogonal to discovery tools).
- **Doctor integration**: `tome doctor` reports model health with optional repair via `--fix`; Phase 5 US4: unchanged (doctor independent of prompts/discovery).
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
| Filesystem | Persistent plugin/workspace data | User-managed (explicit cleanup); persistent across prompt executions; Phase 5 lazy-creation | `<home>/.tome/data/plugins/<catalog>_<plugin>/` and `<home>/.tome/data/workspaces/<workspace_name>/` — created on first `{{TOME_PLUGIN_DATA}}` / `{{TOME_WORKSPACE_DATA}}` reference via `src/substitution/data_dir.rs` (non-atomic, recoverable) |
| Filesystem | Orphaned staging dirs | Explicit cleanup via `tome doctor --fix`; 1-hour mtime gate (stale staging > 1h old assumed abandoned) | `<workspace_root>/.tome.tmp.*` staging dirs from failed atomic writes |
| In-memory | Compiled regex patterns for substitution | Per-process lifetime; lazily initialized on first `render()` call | `src/substitution/regex_sets.rs` — `COMBINED_RE` (all 4 stages via 6 named capture groups) via `OnceLock` slot; single-sweep design ensures all stages compiled together; Phase 5 US4: unchanged (discovery tools do not use substitution) |
| In-memory | Prompt router (enabled + user-invocable entries) | Built at MCP startup via `mcp::prompts::PromptRouter::new()`; static until next MCP restart | Per-MCP-instance router with collision tracking; Phase 5 US4: unchanged (discovery tools independent of prompt router) |
| In-memory | Embedder + Reranker models | Lazy-loaded on first use; cached in `FastembedEmbedder` singleton via `OnceLock` per `src/embedding/runtime.rs` | Per-process (shared across all tools + commands); Phase 5 US4: unchanged (search_skills + get_skill_info reuse same models as query command) |

No TTL-based eviction. Explicit user commands for cleanup (principle VI). Phase 5 US2–US4: plugin/workspace data-dirs have no automatic eviction (user-managed, similar to summary cache). Regex patterns are process-singletons (reused across all prompts); Phase 5 US3: single COMBINED_RE pattern covers all 4 substitution stages (prevents cross-stage exfiltration by structural design). Embedder + Reranker are process-singletons shared across MCP tools + CLI commands.

---

## Monitoring & Observability

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| Structured logging (via `tracing`) | Diagnostic tracing to stderr (CLI) and JSON-lines to file (MCP server) | CLI: `RUST_LOG` or `TOME_LOG` environment variables; independent of `--json` stdout. MCP: JSON-lines to `<home>/.tome/mcp.log` per `contracts/log-format.md`; 10 MiB rotation cap; Phase 5 US2–US3: includes substitution warnings (failed data-dir creation, env var resolution failures, argument count mismatches, coercion failures), collision detection warnings at debug level; Phase 5 US4: includes discovery tool status (search_skills query execution time, get_skill_info resource enumeration progress) at info/trace levels |
| Exit codes | Scriptable error handling | 30+ enumerated codes; Phase 5 F1 adds 25–29 for invalid frontmatter (25), data-dir creation (26, 9), missing entries (27), argument mismatches (28), substitution failures (29); Phase 5 US4: uses code 27 (EntryNotFound) when discovery query returns no matching entries |
| Status checks | Per-subsystem health via `tome status` | Phase 8 — models (all three), index, drift state; Phase 5 US4: unchanged (status independent of discovery tools) |
| Doctor diagnostics | Subsystem health assessment + harness discovery + repair | Phase 3 / US4 onward; Phase 5 US4: unchanged (doctor independent of discovery tools) |

---

## File Storage

| Service | Purpose | Configuration |
|---------|---------|---------------|
| XDG-compliant filesystem | Configuration, catalogs, models, index, logs, workspace directories, plugin/workspace data, project markers | Global: `<home>/.tome/settings.toml`, `<home>/.tome/catalogs/<sha>/`, `<home>/.tome/models/`, `<home>/.tome/index.db`, `<home>/.tome/mcp.log`, `<home>/.tome/workspaces.txt` (opt-in), `<home>/.tome/data/plugins/` and `<home>/.tome/data/workspaces/` (Phase 5 new); Workspace: `<home>/.tome/workspaces/<name>/{settings.toml, RULES.md, index.db, catalogs/<sha>/}`; Project: `${PROJECT}/.tome/{config.toml, RULES.md}` (binding marker with `[bound_workspace]` name field); Phase 5 US4: unchanged |

---

## Email & Notifications

None in Phase 1–5.

---

## Agentic Coding Harness Integration (Phase 5 / US1–US4 extends with prompts + substitution + discovery)

Phase 5 / US1 introduces MCP `prompts` capability exposing enabled-and-user-invocable entries (skills + commands) as slash-prompts with variable substitution. Phase 5 / US2 wires substitution Stage 1 (built-ins) + Stage 2 (env passthrough) with single-sweep COMBINED_RE design. Phase 5 / US3 completes the substitution pipeline with Stage 3 (argument patterns) + Stage 4 (ARGUMENTS footer). Phase 5 / US4 ships three-tool MCP discovery surface (search_skills with description truncation + kind field + searchable filter, get_skill with full body, get_skill_info with description + when_to_use + resource enumeration).

| Harness | Prompts Support | Discovery Support | Changes |
|---------|-----------------|-------------------|---------|
| Claude Code | Via MCP stdio transport | Via MCP stdio transport | Phase 5 / US1: prompts/list + prompts/get handlers wired; prompt router built from `<workspace>` enabled + `user_invocable: true` entries; Phase 5 / US4: search_skills + get_skill_info handlers added; search_skills truncates descriptions per `description_max_chars` (default 150); returns `kind` field (Skill/Command); filters to `searchable = 1` per FR-088; get_skill_info returns full description + when_to_use + resource enumeration (files + subdirs capped at 5 per level) |
| Codex, Cursor, Gemini CLI, OpenCode | Via MCP stdio transport (if integrated) | Via MCP stdio transport (if integrated) | Phase 5: same prompt exposure as Claude Code (harness-agnostic, all route through same MCP server); Phase 5 / US4: same discovery surface as Claude Code (same 3-tool MCP interface, same input/output schemas) |

**Prompt integration details (Phase 5 / US1–US3)**:
- **Prompt naming**: Deterministic `<plugin>__<entry_name>` per `src/mcp/prompt_name.rs`; collision-suffix counter on hash collision (`<plugin>__<entry_name>__N`) per contracts/mcp-prompts.md §Prompt naming algorithm.
- **Prompt listing**: `prompts/list` returns all enabled + user-invocable entries; description field truncated to 300 chars per `DESCRIPTION_MAX_CHARS` (FR-066); `listChanged: false` (static at startup per rmcp contract).
- **Prompt execution**: `prompts/get` accepts prompt name + optional `arguments` field (Object `{key: value}` or String for single arg); loads entry body, validates + coerces arguments per declared schema (Phase 5 US3), builds `SubstitutionContext` with workspace + plugin + entry identity + coerced argument values, renders via four-stage substitution pipeline, returns as PromptMessage array per contracts/mcp-prompts.md.
- **Variable substitution** (Phase 5 / US1–US3 progressive wiring): `{{TOME_*}}` built-ins (workspace name, plugin id, entry kind, data directories, wall-clock) + `{{$VAR}}` env passthrough via unified COMBINED_RE (US2) → `$ARGUMENTS` / `$N` / `$NAME` Claude Code argument syntax (US3); per contracts/substitution-engine.md; all 4 stages scanned once per render call (enforces NFR-007 no-rescan invariant).
- **Scope inference**: Prompt router built using resolved workspace's enabled entries; scope determined at MCP startup via cwd walk (or `--workspace` CLI override) per `src/workspace/resolution.rs`; Phase 5 US4: unchanged (discovery tools use same scope inference).
- **Data directory scaffolding** (Phase 5 / US2–US3): `{{TOME_PLUGIN_DATA}}` and `{{TOME_WORKSPACE_DATA}}` variables trigger lazy directory creation in `<home>/.tome/data/` on first reference per `src/substitution/data_dir.rs`; created non-atomically via `std::fs::create_dir_all` (recoverable via re-run); Phase 5 US4: unchanged (discovery tools do not trigger data-dir creation).
- **Argument schema** (Phase 5 / US3): Skill/command frontmatter declares `arguments` field (list of names + optional descriptions); Phase 5 US3 parses + validates at prompt-execution time; coercion per 6-row table from contracts/substitution-engine.md; Phase 5 US4: unchanged (discovery tools are read-only, no argument processing).
- **No-rescan invariant** (Phase 5 / US2–US3): Unified COMBINED_RE ensures all 4 stages scan once; resolved values never re-enter the scanner (per NFR-007 / FR-051; closes exfiltration vector where hostile plugin's `"version": "${TOME_ENV_GITHUB_TOKEN}"` could leak operator's env var; or caller's `arg = "${TOME_BUILTIN}"` could leak built-in values); Phase 5 US4: unchanged (discovery tools do not use substitution).
- **Clock injection** (Phase 5 / US2–US3): `{{TOME_CLOCK_*}}` variables hook into wall-clock via `src/substitution::current_clock()`; honours `SUBSTITUTION_CLOCK_OVERRIDE` slot for deterministic testing; Phase 5 US4: unchanged.
- **CLI-only execution**: Prompt bodies execute via MCP prompt invocation; substitution runs once over the body per execution. Unlike skills/commands which can be triggered from CLI directly, prompts are MCP-only (US1 ships prompts capability only; CLI slash-commands land in Phase 5 / US4 as first-class CLI entries discriminated by `EntryKind`).

**Discovery tool details (Phase 5 / US4)**:
- **search_skills**: KNN query + reranking over embedding space; accepts caller-supplied `description_max_chars` (default 150, sanity cap 100K per M-1) to fit result descriptions into agent token budgets; returns `kind` field (Skill/Command discriminator) per EntryKind enum; filters to enabled entries with `searchable = 1` per FR-088 (commands excluded from KNN search per US4.a contract definition — reduces ranking noise, improves agent focus); per `src/mcp/tools/search_skills.rs` implementing contracts/mcp-tools.md.
- **get_skill**: Full body fetch with metadata per Phase 3 / US1.c (unchanged in Phase 5).
- **get_skill_info** (new, Phase 5 US4): Middle-tier discovery returning full description (no truncation — that's search_skills' job per FR-082) + optional `when_to_use` guidance text + resource enumeration (top-level files + immediate subdirectories in parent, each capped at 5 entries with overflow collapsed to "and N more" sentinel per PER_DIRECTORY_CAP) + `user_invocable` flag; omits `resources` field for command-kind entries per FR-083; per `src/mcp/tools/get_skill_info.rs` implementing contracts/mcp-tools-p5.md § `get_skill_info`.

---

## Settings Composition (Phase 4 extended, Phase 5 US2–US4 with workspace rename relocation + argument schema + when_to_use)

Composition resolver determines which prompts are available (enabled entries only) + which substitution context to use (workspace-scoped). Phase 5 US2 adds workspace rename relocation of bound project markers. Phase 5 US3 adds argument schema validation. Phase 5 US4 adds when_to_use field to frontmatter + searchable filter to schema.

| Level | Location | Purpose | Precedence | Phase |
|-------|----------|---------|-----------|-------|
| **Project** | `${PROJECT}/.tome/config.toml` (strict) + `.tome/RULES.md` | Project-specific settings + context; Phase 5 US2: `[bound_workspace]` field name updated via toml_edit on `tome workspace rename`; Phase 5 US3: argument schema validation reads from skill/command frontmatter in enabled entries; Phase 5 US4: unchanged (when_to_use is workspace-scoped) | Highest | F1+ |
| **Workspace** | `<home>/.tome/workspaces/<name>/settings.toml` (strict) | Workspace-local enablement, harness overrides, tool preferences, summary cache, entry filters; Phase 5: entry filters (which skills/commands are user-invocable) + `arguments` field validation (parsed leniently; unknown subfields forward-compatible); Phase 5 US4: filter for `searchable` entries (when_to_use field contributes to embedding_text for KNN) | Medium | F8+ |
| **Global** | `<home>/.tome/settings.toml` (strict) | User-wide defaults, catalog list, model preferences; Phase 5: unchanged | Lowest | F8+ |

**Phase 5 additions to composition**:
- **Entry filtering**: Workspace settings can declare `user_invocable: false` to opt out of prompt exposure (for CLI-only skills that don't fit slash-command pattern).
- **Argument schema** (Phase 5 US3): Both skills and commands declare `arguments` frontmatter field (list of `{name: "string", description?: "string"}` objects per contracts/frontmatter-p5.md); Phase 5 US3 validates at prompt-execution time per coercion table.
- **Workspace rename relocation** (Phase 5 / US2–US3): `tome workspace rename <old> <new>` updates `[bound_workspace] workspace = "<new>"` in all bound project markers via toml_edit (surgical field edit preserving comments + order); relocation runs atomically per project under `index.lock`.
- **when_to_use field** (Phase 5 US4): Optional `when_to_use` frontmatter text (per contracts/frontmatter-p5.md) indexed in schema v3 and contributes to `embedding_text` for KNN reranking (returned by get_skill_info, not search_skills per FR-082/FR-083).
- **searchable filter** (Phase 5 US4): Boolean `searchable` column in unified entries table (defaults to 1 per migration); search_skills enforces `searchable = 1` filter per FR-088; get_skill_info returns all matched entries regardless of searchable status (discovery vs result ranking distinction).

---

## Schema Version 3 (Phase 5 / F2, extended Phase 5 US4)

**Structural change**: Unified `entries` table with `kind` discriminator column + `when_to_use` text field + `searchable` boolean filter (replaces `skills` + `commands` tables).

| Aspect | Details |
|--------|---------|
| **Migration path** | v2 → v3: forward-only migration under advisory lock; backfill `kind = 'skill'` for all existing rows; backfill `searchable = 1` for all rows; `commands` table remains empty (future Phase 5 US4 CLI commands populate it); reads from either table work (backward-compat query semantics) per contracts/schema-migration-p5.md |
| **Discriminator** | `EntryKind` enum — Skill vs Command — stored as lowercased string literal ("skill" / "command") per database convention |
| **when_to_use field** (Phase 5 US4) | Optional text field in unified `entries` table; indexed for KNN search per contracts/schema-migration-p5.md §embedding_text; returned by get_skill_info (full text, no truncation); contributes to embedding_text for search_skills KNN ranking |
| **searchable filter** (Phase 5 US4) | Boolean column (default 1) in unified `entries` table; enforced by search_skills query per FR-088 (filters to `searchable = 1` only); get_skill_info returns all matched entries regardless (discovery tool vs ranking distinction); per contracts/schema-migration-p5.md §Searchable entries definition |
| **Collision tracking** | In-memory only (built at router startup); no persistence in schema (collision records are computed from enabled + user-invocable entries) |
| **Prompt routing** | Phase 5 / US1: reads from unified `entries` table; filters by `kind = 'skill'` and `enabled = 1` and scanned frontmatter `user_invocable` field; Phase 5 / US3: argument schema validation uses scanned frontmatter `arguments` field; Phase 5 / US4: search_skills additionally filters by `searchable = 1`; get_skill_info returns all matched entries (discovery vs ranking); Phase 5 / US4: CLI commands land in same table with `kind = 'command'` discrimination |

---

## Substitution Engine (Phase 5 / F3 skeleton, US1 + US2 + US3 wiring, unchanged Phase 5 US4)

**Four-stage pipeline** (`src/substitution/mod.rs` main entry point: `render(body, context)`):

| Stage | Input | Processing | Output | Phase | Implementation |
|-------|-------|-----------|--------|-------|-----------------|
| **Built-ins** | `{{TOME_WORKSPACE_NAME}}`, `{{TOME_WORKSPACE_ID}}`, `{{TOME_PLUGIN_CATALOG}}`, `{{TOME_PLUGIN_ID}}`, `{{TOME_ENTRY_NAME}}`, `{{TOME_ENTRY_KIND}}`, `{{TOME_PLUGIN_DATA}}`, `{{TOME_WORKSPACE_DATA}}`, `{{TOME_CLOCK_*}}` | Via `src/substitution/builtins.rs` — substitution context lookups + lazy data-dir creation on first reference + wall-clock injection | Rendered string | F3 stub, US1–US3 wiring | `resolve_builtin(name, context, default)` matches against compile-time names; data-dir creation non-atomic via `std::fs::create_dir_all` |
| **Environment** | `{{$VAR}}` (any `$` prefix inside `{{...}}`) | Via `src/substitution/env.rs` — pass through `std::env::var` (falls back to default if unset) | Rendered string | F3 stub, US2–US3 wiring | `resolve_env(name, default)` via `std::env::var`; never errors (default is mandatory for env vars) |
| **Arguments** | `$ARGUMENTS`, `$ARGUMENTS[N]`, `$N`, `$NAME` (4 patterns per FR-040/FR-042/FR-043) | Via `src/substitution/arguments.rs` — positional or named argument lookup from coerced `ResolvedArguments` enum (caller supplied Single or Object; coerced per 6-row table per contracts/substitution-engine.md) | Rendered string | F3 stub, US3 wiring | `resolve_argument(token, values)` dispatches on `ResolvedArguments` variant; `shell_split` for Single+declared; `apply_arguments_match` for individual lookups; bare `$ARGUMENTS` joins positional with single space |
| **ARGUMENTS tail** | `$ARGUMENTS` only if zero Stage 3 refs found + caller supplied args | Per FR-044 — append footer `\nARGUMENTS: <value>` with blank-line separation when body has no argument substitutions (sentinel loop tracks replacements) | Rendered string | F3 stub, US3 wiring | `append_arguments_footer(body, body_has_stage3_refs, resolved_args)` checks sentinel + formats footer (Single verbatim OR Object positional values joined by space) |

**Regex compilation** (Phase 5 / US2–US3): Unified COMBINED_RE pattern compiles once per process on first `render()` call via `src/substitution/regex_sets.rs::combined_regex()` returning a static reference. Pattern covers all 4 stages in source order (leftmost-first alternation):
1. `\{\{TOME_(?:ENV_([A-Z0-9_]+)|([A-Z0-9_]+))(?::-(.*?))?\}\}` — Built-ins (Stages 1–2) with default fallback
2. `\$ARGUMENTS\[(\d+)\]` — Positional argument bracket syntax (Stage 3)
3. `\$ARGUMENTS` — Bare arguments (Stage 3 + Stage 4)
4. `\$(\d+)` — Positional argument numeric syntax (Stage 3)
5. `\$([a-z_][a-z0-9_]*)` — Named argument syntax (Stage 3)

Six named capture groups: `ENV_NAME_GROUP` / `BUILTIN_NAME_GROUP` / `DEFAULT_GROUP` / `ARG_INDEX_GROUP` / `POSITIONAL_GROUP` / `NAMED_GROUP`. All matches processed in one regex sweep; each resolved value emitted directly to output buffer and never re-scanned (enforces no-rescan invariant NFR-007 / FR-051).

**Data directory creation** (`src/substitution/data_dir.rs`):
- `{{TOME_PLUGIN_DATA}}` → `<home>/.tome/data/plugins/<catalog>_<plugin>/`
- `{{TOME_WORKSPACE_DATA}}` → `<home>/.tome/data/workspaces/<workspace_name>/`
- Created non-atomically via `std::fs::create_dir_all` on first reference; failure → `SubstitutionError::PluginDataDirCreationFailed` (exit 9) or `WorkspaceDataDirCreationFailed` (exit 26).

**Clock injection** (`src/substitution::current_clock()`):
- Returns `time::OffsetDateTime` for `{{TOME_CLOCK_*}}` variables
- Honours `SUBSTITUTION_CLOCK_OVERRIDE` slot for deterministic testing
- Mutex poison recovery via `PoisonError::into_inner` per Phase 4 / P5 pattern

**Context building** (`src/substitution/context.rs` — `SubstitutionContextBuilder`):
- Per-prompt; workspace + plugin identity + entry name/kind + coerced argument values provided by caller.
- `ArgumentValues` enum — `Positional(Vec<String>)` or `Named(HashMap<String, String>)` per frontmatter declaration.
- Phase 5 US3: caller supplies raw `arguments` (Object or String); `coerce_arguments` validates + transforms to `ResolvedArguments` (Positional or Named per coercion table).

**Argument coercion** (`src/substitution/arguments.rs::coerce_arguments`, Phase 5 US3):
- Single (string) + declared → shell-split into positional array per `shell_split` logic
- Single (string) + no declared → wrap as single-element positional array
- Object{key: value, ...} + declared (full) → validate all keys present in declared; lookup by name for named patterns, by position for numeric
- Object{key: value, ...} + declared (partial) → missing declared names filled with empty string
- Object{args: [...]} + no declared → treat as Single by extracting `args` array
- Object{unknown keys, no args key} + no declared → PromptArgumentMismatch (exit 28)

---

## Discovery Tool Implementation (Phase 5 / US4)

**search_skills** (`src/mcp/tools/search_skills.rs`):
- Inputs: `query` (natural language), `top_k` (1..=100, default 10), optional `catalog` / `plugin` filters, new `description_max_chars` (default 150, sanity cap 100K)
- Processing: Dispatches to `commands::query::pipeline` for KNN + reranking; enforces `searchable = 1` filter in `index::query::knn` per FR-088
- Output: `Vec<SkillResult>` with catalog, plugin, name, **kind** (EntryKind), **description** (truncated per input cap), plugin_version, path, distance
- Location: Wired in `src/mcp/server.rs` as `#[tool]` handler delegating to `search_skills::handle`
- Contract: contracts/mcp-tools.md (unchanged; Phase 5 US4 extends input/output per mcp-tools-p5.md addendum)

**get_skill_info** (`src/mcp/tools/get_skill_info.rs` — new Phase 5 US4):
- Inputs: `catalog`, `plugin`, `name`, optional `kind` (defaults to Skill per FR-084)
- Processing: Direct index lookup per `src/index/skills::find_by_identity`; walks parent directory to enumerate resources (top-level files + subdirs, each capped at 5 per `PER_DIRECTORY_CAP` with overflow collapsed to "and N more" sentinel)
- Output: `SkillInfo` struct with catalog, plugin, name, kind, path, full description (no truncation), optional `when_to_use` text, plugin_version, `user_invocable` flag, optional `resources` (omitted for command-kind per FR-083)
- Location: Wired in `src/mcp/server.rs` as `#[tool]` handler delegating to `get_skill_info::handle`
- Contract: contracts/mcp-tools-p5.md § `get_skill_info`

**get_skill** (`src/mcp/tools/get_skill.rs` — Phase 3 US1.c, unchanged in Phase 5 US4):
- Full entry body fetch with metadata
- Location: Wired in `src/mcp/server.rs`
- Contract: contracts/mcp-tools.md

---

## Project Binding Integration (Phase 4 US1, Phase 5 US2–US4 with relocation + argument schema)

Phase 5 / US1–US4 prompts are workspace-scoped (not project-scoped). Binding still used for:
- Workspace scope inference (`Paths::resolve()` cwd walk detects project marker).
- Project context for summary regeneration (Phase 4 US4 RULES.md body + frontmatter).
- Project-level harness MCP config (Phase 4 US1–US3).
- Phase 5 US2–US4: Bound workspace name relocation on `tome workspace rename` via toml_edit surgical edits (`[bound_workspace] workspace = "<new>"` per project marker).

Phase 5 US2–US4: Prompts don't access project context directly (only workspace + plugin + entry identity + coerced argument values). Workspace rename relocation updates all project markers atomically (one marker update per `index.lock` hold). Argument schema comes from skill/command frontmatter (workspace-scoped), not project context. Discovery tools (search_skills, get_skill_info) are workspace-scoped (same as prompts).

---

## Prompt Name Derivation (Phase 5 / US1)

Per `src/mcp/prompt_name.rs` + `src/mcp/prompt_collision.rs`:

| Input | Processing | Output |
|-------|-----------|--------|
| `(catalog, plugin, entry_name, kind)` | Format as `<plugin>__<entry_name>` per contracts/mcp-prompts.md §Prompt naming algorithm | e.g., `claude-code__ask__skill` |
| Collision detected (hash match on `<plugin>__<entry_name>`) | Append counter suffix (`__1`, `__2`, ...) | e.g., `claude-code__ask__skill__1` on collision |

Collision resolution runs at router startup; in-memory collision records track identity via `src/mcp/prompt_collision.rs::EntryIdentity { catalog, plugin, name, kind }`. Discovery tools (search_skills, get_skill_info) are independent of prompt naming (use catalog/plugin/name directly).

---

## What Does NOT Belong Here

- Internal code architecture → ARCHITECTURE.md
- Testing infrastructure → TESTING.md
- Security policies → SECURITY.md
- Dependency versions → STACK.md

---

*This document maps external service dependencies and integration points in Tome at Phase 5 / US4 (3-tool MCP discovery surface complete: search_skills + get_skill + get_skill_info). Phase 5 / US4 introduces: middle-tier discovery tool get_skill_info (T303–T308) with full descriptions (no truncation) + when_to_use guidance + resource enumeration (parent dir files + subdirs capped at 5 per level; overflow collapsed to "and N more" sentinel per PER_DIRECTORY_CAP); search_skills extended with description_max_chars input parameter (default 150, sanity cap 100K per M-1) + kind field in results (Skill/Command discriminator per EntryKind enum) + searchable filter (enforces searchable = 1 per FR-088); when_to_use field indexed in schema v3 (contributes to embedding_text per US4.b verification); kind field in schema v3 exported to discovery results. Zero new top-level dependencies. Binary size: **~27 MiB on macOS arm64**, well under the 50 MB cap. Data-dir scaffolding under `<home>/.tome/data/` is user-managed (non-atomic, recoverable). Prompt router built dynamically at startup from workspace-enabled + user-invocable entries; listChanged: false indicates static list (changes only on plugin enable/disable/reindex). Substitution pipeline (4 stages) runs once per prompt execution via single COMBINED_RE pass; all resolved values emitted directly to output without re-scanning (prevents cross-stage exfiltration per NFR-007 / FR-051). Discovery tools (search_skills + get_skill_info) are read-only queries over the local index; no substitution invoked. Phase 5 / US1 ships prompts capability (MCP exposure + prompt naming + collision tracking). Phase 5 / US2 ships substitution Stage 1 (built-ins) + Stage 2 (env passthrough). Phase 5 / US3 ships argument substitution Stage 3 + ARGUMENTS footer Stage 4, completing the 4-stage pipeline. Phase 5 / US4 ships three-tool MCP discovery surface (search_skills + get_skill + get_skill_info) with full scannability + description truncation + kind field + searchable filter + when_to_use guidance.*

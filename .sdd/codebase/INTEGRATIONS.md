# External Integrations

> **Purpose**: Document all external services, APIs, databases, and third-party integrations.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-27 (Phase 5 / US3 shipped; argument substitution Stage 3 + ARGUMENTS footer Stage 4 complete; 4-stage pipeline finished; single-sweep combined regex extended from 3 to 6 capture groups; NFR-007 no-rescan enforced structurally across all stages)

## Databases & Data Stores

### Local SQLite Index

| Service | Type | Purpose | Location |
|---------|------|---------|----------|
| SQLite 3 | Embedded relational DB | Local skill index — metadata, embeddings, reranker scores, workspace bindings, project bindings, enabled entries (skills + commands), diagnostic metadata | Global: `<home>/.tome/index.db` (WAL mode); schema v3 in `src/index/schema.rs` (Phase 5 F2) |

### Connection Patterns

- **Statically linked**: `rusqlite` with `bundled` feature — no system SQLite dependency.
- **Concurrency model**: Single advisory lockfile (`index.lock` — global or workspace-scoped) serialises writes; WAL mode allows readers during writes; MCP server uses read-only open per FR-056; Phase 5 US2–US3: `prompts/list` + `prompts/get` run read-only without taking lock.
- **ORM/Query builder**: Direct SQL via `rusqlite` — prepared statements, parameterised queries.
- **Migration approach**: Forward-only migrations under advisory lock in `src/index/migrations.rs`; Phase 5 F2 introduces schema v3 with unified `entries` table (replaces per-kind tables) + `kind` discriminator column; backfill defaults per contracts/schema-migration-p5.md.

### Cache Structure

- **Catalog cache**: Each remote catalog source content-addressed by `sha256(url)` in `<home>/.tome/catalogs/<sha256>/` — Git working tree, refreshed on `tome catalog update`. Multiple scopes can reference the same URL; shared via reference-count tracking — deleted only when no scope references it; Phase 5 US3: unchanged (catalog sync independent of prompts).
- **Model cache**: Downloaded model ONNX + GGUF artefacts stored in `<home>/.tome/models/`; all three models (embedder, reranker, summariser) are global, shared across scopes; Phase 5: unchanged.
- **Workspace summary cache**: Per-workspace `[summaries]` table in `<home>/.tome/workspaces/<name>/settings.toml`; Phase 5: unchanged (summary cache independent of prompts).
- **Plugin/Workspace data directories** (Phase 5 / US2–US3): Lazy-created persistent storage under `<home>/.tome/data/plugins/<catalog>_<plugin>/` and `<home>/.tome/data/workspaces/<workspace-name>/` on first `{{TOME_PLUGIN_DATA}}` / `{{TOME_WORKSPACE_DATA}}` variable reference during prompt execution via `src/substitution/data_dir.rs`; created non-atomically via `std::fs::create_dir_all` per design (recoverable via re-run); failure → `PluginDataDirWriteFailed` (9) or `WorkspaceDataDirWriteFailed` (26).
- **Prompt collision tracking** (Phase 5 / US1): In-memory collision detection via `src/mcp/prompt_collision.rs` — maps `<catalog>_<plugin>_<entry_name>` → `EntryIdentity` and detects collisions when building prompt router at MCP startup; collisions trigger suffix-counter resolution per contracts/mcp-prompts.md §Prompt naming algorithm.
- **Atomic writes**: `tempfile` crate (rename-based) prevents corruption on SIGINT; Phase 5 US2–US3: data-dir creation stays non-atomic (recoverable via re-run; no critical state inside data-dirs per design).

### Workspace Registry (Phase 3 / US2, extended Phase 4, Phase 5 US2 with relocation)

- **File**: `<home>/.tome/workspaces.txt` — opt-in (never created unless explicitly requested)
- **Format**: Line-delimited absolute paths to workspace roots; dedupe by exact-path match and canonicalize
- **Size cap**: 1 MiB; entry cap 10k; no NUL or `..` path traversal sequences
- **Semantics**: Informational in discovery; load-bearing in reference-counting
- **Phase 5 US2 relocation**: `tome workspace rename <old> <new>` updates all bound project markers via `toml_edit` surgical edits (rewrites `[bound_workspace] workspace = "<old>"` → `workspace = "<new>"` per project marker) without touching the registry itself; relocation happens atomically per project under `index.lock`; Phase 5 US3: unchanged

---

## Authentication & Authorization

Phase 1–5 has no explicit application-layer authentication. Phase 3 / US1 MCP server is stdio-based (embedding in harness provides transport-level security). Phase 5 / US1 extends MCP with `prompts` capability — same stdio transport, no auth changes. Phase 5 / US2–US3 add no auth changes (substitution is caller-supplied, no external validation).

- **Git operations**: Inherit system SSH keys and HTTP credential helpers (if configured in `~/.gitconfig`).
- **Hugging Face model downloads**: No API key required; public HTTPS URLs freely accessible.
- **Plugin manifest ownership**: File system permissions validate catalog ownership (email field in `tome-catalog.toml` is metadata only).
- **Workspace ownership**: Implicitly owned by the user who runs `tome workspace init`; Phase 5 US2–US3: rename is workspace-scoped (re-binds all projects atomically); no permission model change.
- **Project binding ownership**: Implicitly owned by the user who runs `tome workspace use`; Phase 5 US2–US3: relocation happens via marker surgical edit (no binding-level auth change).
- **Credential scrubbing**: All Git stderr and model download error chains pass through `scrub_credentials()` before logging; Phase 5 US2–US3: extended to substitution error messages (workspace/plugin data-dir paths scrubbed from error logs; env var names in `{{$VARNAME}}` failures are logged but the values are not).
- **MCP server identity** (Phase 3 / US1, extended Phase 5 / US1): Identified by `server_info { name: "tome", version: "0.x" }` in the MCP handshake; Phase 5: extended with `PromptsCapability { listChanged: false }` indicating static prompt list (no runtime changes via MCP).
- **Prompt access** (Phase 5 / US1–US3): All enabled-and-user-invocable entries from resolved workspace exposed as prompts via MCP; Claude Code harness (or other client) can invoke via `prompts/get`; substitution context built per-call with caller-supplied argument values + environment variable visibility per contracts/mcp-prompts.md; Phase 5 US3: argument values validated against declared schema (coerced per caller-supplied format; unknown fields rejected with PromptArgumentMismatch exit 28).

---

## External APIs

### First-Party APIs

- `commands::query::pipeline(args, deps) -> Result<QueryOutcome, TomeError>` — silent compute path reused by MCP `search_skills` tool (Phase 3 / US1.b); Phase 5: unchanged.
- `mcp::prompts::PromptRouter` — MCP `prompts/list` + `prompts/get` handlers (Phase 5 / US1); router built dynamically from enabled-and-user-invocable entries; `list_all` returns `Vec<Prompt>` with name, description (truncated per `DESCRIPTION_MAX_CHARS` = 300), arguments; `get` loads entry body, renders via substitution pipeline, returns as MCP PromptMessage array per contracts/mcp-prompts.md; Phase 5 US3: `get` accepts caller-supplied `arguments` field (Object or String), validates + coerces per declared schema, passes to substitution context.
- `plugin::identity::EntryKind` enum — Skill vs Command discriminator (Phase 5 F2); used in schema v3, prompt router filtering, collision tracking, error messages; Phase 5 US3: unchanged.
- `mcp::prompt_name::derive_name(catalog, plugin, entry_name, kind) -> String` — deterministic prompt naming per `<plugin>__<entry_name>` + collision-suffix algorithm (Phase 5 / US1); Phase 5 US3: unchanged.
- `mcp::prompt_collision::resolve_collisions(Vec<EntryIdentity>) -> CollisionRecord` — detects and resolves prompt name collisions at startup (Phase 5 / US1) per contracts/mcp-prompts.md §Collision handling; Phase 5 US3: unchanged.
- `substitution::render(body, context) -> Result<String, SubstitutionError>` — four-stage variable substitution pipeline (Phase 5 / US1–US3); Phase 5 / US2: Stages 1–2 (built-ins + env) wired via unified COMBINED_RE single-sweep design; Phase 5 / US3: Stages 3–4 wired via COMBINED_RE extension (6 named capture groups); entire 4-stage pipeline scanned once per render call (enforces NFR-007 no-rescan invariant).
- `substitution::SubstitutionContext` / `SubstitutionContextBuilder` — per-prompt context with workspace, plugin, entry identity, argument values (Phase 5 F3 skeleton, US1 builder wiring, US2 env + data-dir wiring, US3 argument value coercion + validation).
- `substitution::regex_sets::combined_regex()` — lazy-compiled unified regex for all 4 stages ({{TOME_*}} built-ins + {{$VAR}} env + $ARGUMENTS/$N/$NAME patterns + bare $ARGUMENTS) via `src/substitution/regex_sets.rs`; compiled once per process on first `render()` call; enforces no-rescan invariant (NFR-007) by emitting resolved values directly to output without re-scanning; Phase 5 US3: extended to 6 named capture groups (ENV_NAME, BUILTIN_NAME, DEFAULT for Stages 1–2; ARG_INDEX, POSITIONAL, NAMED for Stage 3).
- `substitution::arguments::shell_split(string) -> Vec<String>` — Stage 3 caller coercion per FR-043 (whitespace-separated tokens; single/double quotes preserve internal spaces; no escape sequences; tolerant of unterminated quotes).
- `substitution::arguments::coerce_arguments(supplied, declared) -> Result<ResolvedArguments, SubstitutionError>` — implements 6-row coercion table per contracts/substitution-engine.md (Single+declared → shell-split positional; Single+no-declared → whole-string single; Object+declared-full → named + positional; Object+declared-partial → missing names empty string; Object{args}+no-declared → catch-all to Single; Object{unknown} → PromptArgumentMismatch error).
- `substitution::arguments::apply_arguments_match(capture, resolved_args) -> String` — resolves one regex capture against coerced arguments ($[N] → positional, $<name> → named, bare $ARGUMENTS → space-joined positional).

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
- **Failure modes**: Network error → `TomeError::Io` (exit 7); checksum mismatch → `TomeError::ModelChecksumMismatch` (exit 32); corrupted registry → `TomeError::ModelCorrupt` (exit 31); missing model → `TomeError::ModelMissing` (exit 30); Phase 5 adds data-dir failures (26, 9), argument mismatches (28), missing entries (27), substitution failures (29), invalid frontmatter (25) per contracts/exit-codes-p5.md; Phase 5 US3: uses PromptArgumentMismatch (28) for argument validation failures in prompts/get.
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
| Filesystem | Persistent plugin/workspace data | User-managed (explicit cleanup); persistent across prompt executions; Phase 5 lazy-creation | `<home>/.tome/data/plugins/<catalog>_<plugin>/` and `<home>/.tome/data/workspaces/<workspace_name>/` — created on first `{{TOME_PLUGIN_DATA}}` / `{{TOME_WORKSPACE_DATA}}` reference via `src/substitution/data_dir.rs` (non-atomic, recoverable) |
| Filesystem | Orphaned staging dirs | Explicit cleanup via `tome doctor --fix`; 1-hour mtime gate (stale staging > 1h old assumed abandoned) | `<workspace_root>/.tome.tmp.*` staging dirs from failed atomic writes |
| In-memory | Compiled regex patterns for substitution | Per-process lifetime; lazily initialized on first `render()` call | `src/substitution/regex_sets.rs` — `COMBINED_RE` (all 4 stages via 6 named capture groups) via `OnceLock` slot; single-sweep design ensures all stages compiled together |

No TTL-based eviction. Explicit user commands for cleanup (principle VI). Phase 5 US2–US3: plugin/workspace data-dirs have no automatic eviction (user-managed, similar to summary cache). Regex patterns are process-singletons (reused across all prompts); Phase 5 US3: single COMBINED_RE pattern covers all 4 substitution stages (prevents cross-stage exfiltration by structural design).

---

## Monitoring & Observability

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| Structured logging (via `tracing`) | Diagnostic tracing to stderr (CLI) and JSON-lines to file (MCP server) | CLI: `RUST_LOG` or `TOME_LOG` environment variables; independent of `--json` stdout. MCP: JSON-lines to `<home>/.tome/mcp.log` per `contracts/log-format.md`; 10 MiB rotation cap; Phase 5 US2–US3: includes substitution warnings (failed data-dir creation, env var resolution failures, argument count mismatches, coercion failures), collision detection warnings at debug level |
| Exit codes | Scriptable error handling | 30+ enumerated codes; Phase 5 F1 adds 25–29 for invalid frontmatter (25), data-dir creation (26, 9), missing entries (27), argument mismatches (28), substitution failures (29); Phase 5 US3 uses code 28 (PromptArgumentMismatch) on argument validation failures in prompts/get |
| Status checks | Per-subsystem health via `tome status` | Phase 8 — models (all three), index, drift state; Phase 5: unchanged (status independent of prompts capability) |
| Doctor diagnostics | Subsystem health assessment + harness discovery + repair | Phase 3 / US4 onward; Phase 5: unchanged (doctor independent of prompts) |

---

## File Storage

| Service | Purpose | Configuration |
|---------|---------|-----------------|
| XDG-compliant filesystem | Configuration, catalogs, models, index, logs, workspace directories, plugin/workspace data, project markers | Global: `<home>/.tome/settings.toml`, `<home>/.tome/catalogs/<sha>/`, `<home>/.tome/models/`, `<home>/.tome/index.db`, `<home>/.tome/mcp.log`, `<home>/.tome/workspaces.txt` (opt-in), `<home>/.tome/data/plugins/` and `<home>/.tome/data/workspaces/` (Phase 5 new); Workspace: `<home>/.tome/workspaces/<name>/{settings.toml, RULES.md, index.db, catalogs/<sha>/}`; Project: `${PROJECT}/.tome/{config.toml, RULES.md}` (binding marker with `[bound_workspace]` name field); Phase 5 US2–US3: project marker updated via toml_edit on `tome workspace rename` |

---

## Email & Notifications

None in Phase 1–5.

---

## Agentic Coding Harness Integration (Phase 5 / US1–US3 extends with prompts + substitution)

Phase 5 / US1 introduces MCP `prompts` capability exposing enabled-and-user-invocable entries (skills + commands) as slash-prompts with variable substitution. Phase 5 / US2 wires substitution Stage 1 (built-ins) + Stage 2 (env passthrough) with single-sweep COMBINED_RE design. Phase 5 / US3 completes the substitution pipeline with Stage 3 (argument patterns) + Stage 4 (ARGUMENTS footer).

| Harness | Prompts Support | Changes |
|---------|-----------------|---------|
| Claude Code | Via MCP stdio transport | Phase 5 / US1: prompts/list + prompts/get handlers wired; prompt router built from `<workspace>` enabled + `user_invocable: true` entries; Phase 5 / US2: prompts/get invokes substitution render with Stages 1–2 via unified COMBINED_RE (enforces no-rescan invariant; closes exfiltration vector); Phase 5 / US3: prompts/get accepts caller-supplied `arguments` (Object or String), validates + coerces per declared schema, invokes full 4-stage render pipeline (built-ins + env + argument patterns + ARGUMENTS footer); argument mismatches → exit 28 (PromptArgumentMismatch) |
| Codex, Cursor, Gemini CLI, OpenCode | Via MCP stdio transport (if integrated) | Phase 5: same prompt exposure as Claude Code (harness-agnostic, all route through same MCP server); Phase 5 / US3: same argument coercion + validation as Claude Code |

**Prompt integration details (Phase 5 / US1–US3)**:
- **Prompt naming**: Deterministic `<plugin>__<entry_name>` per `src/mcp/prompt_name.rs`; collision-suffix counter on hash collision (`<plugin>__<entry_name>__N`) per contracts/mcp-prompts.md §Prompt naming algorithm.
- **Prompt listing**: `prompts/list` returns all enabled + user-invocable entries; description field truncated to 300 chars per `DESCRIPTION_MAX_CHARS` (FR-066); `listChanged: false` (static at startup per rmcp contract).
- **Prompt execution**: `prompts/get` accepts prompt name + optional `arguments` field (Object `{key: value}` or String for single arg); loads entry body, validates + coerces arguments per declared schema (Phase 5 US3), builds `SubstitutionContext` with workspace + plugin + entry identity + coerced argument values, renders via four-stage substitution pipeline, returns as PromptMessage array per contracts/mcp-prompts.md.
- **Variable substitution** (Phase 5 / US1–US3 progressive wiring): `{{TOME_*}}` built-ins (workspace name, plugin id, entry kind, data directories, wall-clock) + `{{$VAR}}` env passthrough via unified COMBINED_RE (US2) → `$ARGUMENTS` / `$N` / `$NAME` Claude Code argument syntax (US3); per contracts/substitution-engine.md; all 4 stages scanned once per render call (enforces NFR-007 no-rescan invariant).
- **Scope inference**: Prompt router built using resolved workspace's enabled entries; scope determined at MCP startup via cwd walk (or `--workspace` CLI override) per `src/workspace/resolution.rs`.
- **Data directory scaffolding** (Phase 5 / US2–US3): `{{TOME_PLUGIN_DATA}}` and `{{TOME_WORKSPACE_DATA}}` variables trigger lazy directory creation in `<home>/.tome/data/` on first reference per `src/substitution/data_dir.rs`; created non-atomically via `std::fs::create_dir_all` (recoverable via re-run).
- **Argument schema** (Phase 5 / US3): Skill/command frontmatter declares `arguments` field (list of names + optional descriptions); Phase 5 US3 parses + validates at prompt-execution time; coercion per 6-row table from contracts/substitution-engine.md (Single+declared shell-splits, Single+no-declared treats as single positional, Object+declared full/partial handles named + positional, Object{args} catches all to Single, Object{unknown} errors with PromptArgumentMismatch); per contracts/mcp-prompts.md §Arguments.
- **No-rescan invariant** (Phase 5 / US2–US3): Unified COMBINED_RE ensures all 4 stages scan once; resolved values never re-enter the scanner (per NFR-007 / FR-051; closes exfiltration vector where hostile plugin's `"version": "${TOME_ENV_GITHUB_TOKEN}"` could leak operator's env var; or caller's `arg = "${TOME_BUILTIN}"` could leak built-in values).
- **Clock injection** (Phase 5 / US2–US3): `{{TOME_CLOCK_*}}` variables hook into wall-clock via `src/substitution::current_clock()`; honours `SUBSTITUTION_CLOCK_OVERRIDE` slot for deterministic testing.
- **CLI-only execution**: Prompt bodies execute via MCP prompt invocation; substitution runs once over the body per execution. Unlike skills/commands which can be triggered from CLI directly, prompts are MCP-only (US1 ships prompts capability only; CLI slash-commands land in Phase 5 / US4 as first-class CLI entries discriminated by `EntryKind`).

---

## Settings Composition (Phase 4 extended, Phase 5 US2–US3 with workspace rename relocation + argument schema)

Composition resolver determines which prompts are available (enabled entries only) + which substitution context to use (workspace-scoped). Phase 5 US2 adds workspace rename relocation of bound project markers. Phase 5 US3 adds argument schema validation.

| Level | Location | Purpose | Precedence | Phase |
|-------|----------|---------|-----------|-------|
| **Project** | `${PROJECT}/.tome/config.toml` (strict) + `.tome/RULES.md` | Project-specific settings + context; Phase 5 US2: `[bound_workspace]` field name updated via toml_edit on `tome workspace rename`; Phase 5 US3: argument schema validation reads from skill/command frontmatter in enabled entries; Prompts stay workspace-scoped (no project discrimination) | Highest | F1+ |
| **Workspace** | `<home>/.tome/workspaces/<name>/settings.toml` (strict) | Workspace-local enablement, harness overrides, tool preferences, summary cache, entry filters; Phase 5: entry filters (which skills/commands are user-invocable) + `arguments` field validation (parsed leniently; unknown subfields forward-compatible) | Medium | F8+ |
| **Global** | `<home>/.tome/settings.toml` (strict) | User-wide defaults, catalog list, model preferences; Phase 5: unchanged | Lowest | F8+ |

**Phase 5 additions to composition**:
- **Entry filtering**: Workspace settings can declare `user_invocable: false` to opt out of prompt exposure (for CLI-only skills that don't fit slash-command pattern).
- **Argument schema** (Phase 5 US3): Both skills and commands declare `arguments` frontmatter field (list of `{name: "string", description?: "string"}` objects per contracts/frontmatter-p5.md); Phase 5 US3 validates at prompt-execution time per coercion table.
- **Workspace rename relocation** (Phase 5 / US2–US3): `tome workspace rename <old> <new>` updates `[bound_workspace] workspace = "<new>"` in all bound project markers via toml_edit (surgical field edit preserving comments + order); relocation runs atomically per project under `index.lock`.

---

## Schema Version 3 (Phase 5 / F2)

**Structural change**: Unified `entries` table with `kind` discriminator column (replaces `skills` + `commands` tables).

| Aspect | Details |
|--------|---------|
| **Migration path** | v2 → v3: forward-only migration under advisory lock; backfill `kind = 'skill'` for all existing rows; `commands` table remains empty (future Phase 5 US4 CLI commands populate it); reads from either table work (backward-compat query semantics) per contracts/schema-migration-p5.md |
| **Discriminator** | `EntryKind` enum — Skill vs Command — stored as lowercased string literal ("skill" / "command") per database convention |
| **Collision tracking** | In-memory only (built at router startup); no persistence in schema (collision records are computed from enabled + user-invocable entries) |
| **Prompt routing** | Phase 5 / US1: reads from unified `entries` table; filters by `kind = 'skill'` and `enabled = 1` and scanned frontmatter `user_invocable` field; Phase 5 / US3: argument schema validation uses scanned frontmatter `arguments` field; Phase 5 / US4: CLI commands land in same table with `kind = 'command'` discrimination |

---

## Substitution Engine (Phase 5 / F3 skeleton, US1 + US2 + US3 wiring)

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

## Project Binding Integration (Phase 4 US1, Phase 5 US2–US3 with relocation + argument schema)

Phase 5 / US1–US3 prompts are workspace-scoped (not project-scoped). Binding still used for:
- Workspace scope inference (`Paths::resolve()` cwd walk detects project marker).
- Project context for summary regeneration (Phase 4 US4 RULES.md body + frontmatter).
- Project-level harness MCP config (Phase 4 US1–US3).
- Phase 5 US2–US3: Bound workspace name relocation on `tome workspace rename` via toml_edit surgical edits (`[bound_workspace] workspace = "<new>"` per project marker).

Phase 5 US2–US3: Prompts don't access project context directly (only workspace + plugin + entry identity + coerced argument values). Workspace rename relocation updates all project markers atomically (one marker update per `index.lock` hold). Argument schema comes from skill/command frontmatter (workspace-scoped), not project context.

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

*This document maps external service dependencies and integration points in Tome at Phase 5 / US3 (4-stage substitution pipeline complete). Phase 5 / US3 introduces: argument substitution Stage 3 with 4 patterns integrated into COMBINED_RE (extending regex from 3 to 6 named capture groups); argument coercion per 6-row table (Single+declared shell-split, Single+no-declared, Object+declared-full/partial, Object{args} catch-all, Object{unknown} error); Stage 4 ARGUMENTS append-fallback footer (triggers when body has zero Stage-3 matches and caller supplied args); argument validation at prompt-execution time per coercion rules (PromptArgumentMismatch exit 28 on unknown/mismatched fields); structural enforcement of NFR-007 no-rescan across all 4 stages via single-sweep regex design. Zero new top-level dependencies. Binary size: **~27 MiB on macOS arm64**, well under the 50 MB cap. Data-dir scaffolding under `<home>/.tome/data/` is user-managed (non-atomic, recoverable). Prompt router built dynamically at startup from workspace-enabled + user-invocable entries; `listChanged: false` indicates static list (changes only on plugin enable/disable/reindex). Substitution pipeline (4 stages) runs once per prompt execution via single COMBINED_RE pass; all resolved values emitted directly to output without re-scanning (prevents cross-stage exfiltration per NFR-007 / FR-051). Phase 5 / US1 ships prompts capability (MCP exposure + prompt naming + collision tracking). Phase 5 / US2 ships substitution Stage 1 (built-ins) + Stage 2 (env passthrough). Phase 5 / US3 ships argument substitution Stage 3 + ARGUMENTS footer Stage 4, completing the 4-stage pipeline. Phase 5 / US4 ships CLI slash-commands as first-class entries alongside skills.*

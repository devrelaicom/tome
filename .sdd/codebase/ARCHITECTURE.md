# Architecture

> **Purpose**: Document system design, patterns, component relationships, and data flow.
> **Generated**: 2026-05-26
> **Last Updated**: 2026-05-27 (Phase 5 / US2 shipped; single-pass render pipeline, lazy data-dir creation, workspace rename relocation)

## Architecture Overview

Tome is a Rust CLI tool and MCP server that manages plugin ecosystems across coding harnesses (Claude Code, Cursor, Gemini CLI, Codex, OpenCode). It provides a centralized index for skill discovery and reranking, multi-workspace support with per-project bindings, harness composition management, workspace-scoped plugin enablement, comprehensive health diagnostics with auto-repair, command indexing and MCP prompts capability, and **Phase 5 / US2 NEW** variable substitution engine with four-stage rendering pipeline and per-plugin/workspace data directories.

The architecture is **monolithic with layered structure** split across two execution contexts:
- **CLI layer** — sync command dispatcher
- **MCP layer** — async stdio server (Phase 3+)

The central nervous system is a **single SQLite database** (`<home>/.tome/index.db`) that centralizes all state: plugin metadata, embeddings, workspace bindings, project bindings, enabled entries (skills/commands), and diagnostic metadata. Per-workspace composition settings and summaries live in separate TOML files (`<root>/workspaces/<name>/settings.toml`) and central RULES.md. Project markers (`<project>/.tome/config.toml`) are thin binding pointers, not databases.

Phase 5 / US1 shipped **commands as first-class database entries**, **MCP prompts capability**, and **substitution engine skeleton**. Phase 5 / US2 ships **single-pass rendering pipeline** (COMBINED_RE union regex), **lazy data-directory creation** within render, and **workspace rename integration** to relocate plugin-data directories.

## Architecture Pattern

| Pattern | Description |
|---------|-------------|
| Layered (capability-based) | Commands → Business Logic (Lifecycle, Embedding, Workspace, Harness, Summarise, Doctor, Substitution) → Data Access (Index, Catalog, Config) → Persistence (SQLite, Filesystem, Git) |
| Hexagonal (ports & adapters) | Trait boundaries for `Embedder`/`Reranker`/`Summariser`/`HarnessModule`/`ScopeProvider` allow swappable implementations (production vs stub for tests) |
| Trait-driven | Core abstractions decouple policy from mechanism; composition via struct fields rather than factory functions |
| Phase 5 / US1 — Unified entry dispatch | `EntryKind` enum (`Skill` \| `Command`) with kind-discriminated `skills` table rows; MCP prompts derived from user-invocable entries |
| Phase 5 / US2 — Single-pass substitution | Combined regex union (COMBINED_RE) replaces Stage 1+2 dual sweeps; built-ins, env vars, and arguments rendered in one loop |

## Core Components

### CLI Entry Point (`src/main.rs`)

- **Purpose**: Parse arguments, resolve workspace context, dispatch to subcommands
- **Location**: `src/main.rs`
- **Key flow**:
  1. Pre-parse `--version` flag (before clap) to include embedder/reranker/summariser identities
  2. Resolve `Paths` once from `$HOME/.tome/` (Phase 4 single root per constitution v1.3.0)
  3. Resolve workspace via `workspace::resolution::resolve()` (consults central DB)
  4. Route command dispatch; translate TomeError to exit codes
  5. Special-case MCP: skip stderr logging init + ctrlc handler (uses tokio signal)

### Substitution Engine (`src/substitution/`)

- **Purpose**: Phase 5 / US1–US2 — Render entry bodies through a single-pass variable pipeline
- **Location**: `src/substitution/{mod,context,builtins,env,arguments,data_dir,regex_sets}.rs`
- **Phase 5 / US2 wire-up (rendering pipeline)**:
  - **Combined regex** (COMBINED_RE in `regex_sets.rs`): Union of all placeholder patterns (`{{TOME_*}}`, `{{$*}}`, `$ARGUMENTS`/`$N`/`$name`)
  - **Single-pass loop** in `render()`: One regex scan over the entire body (replaces Phase 1+2 dual-sweep pattern)
  - **Per-match dispatch**: On each match, classify by pattern type and invoke the appropriate stage handler
  - **Lazy data-dir creation**: `ensure_plugin_data()` and `ensure_workspace_data()` called on first `{{TOME_*}}` reference within a single render pass
- **Module layout**:
  - `mod.rs` — `render(body, context) -> Result<String, SubstitutionError>` entry point (single-pass loop); `SubstitutionError` enum (6 variants: PluginDataDirCreationFailed, WorkspaceDataDirCreationFailed, InvalidArgumentFrontmatter, PromptArgumentMismatch, + 2 error types from specific stages)
  - `context.rs` — `SubstitutionContext` + `SubstitutionContextBuilder`; `ArgumentValues` enum (named/positional)
  - `builtins.rs` — Stage handler: `{{TOME_PLUGIN_DATA}}`, `{{TOME_WORKSPACE_DATA}}`, `{{TOME_WORKSPACE_NAME}}`, `{{TOME_CATALOG_NAME}}`, `{{TOME_PLUGIN_NAME}}`
  - `env.rs` — Stage handler: `{{$VAR}}` env passthrough (TOME_ENV_ prefix + generic env passthrough)
  - `arguments.rs` — Stage handler: Claude Code `$ARGUMENTS` / `$N` / `$name` (shell-style quoting rules)
  - `data_dir.rs` — Lazy directory creation on first reference (ensures plugin-data and workspace-data trees exist at render time)
  - `regex_sets.rs` — `OnceLock<Regex>` COMBINED_RE (compiled once at startup or on first use)
- **Test injection seam**: `SUBSTITUTION_OVERRIDE` thread_local (mirrors `MIGRATIONS_OVERRIDE` / `SUMMARISER_OVERRIDE` pattern per Phase 4 P6 lesson)

### Entry Kind Discriminator (`src/plugin/identity.rs::EntryKind`)

- **Purpose**: Phase 5 / US1 — Distinguish skills from commands in the unified `skills` table
- **Location**: `src/plugin/identity.rs`
- **Type**: `#[serde(rename_all = "lowercase")] pub enum EntryKind { Skill, Command }`
- **Usage**:
  - Written to `skills.kind` column (v3 schema migration backfills from directory source)
  - Serialized as `"skill"` / `"command"` in JSON (wire shape matches v3 migration SQL constants)
  - Read by `plugin::components::list_command_files` (enumerates `<plugin>/commands/*.md`)
  - Plumbed through `PendingSkill` struct in `index::skills` (F3 skeleton)
  - Propagated through MCP prompts registry to surface command entries as invocable prompts

### Plugin Components & Commands (`src/plugin/components.rs`)

- **Purpose**: Phase 5 / US1 — Walk plugin directories and enumerate commands
- **Location**: `src/plugin/components.rs`
- **Key additions**:
  - `list_command_files(plugin_dir) -> Vec<CommandFile>` — enumerate `<plugin>/commands/*.md` flat (non-recursive)
  - `CommandFile { path: PathBuf, name: String }` — one discovered command entry
  - Naming: `name` is filename stem (fallback when frontmatter omits); on-disk snapshot stays deterministic
- **Integration**: Called by `plugin::lifecycle::collect_pending_commands` to expand the enable pipeline to both skills and commands

### Paths & Data Directories (`src/paths.rs`)

- **Purpose**: Phase 4 consolidated root; Phase 5 / US1–US2 — Central data-directory accessors
- **Location**: `src/paths.rs`
- **New methods**:
  - `plugin_data_dir_for(catalog, plugin) -> PathBuf` — `<root>/plugin-data/<catalog>/<plugin>/` (process-wide)
  - `workspace_data_dir_for(workspace, catalog, plugin) -> PathBuf` — `<root>/workspaces/<name>/plugin-data/<catalog>/<plugin>/` (workspace-scoped)
- **Semantics**:
  - Process-wide vs workspace-scoped scratch space (mirrors substitution engine's dual reference)
  - Paths computed in F3 (Phase 5 skeleton); creation wired in US2 (lazy, within substitution render)
  - Matching `{{TOME_PLUGIN_DATA}}` / `{{TOME_WORKSPACE_DATA}}` built-in variables
  - **Phase 5 / US2 NEW**: `ensure_plugin_data()` / `ensure_workspace_data()` called by `substitution::render()` on first `{{TOME_*}}` reference (lazy, idempotent)

### Plugin Lifecycle & Command Indexing (`src/plugin/lifecycle.rs`)

- **Purpose**: Phase 5 / US1 — Extended enable/disable/reindex to handle commands alongside skills
- **Location**: `src/plugin/lifecycle.rs`
- **Changes**:
  - `enable_plugin` now calls `plugin::components::list_command_files` to enumerate commands
  - `collect_pending_commands(plugin_dir, catalog, plugin, plugin_version) -> Vec<PendingCommand>`
  - Both skills and commands are inserted via a unified `index::skills::enable_plugin_atomic` call
  - `PendingSkill` struct extended with `kind: EntryKind`, `when_to_use: Option<String>`, `searchable: bool`, `user_invocable: bool`

### Workspace Rename & Plugin-Data Relocation (`src/workspace/rename.rs`)

- **Purpose**: Phase 5 / US2 NEW — Relocate plugin-data directories during workspace rename
- **Location**: `src/workspace/rename.rs`
- **Algorithm additions**:
  1. Existing steps 1–5 (rename markers, update DB, rename workspace dir) — **unchanged**
  2. **NEW Step 6: Plugin-data relocation** (inside the workspace directory rename at step 5):
     - Before `std::fs::rename(<root>/workspaces/<old>/, ...)`, enumerate `<root>/workspaces/<old>/plugin-data/` for any existing plugin-specific data
     - Move each `<catalog>/<plugin>/` subdirectory to the new workspace location
     - Pattern: `std::fs::rename(<old>/plugin-data/<cat>/<plug>/, <new>/plugin-data/<cat>/<plug>/)`
     - Failures are logged; doctor `--fix` can recover via simple re-copy if needed
- **Integration**: Part of the single `fs::rename` operation that relocates the workspace directory tree (same atomic boundary)

### Index Schema / Entry Records (`src/index/skills.rs`)

- **Purpose**: Phase 5 / US1 — CRUD over the unified `skills` table with kind discriminator
- **Location**: `src/index/skills.rs`
- **Changes**:
  - `SkillRecord` struct gains `kind: EntryKind` field (reads from `skills.kind` column)
  - `SkillRecord` gains `when_to_use: Option<String>`, `searchable: bool`, `user_invocable: bool` (new v3 columns)
  - `PendingSkill` struct extended with matching fields
  - `resolve_entry_body_path(catalog, plugin, name, kind) -> PathBuf` — NEW helper (routes via kind)
    - Returns `<plugin>/skills/<name>/SKILL.md` or `<plugin>/commands/<name>.md` based on kind
  - Schema v2→v3 migration (in `index::migrations.rs`, F3 skeleton):
    - Adds `kind` column (backfilled via directory walk: `skill` if exists in `skills/`, else `command`)
    - Adds `when_to_use`, `searchable`, `user_invocable` columns (backfilled with defaults per contract)

### MCP Prompts Registry (`src/mcp/{prompts,prompt_name,prompt_collision}.rs`)

- **Purpose**: Phase 5 / US1 — Expose commands/skills as invocable MCP prompts
- **Location**: `src/mcp/{prompts,prompt_name,prompt_collision}.rs`
- **Components**:
  - **`prompts.rs`** — `PromptRegistry` and `PromptEntry` (one resolved entry ready for registration):
    - Driven by workspace's enabled + user-invocable entries at MCP startup
    - Hand-rolled `PromptRouter` via `rmcp::handler::server::router::prompt::PromptRoute::new_dyn` (NOT macro)
    - `PromptsCapability` declared in `Server::get_info` alongside tools
  - **`prompt_name.rs`** — Derivation algorithm: `<plugin>__<entry>` with sanitisation/truncation
    - `sanitise(input) -> String` — ASCII-lowercase, `[a-z0-9_-]` charset, collapse `_` runs, strip boundaries
    - `sanitise_trunc(input, max) -> String` — sanitise + truncate at char boundary
    - `derive_name(plugin, entry, name_override) -> String` — apply override or format `<plugin>__<entry>`
    - Caps: PLUGIN_PORTION_MAX=16, ENTRY_PORTION_MAX=32, OVERRIDE_MAX=48
  - **`prompt_collision.rs`** — Collision detection when multiple entries map to the same prompt name:
    - `CollisionRecord { prompt_name, entries: Vec<EntryIdentity> }`
    - `resolve_collisions(registry) -> Vec<CollisionRecord>` — identifies conflicts for user visibility
  - **`tool_description.rs`** (Phase 4 US4.b, preserved): Compose runtime tool description from scaffold + cached summary

### Prompt Arguments & Frontmatter (`src/plugin/frontmatter.rs`)

- **Purpose**: Phase 5 / US1 — Parse `arguments` frontmatter for entry invocation
- **Location**: `src/plugin/frontmatter.rs`
- **Changes**:
  - `SkillFrontmatter` extended with:
    - `arguments: Option<Vec<PromptArgument>>` — ordered list of expected arguments
    - `argument_hint: Option<String>` — hint for catch-all `args` argument description (Case B fallback)
    - `prompt_name: Option<String>` — override for derived `<plugin>__<entry>` format
    - `when_to_use: Option<String>` — guidance indexed for search
    - `searchable: Option<bool>` (default `true`) — controls `search_skills` visibility
    - `user_invocable: Option<bool>` (default `false` for skills; Tome explicit no-op) — controls `prompts/list` visibility

### Data Flow — Phase 5 / US1–US2

```
CLI: tome plugin enable <catalog>/<plugin>  (existing)
     ↓
Load workspace scope + central index
     ↓
plugin::components::list_command_files(plugin_dir) + collect_pending_commands(...)
     ↓
For each command + skill, read frontmatter (widened with when_to_use, searchable, user_invocable)
     ↓
index::skills::enable_plugin_atomic(pending_commands, pending_skills)
  ↓
  Insert/update skills table rows with kind=command/skill
  ↓
  Insert workspace_skills junction rows (existing)
     ↓
Release advisory lock
     ↓
regenerate_for_trigger(workspace_name, paths)  (Phase 4 — unchanged)
```

```
CLI: tome prompts invoke <prompt_name> [--arg-1 value1 --arg-2 value2 ...]
     ↓
Load workspace scope + central index
     ↓
Reverse-lookup prompt_name → (catalog, plugin, entry_name, kind)
     ↓
Resolve entry body path via resolve_entry_body_path(catalog, plugin, name, kind)
     ↓
Read entry body (SKILL.md or command .md)
     ↓
Parse entry frontmatter (including arguments schema)
     ↓
Validate supplied arguments against declared schema
     ↓
Build SubstitutionContext { entry, workspace, arguments }
     ↓
substitution::render(body, context) — SINGLE-PASS pipeline (Phase 5 / US2)
  ↓
  Compile COMBINED_RE (union of all stage patterns) [once at startup via OnceLock]
  ↓
  For each regex match in body (in order):
    ↓
    If pattern matches {{TOME_*}}: invoke builtins stage handler
      ↓
      On first {{TOME_*}}: call ensure_plugin_data() + ensure_workspace_data()
        ↓
        create_dir_all(plugin_data_dir_for(...)) if not exists [lazy, idempotent]
        ↓
        create_dir_all(workspace_data_dir_for(...)) if not exists [lazy, idempotent]
    ↓
    If pattern matches {{$*}}: invoke env stage handler
    ↓
    If pattern matches $ARGUMENTS/$N/$name: invoke arguments stage handler
    ↓
    Replace match with resolved value
     ↓
CLI: output rendered body (or pass to harness CLI)
```

```
CLI: tome workspace rename <old> <new>
     ↓
Existing steps 1-5 (per Phase 4 / US2 contract)
     ↓
Within workspace dir rename (step 5):
  ↓
  if <old>/plugin-data/ exists:
    ↓
    For each <catalog>/<plugin>/ subdir:
      ↓
      std::fs::rename(<old>/plugin-data/<cat>/<plug>/, <new>/plugin-data/<cat>/<plug>/)
      ↓
      Log success or non-critical error
```

## Layer Boundaries

| Layer | Responsibility | Can Access | Cannot Access |
|-------|----------------|------------|---------------|
| CLI | Argument parsing, mode dispatch, error formatting | Commands | Database, embedder directly |
| Commands | Command logic, outcome assembly, emit wrappers | Business logic (workspace, plugin, harness, settings, summarise, doctor, substitution) | Database directly (via deps) |
| Business logic | Policy (binding, lifecycle, sync, substitution, summarisation, diagnostics) | Index, catalog, plugin, settings, embedding, summarise, substitution | CLI, presentation |
| Data access | Queries, writes, transactions | Index, config, catalog on-disk | Commands, business logic |
| Persistence | SQLite, filesystem, git | Raw operations | Higher layers |

## Dependency Rules

- Higher layers can depend on lower layers, not vice versa
- Trait boundaries (`Embedder`, `Reranker`, `Summariser`, `HarnessModule`, `ScopeProvider`) decouple policy from mechanism
- `src/mcp/` is the only module allowed async (`tokio`); enforced by `tests/sync_boundary.rs`
- `src/substitution/` is sync-only; variable rendering is pure compute (lazy data-dir creation is the only I/O side effect)
- Workspace-specific code never reads/writes global index directly; uses scope-parameterized helpers
- Substitution engine allows test injection via `SUBSTITUTION_OVERRIDE` thread_local (mirrors `MIGRATIONS_OVERRIDE` / `SUMMARISER_OVERRIDE` pattern)
- Entry kind dispatch via `EntryKind` enum is exhaustive; matches are type-safe
- **Phase 5 / US2**: Single-pass rendering pipeline ensures each stage pattern is matched exactly once per body (no dual-sweep inefficiency)

---

*This document describes HOW the system is organized at Phase 5 / US2 (commands + prompts + single-pass substitution shipped). 954+ tests across 127+ suites.*
